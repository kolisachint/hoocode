//! The command that starts a detached hoocode child session (SA-12).
//!
//! Shared by `SessionSpawn` (a fresh child with a brief), and the TUI's "approve and
//! detach" choice (the same session, continued). The child runs unattended with the
//! subagent limits, has no stdin, and is not killed when this process exits.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};

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

/// A detached session this process (or the session it is resumed in) spawned:
/// its file, its goal line and when it was started (unix ms; 0 when it is read
/// back from a session file, which does not record the start).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnedSession {
    pub session: PathBuf,
    pub goal: String,
    pub started_at: u64,
}

/// The sessions spawned by the session the process has open. Process-wide, so the
/// tool (on the agent's thread) and the task panel (on the UI thread) share it.
static SPAWNED: Mutex<Vec<SpawnedSession>> = Mutex::new(Vec::new());

fn spawned() -> MutexGuard<'static, Vec<SpawnedSession>> {
    SPAWNED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Records a session `SessionSpawn` has just started.
pub fn record_spawn(session: PathBuf, goal: String) {
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    spawned().push(SpawnedSession {
        session,
        goal,
        started_at,
    });
}

/// The spawned sessions, in the order they were recorded.
pub fn spawned_sessions() -> Vec<SpawnedSession> {
    spawned().clone()
}

/// Replaces the registry with `sessions` (the sessions a newly opened session spawned).
pub fn replace_spawned_sessions(sessions: Vec<SpawnedSession>) {
    *spawned() = sessions;
}

/// The session a `SessionSpawn` result names, read from its text: the goal from
/// `Started a detached session: <goal>` and the file from the line
/// `hoocode --session <path>`. `None` when the text has no such file line.
pub fn parse_spawn_result(text: &str) -> Option<(PathBuf, String)> {
    let session = text
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("hoocode --session "))
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let goal = text
        .lines()
        .find_map(|line| line.strip_prefix("Started a detached session: "))
        .unwrap_or_default()
        .trim()
        .to_string();
    Some((PathBuf::from(session), goal))
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    const RESULT: &str = "Started a detached session: Fix the parser\n\nOpen it with:\n\n```\nhoocode --session /work/.hoocode/sessions/child.jsonl\n```\n\nStatus: running. It works on its own and returns nothing here. Stop working on this task.";

    #[test]
    fn the_result_text_names_the_file_and_the_goal() {
        let (path, goal) = parse_spawn_result(RESULT).unwrap();
        assert_eq!(path, PathBuf::from("/work/.hoocode/sessions/child.jsonl"));
        assert_eq!(goal, "Fix the parser");
    }

    #[test]
    fn a_text_without_the_file_line_names_no_session() {
        assert_eq!(parse_spawn_result("Started a detached session: x"), None);
        assert_eq!(parse_spawn_result("hoocode --session   \n"), None);
        assert_eq!(parse_spawn_result(""), None);
    }

    #[test]
    fn a_result_without_a_goal_line_keeps_the_file() {
        let (path, goal) = parse_spawn_result("hoocode --session /tmp/a.jsonl").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/a.jsonl"));
        assert_eq!(goal, "");
    }

    #[test]
    fn the_registry_records_replaces_and_lists_in_order() {
        replace_spawned_sessions(Vec::new());
        record_spawn(PathBuf::from("/a.jsonl"), "first".into());
        record_spawn(PathBuf::from("/b.jsonl"), "second".into());
        let listed = spawned_sessions();
        assert_eq!(
            listed.iter().map(|s| s.goal.as_str()).collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(listed[0].started_at > 0);
        replace_spawned_sessions(vec![SpawnedSession {
            session: PathBuf::from("/c.jsonl"),
            goal: "restored".into(),
            started_at: 0,
        }]);
        let restored = spawned_sessions();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].session, PathBuf::from("/c.jsonl"));
        replace_spawned_sessions(Vec::new());
    }
}
