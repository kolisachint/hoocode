#![cfg(unix)]
#![allow(clippy::disallowed_methods)] // test code: the probe builds its own runtime
//! A subagent child never gets hoocode's terminal: no controlling tty, so an
//! interactive shell it runs (`zsh -i`) cannot make itself the terminal's
//! foreground group. Before the fix it did, and the parent TUI, left in the
//! background, was stopped by SIGTTIN/SIGTTOU ("suspended (tty input)").
//!
//! The test re-runs itself as a probe process that owns a fresh pty as its
//! controlling terminal (a session leader in the foreground, like the TUI),
//! spawns the child through [`ProcessRunner`], and reports the terminal's
//! foreground group before and after.

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use hoocode_code_subagents::runner::{ProcessRunner, RunSpec, Runner};

const PROBE_ENV: &str = "HOOCODE_TEST_TTY_PROBE";
const TEST_NAME: &str = "tty::a_subagent_child_cannot_take_the_terminal_foreground";

/// The child under test: does it see a terminal, and an interactive zsh
/// (when installed) that would otherwise grab the foreground and keep it.
const SCRIPT: &str = "if : 2>/dev/null </dev/tty; then echo tty=yes; else echo tty=no; fi; \
     if command -v zsh >/dev/null 2>&1; then zsh -f -i -c 'echo zsh=ran'; fi";

#[test]
fn a_subagent_child_cannot_take_the_terminal_foreground() {
    if std::env::var_os(PROBE_ENV).is_some() {
        probe();
        return;
    }
    let report = run_probe_on_a_pty();
    assert!(report.contains("fg_before=own"), "probe setup: {report}");
    assert!(
        report.contains("tty=no"),
        "a subagent child must have no controlling terminal: {report}"
    );
    assert!(
        report.contains("fg_after=own"),
        "the child took the terminal's foreground group: {report}"
    );
}

/// Runs this test binary again as the probe, with a new pty as its
/// controlling terminal, and returns its report line.
fn run_probe_on_a_pty() -> String {
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: openpty(3) with valid out-pointers and no name/termios/winsize.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
    let mut command = Command::new(std::env::current_exe().expect("test binary"));
    command
        .args([TEST_NAME, "--exact", "--nocapture", "--test-threads=1"])
        .env(PROBE_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: setsid and ioctl are async-signal-safe; `slave` is an open fd.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 || libc::ioctl(slave, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command.output().expect("probe runs");
    // SAFETY: closing the two fds openpty gave us, after the probe exited.
    unsafe {
        libc::close(slave);
        libc::close(master);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "probe failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // libtest prints the test name on the same line, before the report.
    stdout
        .lines()
        .find_map(|line| line.find("PROBE ").map(|at| line[at..].to_string()))
        .unwrap_or_else(|| panic!("no probe report in: {stdout}"))
}

/// The probe side: spawns [`SCRIPT`] through the real subagent runner and
/// prints one `PROBE` line.
fn probe() {
    // SAFETY: open(2) with a constant path; the fd is closed below.
    let tty = unsafe { libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    assert!(tty >= 0, "the probe has a controlling terminal");
    // SAFETY: plain syscalls on our own process and an open tty fd.
    let (own, before) = unsafe { (libc::getpgrp(), libc::tcgetpgrp(tty)) };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (out, status) = runtime.block_on(async {
        let runner = ProcessRunner::new(PathBuf::from("/bin/sh"), vec!["-c".into()]);
        let mut env = HashMap::new();
        if let Some(path) = std::env::var_os("PATH") {
            env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
        }
        let mut handle = runner
            .spawn(RunSpec {
                task_id: "tty".into(),
                agent_type: "explore".into(),
                cwd: std::env::temp_dir(),
                argv: vec![SCRIPT.into()],
                env,
                max_turns: 1,
                ..Default::default()
            })
            .expect("spawn /bin/sh");
        let (mut stdout, _stderr) = handle.take_streams();
        let mut out = Vec::new();
        while let Some(chunk) = stdout.recv().await {
            out.extend(chunk);
        }
        (out, handle.wait().await)
    });
    // SAFETY: as above.
    let after = unsafe { libc::tcgetpgrp(tty) };
    unsafe { libc::close(tty) };
    let label = |pgrp: libc::pid_t| {
        if pgrp == own {
            "own".to_string()
        } else {
            pgrp.to_string()
        }
    };
    let text = String::from_utf8_lossy(&out).replace('\n', " ");
    println!(
        "PROBE fg_before={} fg_after={} status={status:?} output={text}",
        label(before),
        label(after),
    );
}
