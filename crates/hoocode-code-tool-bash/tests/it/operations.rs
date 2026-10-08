#![allow(clippy::disallowed_methods)] // test code: threads that stand in for a peer, a slow tool or a second caller
//! `LocalBashOperations` on the async pipe path: output parity under load, the
//! idle tick, and kill on abort at the operations level.
#![cfg(unix)]

use std::time::{Duration, Instant};

use hoocode_ai_types::AbortSignal;
use hoocode_code_tool_bash::{BashExecOptions, BashOperations, LocalBashOperations};

/// Runs `command` with the local shell and returns (output bytes, exit result, idle ticks).
fn exec(
    command: &str,
    signal: Option<AbortSignal>,
    timeout: Option<f64>,
) -> (Vec<u8>, Result<Option<i32>, String>, usize) {
    let mut out = Vec::new();
    let mut ticks = 0usize;
    let mut on_idle = || ticks += 1;
    let result = LocalBashOperations::default()
        .exec(
            command,
            &std::env::temp_dir(),
            BashExecOptions {
                on_data: &mut |data| out.extend_from_slice(data),
                on_idle: Some(&mut on_idle),
                signal,
                timeout,
                env: None,
            },
        )
        .map_err(|e| e.to_string());
    (out, result, ticks)
}

#[test]
fn large_interleaved_stdout_and_stderr_keep_each_stream_in_order() {
    let command = "i=1; while [ $i -le 3000 ]; do echo out$i; echo err$i >&2; i=$((i+1)); done";
    let (out, result, _) = exec(command, None, None);
    assert_eq!(result, Ok(Some(0)));
    let text = String::from_utf8(out).expect("utf8");
    let outs: Vec<&str> = text.lines().filter(|l| l.starts_with("out")).collect();
    let errs: Vec<&str> = text.lines().filter(|l| l.starts_with("err")).collect();
    assert_eq!(outs.len(), 3000);
    assert_eq!(errs.len(), 3000);
    for (i, line) in outs.iter().enumerate() {
        assert_eq!(*line, format!("out{}", i + 1));
    }
    for (i, line) in errs.iter().enumerate() {
        assert_eq!(*line, format!("err{}", i + 1));
    }
}

#[test]
fn multi_megabyte_output_from_both_pipes_arrives_whole() {
    // 2 MiB to stdout and 1 MiB to stderr: far more than the caller's queue holds.
    let command =
        "head -c 2097152 /dev/zero | tr '\\0' o; head -c 1048576 /dev/zero | tr '\\0' e >&2";
    let (out, result, _) = exec(command, None, None);
    assert_eq!(result, Ok(Some(0)));
    assert_eq!(out.len(), 2_097_152 + 1_048_576);
    assert_eq!(out.iter().filter(|b| **b == b'o').count(), 2_097_152);
    assert_eq!(out.iter().filter(|b| **b == b'e').count(), 1_048_576);
}

#[test]
fn abort_kills_the_process_group_and_returns_promptly() {
    let signal = AbortSignal::new();
    let aborter = signal.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        aborter.abort();
    });
    let started = Instant::now();
    let (_, result, _) = exec("echo started; sleep 30 & sleep 30", Some(signal), None);
    assert_eq!(result, Err("aborted".to_owned()));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "abort took {:?}",
        started.elapsed()
    );
}

#[test]
fn timeout_kills_the_command_and_reports_it() {
    let started = Instant::now();
    let (_, result, _) = exec("sleep 30", None, Some(0.2));
    assert_eq!(result, Err("timeout:0.2".to_owned()));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn idle_tick_keeps_running_after_the_pipes_close() {
    // Both pipes close at once, the process runs on; ticks must not stop.
    let (out, result, ticks) = exec("exec >&- 2>&-; sleep 0.3", None, None);
    assert_eq!(result, Ok(Some(0)));
    assert!(out.is_empty());
    assert!(ticks >= 5, "only {ticks} idle ticks after the pipes closed");
}

#[test]
fn idle_tick_runs_while_the_command_is_silent() {
    let (out, result, ticks) = exec("sleep 0.3; echo late", None, None);
    assert_eq!(result, Ok(Some(0)));
    assert_eq!(String::from_utf8_lossy(&out).trim(), "late");
    assert!(ticks >= 5, "only {ticks} idle ticks in 300 ms");
}
