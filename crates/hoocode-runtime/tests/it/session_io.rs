//! Tests for the `hoocode-session-io` writer: ordering, barriers, backpressure,
//! the crash simulation and the timeout path.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hoocode_runtime::{block_on_entry, FileWriter, FlushError, SESSION_IO_THREAD};

const DEADLINE: Duration = Duration::from_secs(5);

/// A fresh directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "hoocode-session-io-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn appends_land_in_order_per_file() {
    let dir = TempDir::new("order");
    let a = dir.file("a.jsonl");
    let b = dir.file("b.jsonl");
    fs::write(&a, "").unwrap();
    fs::write(&b, "").unwrap();
    let writer = FileWriter::start("hoocode-test-order", 64, 1 << 20).unwrap();
    for i in 0..200 {
        writer.append_line(&a, format!("a{i}"));
        if i % 3 == 0 {
            writer.append_line(&b, format!("b{i}"));
        }
    }
    writer.flush_blocking(DEADLINE).expect("flushed");
    let want_a: Vec<String> = (0..200).map(|i| format!("a{i}")).collect();
    let want_b: Vec<String> = (0..200).step_by(3).map(|i| format!("b{i}")).collect();
    assert_eq!(lines(&a), want_a);
    assert_eq!(lines(&b), want_b);
}

#[test]
fn replace_then_append_keeps_the_order() {
    let dir = TempDir::new("replace");
    let path = dir.file("s.jsonl");
    fs::write(&path, "old\n").unwrap();
    let writer = FileWriter::start("hoocode-test-replace", 64, 1 << 20).unwrap();
    writer.append_line(&path, "first".into());
    writer.replace(&path, "header\n".into());
    writer.append_line(&path, "after".into());
    writer.flush_blocking(DEADLINE).expect("flushed");
    assert_eq!(lines(&path), vec!["header", "after"]);
}

#[test]
fn replace_creates_missing_parent_directories() {
    let dir = TempDir::new("mkdir");
    let path = dir.file("nested/deeper/s.jsonl");
    let writer = FileWriter::start("hoocode-test-mkdir", 64, 1 << 20).unwrap();
    writer.replace(&path, "x\n".into());
    writer.flush_blocking(DEADLINE).expect("flushed");
    assert_eq!(lines(&path), vec!["x"]);
}

#[test]
fn barrier_returns_only_after_earlier_entries_are_on_disk() {
    let dir = TempDir::new("barrier");
    let path = dir.file("s.jsonl");
    fs::write(&path, "").unwrap();
    let writer = FileWriter::start("hoocode-test-barrier", 4096, 1 << 26).unwrap();
    for i in 0..3 {
        writer.append_line(&path, format!("line{i}"));
    }
    writer.barrier().wait_blocking(DEADLINE).expect("flushed");
    assert_eq!(lines(&path), vec!["line0", "line1", "line2"]);
    // A barrier on an idle writer completes at once.
    writer
        .flush_blocking(Duration::from_millis(50))
        .expect("idle");
}

#[test]
fn async_wait_on_a_barrier_completes() {
    let dir = TempDir::new("async");
    let path = dir.file("s.jsonl");
    fs::write(&path, "").unwrap();
    let writer = FileWriter::start("hoocode-test-async", 64, 1 << 20).unwrap();
    writer.append_line(&path, "x".into());
    let ticket = writer.barrier();
    block_on_entry(ticket.wait(DEADLINE)).expect("flushed");
    assert_eq!(lines(&path), vec!["x"]);
}

#[test]
fn a_failed_write_is_reported_by_the_next_barrier() {
    let dir = TempDir::new("error");
    // Append never creates a file, so this write fails.
    let missing = dir.file("missing.jsonl");
    let writer = FileWriter::start("hoocode-test-error", 64, 1 << 20).unwrap();
    writer.append_line(&missing, "lost".into());
    match writer.flush_blocking(DEADLINE) {
        Err(FlushError::Io(_)) => {}
        other => panic!("expected an io error, got {other:?}"),
    }
    // The error is reported once.
    writer.flush_blocking(DEADLINE).expect("error was consumed");
    assert!(!missing.exists());
}

#[test]
fn backpressure_waits_and_drops_nothing() {
    let dir = TempDir::new("backpressure");
    let path = dir.file("s.jsonl");
    fs::write(&path, "").unwrap();
    // Four entries at most; all 300 lines must arrive, in order.
    let writer = FileWriter::start("hoocode-test-backpressure", 4, 1 << 20).unwrap();
    for i in 0..300 {
        writer.append_line(&path, format!("{i}"));
    }
    writer.flush_blocking(DEADLINE).expect("flushed");
    let want: Vec<String> = (0..300).map(|i| i.to_string()).collect();
    assert_eq!(lines(&path), want);
}

#[test]
fn byte_cap_applies_backpressure_too() {
    let dir = TempDir::new("bytes");
    let path = dir.file("s.jsonl");
    fs::write(&path, "").unwrap();
    // 64 bytes at most; each line is 33 bytes with its newline.
    let writer = FileWriter::start("hoocode-test-bytes", 4096, 64).unwrap();
    let big = "x".repeat(32);
    for _ in 0..50 {
        writer.append_line(&path, big.clone());
    }
    // A single entry larger than the cap is accepted when the queue is empty.
    writer.append_line(&path, "y".repeat(1000));
    writer.flush_blocking(DEADLINE).expect("flushed");
    let got = lines(&path);
    assert_eq!(got.len(), 51);
    assert!(got[..50].iter().all(|l| *l == big));
    assert_eq!(got[50].len(), 1000);
}

