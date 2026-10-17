//! A UI-thread panic writes the terminal restore to stdout: the alternate screen
//! is left (`?1049l`), the cursor shown and the modes turned off, before the
//! process unwinds. Run in a child process, because the hook and the restore
//! write to the real stdout (fd 1) of the process that panics.

use std::process::{Command, Stdio};

use hoocode_tui_terminal::interrupt::restore_terminal;
use hoocode_tui_terminal::panic_hook::{install, Effects};

/// Set by the parent test for the child run; unset, the child test does nothing.
const CHILD_ENV: &str = "HOOCODE_TEST_CRASH_RESTORE_CHILD";

/// The child: installs the real restore on the test thread (the UI thread for
/// this process), then panics there.
#[test]
fn crash_restore_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let effects = Effects {
        restore: Box::new(restore_terminal),
        flush: Box::new(|| {}),
        log: Box::new(|_: &str| {}),
    };
    assert!(install(std::thread::current().id(), effects));
    panic!("crash-restore-marker");
}

#[cfg(unix)]
#[test]
fn ui_thread_panic_leaves_the_alternate_screen_and_modes() {
    let exe = std::env::current_exe().expect("test binary path");
    let output = Command::new(exe)
        .args([
            "crash_restore::crash_restore_child",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("crash-restore-marker"),
        "the child panicked: {stderr}"
    );
    // The restore sequence, in full, as the first thing written after the test
    // harness' own lines. `?1049l` must be there: that is the normal screen.
    let sequence = "\x1b[?1049l\x1b[?25h\x1b[?2004l\x1b[<u\x1b[>4;0m\x1b[?1006l\x1b[?1000l";
    assert!(
        stdout.contains(sequence),
        "restore sequence missing from child stdout: {stdout:?}"
    );
    assert_eq!(
        stdout.matches("\x1b[?1049l").count(),
        1,
        "the restore runs once: {stdout:?}"
    );
}

/// Set by the parent for the SIGTERM child run.
#[cfg(unix)]
const TERM_CHILD_ENV: &str = "HOOCODE_TEST_TERM_RESTORE_CHILD";

/// The child for the SIGTERM test: installs the terminate restore, reports ready,
/// then waits for the signal.
#[cfg(unix)]
#[test]
fn term_restore_child() {
    if std::env::var_os(TERM_CHILD_ENV).is_none() {
        return;
    }
    hoocode_tui_terminal::interrupt::install_terminate_restore();
    println!("term-child-ready");
    std::thread::sleep(std::time::Duration::from_secs(30));
    panic!("the child was not signalled");
}

/// SIGTERM on a running TUI restores the terminal (`?1049l`) and exits 128+15,
/// instead of dying on the alternate screen.
#[cfg(unix)]
#[test]
fn sigterm_restores_the_terminal_and_exits_143() {
    use std::io::{BufRead, BufReader, Read};

    let exe = std::env::current_exe().expect("test binary path");
    let mut child = Command::new(exe)
        .args([
            "crash_restore::term_restore_child",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(TERM_CHILD_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn child");
    let mut out = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(&mut out);
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(
            reader.read_line(&mut line).expect("read child"),
            0,
            "child exited early"
        );
        if line.contains("term-child-ready") {
            break;
        }
    }
    // SAFETY: plain kill(2) on the child we spawned.
    let rc = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(rc, 0);
    let status = child.wait().expect("wait child");
    let mut rest = String::new();
    let _ = out.read_to_string(&mut rest);
    assert_eq!(status.code(), Some(143), "exit status: {status:?}");
    assert!(
        rest.contains("\x1b[?1049l"),
        "alternate screen not left after SIGTERM: {rest:?}"
    );
}
