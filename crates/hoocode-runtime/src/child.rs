//! Priority of child processes, set at spawn (`docs/design/concurrency.md`
//! section 2). Subagent children run at nice +5; `Shell` children run at
//! `performance.bashNice`.

use std::process::Command;

/// Niceness of a subagent child process.
pub const SUBAGENT_CHILD_NICE: u64 = 5;

/// Highest niceness Linux and macOS allow (`nice` 19).
const MAX_CHILD_NICE: u64 = 19;

/// Windows creation flag for `BELOW_NORMAL_PRIORITY_CLASS`.
pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

/// Lowers the priority of a child process that `command` will spawn.
///
/// `nice` 0 changes nothing. On Unix a value above 0 sets the child's niceness
/// to `nice` in `pre_exec`, before exec, so the child never runs at the normal
/// priority first. The niceness is never lowered below the parent's, so a child
/// of a child keeps the higher value. On Windows any value above 0 gives
/// `BELOW_NORMAL_PRIORITY_CLASS`. A failure to lower the priority is ignored:
/// the child runs at the inherited priority.
///
/// A caller that also uses a `process_wrap` `JobObject` must not rely on the
/// Windows flag: that wrapper overwrites creation flags set on the `Command`.
pub fn lower_child_priority(command: &mut Command, nice: u64) {
    if nice == 0 {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let nice = nice.min(MAX_CHILD_NICE) as libc::c_int;
        // SAFETY: the closure only calls setpriority, which is async-signal-safe,
        // between fork and exec. It touches no memory of the parent.
        unsafe {
            command.pre_exec(move || {
                // getpriority and setpriority are async-signal-safe.
                if libc::getpriority(libc::PRIO_PROCESS, 0) < nice {
                    libc::setpriority(libc::PRIO_PROCESS, 0, nice);
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(BELOW_NORMAL_PRIORITY_CLASS);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = command;
    }
}

/// Runs the child that `command` will spawn in a new session (`setsid` in
/// `pre_exec`), so it has no controlling terminal.
///
/// The child leads a new process group whose id is its pid, as with
/// `process_group(0)`, so `kill(-pid, …)` still reaches its whole tree. Unlike
/// a plain process group, the child and its descendants cannot open
/// `/dev/tty` or move the terminal's foreground group: an interactive shell
/// (`zsh -i`) run by a tool would otherwise make itself the foreground group
/// and leave hoocode in the background, where the TUI's next tty read or
/// termios write stops it with SIGTTIN or SIGTTOU.
///
/// Do not combine with `process_group(0)`: a session leader cannot change its
/// group, so that `setpgid` would fail the spawn. A no-op off Unix.
pub fn spawn_in_new_session(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // SAFETY: the closure only calls setsid, which is async-signal-safe,
        // between fork and exec. It touches no memory of the parent.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

#[cfg(test)]
mod unit {
    use super::SUBAGENT_CHILD_NICE;

    #[test]
    fn subagent_children_are_five_nice_levels_down() {
        assert_eq!(SUBAGENT_CHILD_NICE, 5);
    }
}
