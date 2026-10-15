#![cfg(unix)]
#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! The process runner's pipes: bytes arrive in order, and a child's pipes do
//! not cost a thread each (concurrency.md §1, the thread ceiling). The pipes
//! are read by tasks on the `hoocode-io` runtime.

use std::collections::HashMap;
use std::path::PathBuf;

use hoocode_code_subagents::runner::{ProcessRunner, RunSpec, Runner};

/// Threads named like the parent's pipe readers. Linux names are cut to 15
/// bytes, so the prefix is what survives.
#[cfg(target_os = "linux")]
fn pipe_reader_threads() -> usize {
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return 0;
    };
    tasks
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .filter(|name| name.trim().starts_with("hoocode-subagen"))
        .count()
}

#[cfg(not(target_os = "linux"))]
fn pipe_reader_threads() -> usize {
    0
}

fn spec(script: &str) -> RunSpec {
    RunSpec {
        task_id: "pipes".into(),
        agent_type: "explore".into(),
        cwd: std::env::temp_dir(),
        argv: vec![script.into()],
        env: HashMap::new(),
        prompt: String::new(),
        max_turns: 1,
        ..Default::default()
    }
}

fn runner() -> ProcessRunner {
    ProcessRunner::new(PathBuf::from("/bin/sh"), vec!["-c".into()])
}

async fn collect(mut stream: tokio::sync::mpsc::Receiver<Vec<u8>>) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.recv().await {
        bytes.extend(chunk);
    }
    bytes
}

#[tokio::test]
async fn both_pipes_arrive_in_order_without_a_pipe_thread() {
    let _serial = crate::SERIAL.lock().await;
    let before = pipe_reader_threads();
    let mut handle = runner()
        .spawn(spec(
            "printf 'one\\n'; printf 'err\\n' >&2; sleep 1; printf 'two\\n'",
        ))
        .expect("spawn /bin/sh");
    assert_eq!(
        pipe_reader_threads(),
        before,
        "reading a child's pipes must not start a thread"
    );
    let (stdout, stderr) = handle.take_streams();
    let (out, err) = tokio::join!(collect(stdout), collect(stderr));
    assert_eq!(String::from_utf8(out).unwrap(), "one\ntwo\n");
    assert_eq!(String::from_utf8(err).unwrap(), "err\n");
    assert_eq!(handle.wait().await.unwrap(), Some(0));
}

#[tokio::test]
async fn a_large_output_arrives_whole() {
    let _serial = crate::SERIAL.lock().await;
    // Far more than the channel holds (64 chunks of 16 KiB): the reader must
    // wait for the consumer, and no byte may be lost.
    let mut handle = runner()
        .spawn(spec("head -c 4194304 /dev/zero"))
        .expect("spawn /bin/sh");
    let (stdout, _stderr) = handle.take_streams();
    let out = collect(stdout).await;
    assert_eq!(out.len(), 4 * 1024 * 1024);
    assert!(out.iter().all(|&b| b == 0));
    assert_eq!(handle.wait().await.unwrap(), Some(0));
}