#[test]
fn dropping_the_writer_drains_the_queue() {
    let dir = TempDir::new("drop");
    let path = dir.file("s.jsonl");
    fs::write(&path, "").unwrap();
    let writer = FileWriter::start("hoocode-test-drop", 4096, 1 << 26).unwrap();
    for i in 0..100 {
        writer.append_line(&path, format!("{i}"));
    }
    drop(writer);
    let start = Instant::now();
    while lines(&path).len() < 100 && start.elapsed() < DEADLINE {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(lines(&path).len(), 100);
}

/// Simulates kill -9 mid-turn: the first 10 entries were flushed, the next 10
/// were queued and the writer stopped before it wrote them. The file must hold
/// whole lines, in order, up to the flushed entries.
#[test]
fn crash_loses_only_the_unflushed_entries() {
    for round in 0..20 {
        let dir = TempDir::new("crash");
        let path = dir.file("s.jsonl");
        fs::write(&path, "").unwrap();
        let writer =
            FileWriter::start(&format!("hoocode-test-crash-{round}"), 4096, 1 << 26).unwrap();
        for i in 0..10 {
            writer.append_line(&path, format!(r#"{{"n":{i}}}"#));
        }
        writer.flush_blocking(DEADLINE).expect("flushed");
        for i in 10..20 {
            writer.append_line(&path, format!(r#"{{"n":{i}}}"#));
        }
        writer.abandon();

        let raw = fs::read_to_string(&path).unwrap();
        assert!(
            raw.is_empty() || raw.ends_with('\n'),
            "round {round}: torn last line"
        );
        let got: Vec<&str> = raw.lines().collect();
        assert!(
            got.len() >= 10 && got.len() <= 20,
            "round {round}: {} lines",
            got.len()
        );
        for (i, line) in got.iter().enumerate() {
            assert_eq!(
                *line,
                format!(r#"{{"n":{i}}}"#),
                "round {round}: line {i} is torn or out of order"
            );
        }
    }
}

#[test]
fn a_flushed_file_has_no_partial_lines() {
    let dir = TempDir::new("partial");
    let path = dir.file("s.jsonl");
    fs::write(&path, "").unwrap();
    let writer = FileWriter::start("hoocode-test-partial", 4096, 1 << 26).unwrap();
    for i in 0..500 {
        writer.append_line(
            &path,
            format!(r#"{{"n":{i},"pad":"{}"}}"#, "p".repeat(i % 97)),
        );
    }
    writer.flush_blocking(DEADLINE).expect("flushed");
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.ends_with("}\n"));
    assert_eq!(raw.lines().count(), 500);
}

/// A crash can leave a torn line with no newline. The next append must start a
/// new line instead of being glued onto the torn one.
#[test]
fn append_after_a_torn_line_starts_a_new_line() {
    let dir = TempDir::new("torn");
    let path = dir.file("s.jsonl");
    fs::write(&path, "header\n{\"torn").unwrap();
    let writer = FileWriter::start("hoocode-test-torn", 64, 1 << 20).unwrap();
    writer.append_line(&path, "next".into());
    writer.flush_blocking(DEADLINE).expect("flushed");
    assert_eq!(lines(&path), vec!["header", "{\"torn", "next"]);
}

#[cfg(unix)]
#[test]
fn barrier_times_out_while_the_writer_is_stuck() {
    let dir = TempDir::new("timeout");
    // The writer opens the FIFO read-write (that never blocks), so it stalls on
    // a write once the pipe buffer (64 KiB) is full: a stuck disk. One line
    // larger than the buffer stalls it until the test reads.
    let fifo = dir.file("stuck.jsonl");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(status.success());
    const LINE: usize = 1 << 20;
    let writer = FileWriter::start("hoocode-test-timeout", 64, 4 << 20).unwrap();
    writer.append_line(&fifo, "x".repeat(LINE));
    assert_eq!(
        writer.flush_blocking(Duration::from_millis(100)),
        Err(FlushError::Timeout)
    );
    // Unblock the writer by reading what it wrote; the barrier then completes.
    let mut reader = fs::File::open(&fifo).expect("reader opens");
    let mut sink = vec![0u8; LINE + 1];
    std::io::Read::read_exact(&mut reader, &mut sink).expect("drained");
    writer
        .flush_blocking(DEADLINE)
        .expect("flushed after unblock");
    assert_eq!(sink[LINE], b'\n');
}

#[test]
fn the_session_writer_has_its_name() {
    assert_eq!(SESSION_IO_THREAD, "hoocode-session-io");
    let dir = TempDir::new("name");
    let path = dir.file("s.jsonl");
    fs::write(&path, "").unwrap();
    hoocode_runtime::session_io().append_line(&path, "ok".into());
    hoocode_runtime::session_io()
        .flush_blocking(DEADLINE)
        .expect("flushed");
    assert_eq!(lines(&path), vec!["ok"]);
}
