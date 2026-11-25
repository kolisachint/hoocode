//! The command that starts a detached hoocode child session (SA-12).
//!
//! Shared by `SessionSpawn` (a fresh child with a brief), and the TUI's "approve and
//! detach" choice (the same session, continued). The child runs unattended with the
//! subagent limits, has no stdin, and is not killed when this process exits.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::depth::SUBAGENT_DEPTH_ENV;
use crate::lifeguard::SUBAGENT_DEADLINE_MS;
use crate::DEFAULT_SUBAGENT_MAX_TURNS;

/// Set in the environment of every detached run (and read by the run itself).
pub const DETACHED_ENV: &str = "HOOCODE_DETACHED";

/// The task-identity variables the subagent machinery sets on its children. A
/// detached session is a root run, so it must not inherit them.
pub const AGENT_IDENTITY_ENV: [&str; 4] = [
    "HOOCODE_AGENT_ID",
    "HOOCODE_AGENT_TOKEN",
    "HOOCODE_AGENT_PARENT",
    "HOOCODE_AGENT_DEPTH",
];

/// Prefix of the tier C helper socket variables, which a child must not inherit.
const HELPER_SOCKET_PREFIX: &str = "HOOCODE_HELPER_SOCKET_";

/// The command that runs a new child: `-p <prompt> --session <path>` with the subagent
/// turn and deadline limits, and the parent's model when it has one.
/// The child gets `HOOCODE_DETACHED=1`, no subagent depth and no task identity,
/// and no stdin. Its stdio is set by [`spawn_detached`].
pub fn child_command(
    exe: &Path,
    session: &Path,
    prompt: &str,
    cwd: &Path,
    model: Option<(&str, &str)>,
) -> Command {
    let mut command = Command::new(exe);
    command.arg("-p").arg(prompt).arg("--session").arg(session);
    configure_child(command, cwd, model)
}

/// The command that continues an existing detached session: as [`child_command`] without
/// a prompt, so the run continues from the session's trailing tool results.
pub fn child_command_continue(
    exe: &Path,
    session: &Path,
    cwd: &Path,
    model: Option<(&str, &str)>,
) -> Command {
    let mut command = Command::new(exe);
    command.arg("-p").arg("--session").arg(session);
    configure_child(command, cwd, model)
}

/// The limits, model, working directory and environment every detached child gets.
fn configure_child(mut command: Command, cwd: &Path, model: Option<(&str, &str)>) -> Command {
    command
        .arg("--max-turns")
        .arg(DEFAULT_SUBAGENT_MAX_TURNS.to_string())
        .arg("--deadline-ms")
        .arg(SUBAGENT_DEADLINE_MS.to_string());
    if let Some((provider, id)) = model {
        command
            .arg("--provider")
            .arg(provider)
            .arg("--model")
            .arg(id);
    }
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .env(DETACHED_ENV, "1");
    for prefix in hoocode_code_paths::ENV_PREFIXES {
        command.env_remove(format!("{prefix}{SUBAGENT_DEPTH_ENV}"));
    }
    for name in AGENT_IDENTITY_ENV {
        command.env_remove(name);
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with(HELPER_SOCKET_PREFIX) {
            command.env_remove(key);
        }
    }
    command
}

/// The log file of a detached session: `<session file>.log`.
pub fn log_path_for(session: &Path) -> PathBuf {
    let mut name = session.as_os_str().to_owned();
    name.push(".log");
    PathBuf::from(name)
}

/// Starts `command` detached (its own session, `SIGHUP` ignored) with stdout and
/// stderr appended to `log`. The child is not killed when this process exits.
pub fn spawn_detached(command: &mut Command, log: &Path) -> std::io::Result<Child> {
    let file = OpenOptions::new().create(true).append(true).open(log)?;
    let copy = file.try_clone()?;
    command.stdout(Stdio::from(file)).stderr(Stdio::from(copy));
    hoocode_runtime::spawn_detached_session(command);
    command.spawn()
}
