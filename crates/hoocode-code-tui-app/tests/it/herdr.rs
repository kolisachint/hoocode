//! Herdr reporting through a fake `herdr` binary that logs its argv.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hoocode_code_tui_app::herdr::{run_cli, AgentState, CliTransport, Reporter};

/// A script that appends its argv (one line per call) to `log`.
fn fake_herdr(dir: &Path, log: &Path, body: &str) -> PathBuf {
    let bin = dir.join("herdr");
    std::fs::write(
        &bin,
        format!("#!/bin/sh\n{body}\necho \"$*\" >> '{}'\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn read_lines(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn wait_for_lines(log: &Path, n: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let lines = read_lines(log);
        if lines.len() >= n || Instant::now() > deadline {
            return lines;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn reports_states_then_releases_through_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("argv.log");
    let bin = fake_herdr(dir.path(), &log, "");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let file = sessions.join("s_abc.jsonl");
    std::fs::write(&file, "{}\n").unwrap();
    let launch = sessions.clone();
    let mut seq = 0;
    let mut reporter = Reporter::new(
        "w1:p1".into(),
        // Long timeouts: a loaded CI machine must not cut the release short.
        Box::new(CliTransport::start_with(
            bin,
            Duration::from_secs(20),
            Duration::from_secs(20),
        )),
        Box::new(move || {
            seq += 10;
            seq
        }),
        Box::new(|path: &Path| path.is_file()),
        Some(launch),
    );
    reporter.update(AgentState::Idle, None, "abc", Some(&file));
    wait_for_lines(&log, 1);
    reporter.update(AgentState::Working, None, "abc", Some(&file));
    wait_for_lines(&log, 2);
    reporter.release();
    let lines = wait_for_lines(&log, 3);
    let path = file.display();
    let base = "pane report-agent w1:p1 --source hoocode --agent hoocode --state";
    assert_eq!(
        lines,
        vec![
            format!(
                "{base} idle --seq 10 --agent-session-id abc --agent-session-path {path} \
                 -- hoocode --session abc"
            ),
            format!(
                "{base} working --seq 20 --agent-session-id abc --agent-session-path {path} \
                 -- hoocode --session abc"
            ),
            "pane release-agent w1:p1 --source hoocode --agent hoocode --seq 30".to_string(),
        ]
    );
}

#[test]
fn a_hung_cli_is_killed_at_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("argv.log");
    let bin = fake_herdr(dir.path(), &log, "exec sleep 30");
    let started = Instant::now();
    run_cli(&bin, &["x".to_string()], Duration::from_millis(200), None);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(read_lines(&log).is_empty());
}

#[test]
fn a_missing_cli_is_ignored() {
    run_cli(
        Path::new("/nonexistent/herdr"),
        &["x".to_string()],
        Duration::from_millis(200),
        None,
    );
}

#[test]
fn a_cancelled_cli_is_killed_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("argv.log");
    let bin = fake_herdr(dir.path(), &log, "exec sleep 30");
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let setter = hoocode_runtime::spawn_thread("herdr-test-cancel", move || {
        std::thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    run_cli(
        &bin,
        &["x".to_string()],
        Duration::from_secs(20),
        Some(&cancel),
    );
    let _ = setter.join();
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// The release does not run beside a report in flight: it cuts the report
/// short, and it is the only call that logs.
#[test]
fn the_release_comes_after_a_report_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("argv.log");
    let body = "case \"$*\" in *report-agent*) exec sleep 30;; esac";
    let bin = fake_herdr(dir.path(), &log, body);
    let mut seq = 0;
    let mut reporter = Reporter::new(
        "w1:p1".into(),
        Box::new(CliTransport::start_with(
            bin,
            Duration::from_secs(20),
            Duration::from_secs(20),
        )),
        Box::new(move || {
            seq += 1;
            seq
        }),
        Box::new(|_: &Path| false),
        None,
    );
    reporter.update(AgentState::Working, None, "abc", None);
    // Let the worker start the report that hangs.
    std::thread::sleep(Duration::from_millis(200));
    reporter.update(AgentState::Idle, None, "abc", None);
    let started = Instant::now();
    reporter.finish(true);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(
        read_lines(&log),
        vec!["pane release-agent w1:p1 --source hoocode --agent hoocode --seq 3".to_string()]
    );
}
