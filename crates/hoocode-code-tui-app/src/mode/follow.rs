//! The follow view (SA-12 row 5): an interactive run opens a session another process
//! holds. The session is read-only and no agent run starts. The transcript is the file's
//! history, re-rendered through the resume path when the file grows. Enter refuses a
//! send, Ctrl-C signals the holder, and when the lock is free the view becomes a normal
//! session through the switch path.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hoocode_code_agent_session::AgentSession;
use hoocode_code_session::lock::lock_holder;
use hoocode_code_session::SessionManager;
use hoocode_code_tui_keybindings::key_text;
use hoocode_runtime::send_sigint;

use super::commands::BuiltinCommand;
use super::input::Action;
use super::*;

/// How often the follow view checks the session file: the once-a-second cadence.
pub(super) const FOLLOW_CHECK: Duration = Duration::from_secs(1);

/// A session another process holds, shown read-only.
pub(super) struct Follow {
    pub(super) path: PathBuf,
    /// The holder's process id; 0 when the lock file does not name it.
    pub(super) pid: u32,
    /// The file length when the transcript was last rendered.
    pub(super) len: u64,
    /// When the file is next checked.
    pub(super) next_check: Instant,
    /// The lock was free and the switch to the session is in progress.
    pub(super) leaving: bool,
    /// Sends the Ctrl-C signal. Tests replace it, so no real process is signalled.
    pub(super) signal: fn(u32) -> io::Result<()>,
}

impl Follow {
    /// The follow view for `session`, when another process holds its file.
    pub(super) fn for_session(session: &AgentSession) -> Option<Follow> {
        let manager = session.session_manager();
        let pid = manager.lock_held_by()?;
        let path = manager.session_file()?.to_path_buf();
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Some(Follow {
            path,
            pid,
            len,
            next_check: Instant::now() + FOLLOW_CHECK,
            leaving: false,
            signal: send_sigint,
        })
    }

    /// `process 123`, or `another process` when the lock file does not name it.
    fn holder(&self) -> String {
        holder_name(self.pid)
    }
}

fn holder_name(pid: u32) -> String {
    if pid == 0 {
        "another process".to_string()
    } else {
        format!("process {pid}")
    }
}

/// The notice when the view takes over a session whose process ended.
pub(super) fn follow_ended_text(pid: u32) -> String {
    if pid == 0 {
        "The process ended; the session is yours.".to_string()
    } else {
        format!("Process {pid} ended; the session is yours.")
    }
}

/// The slash commands the follow view runs. Everything else is refused: a command that
/// writes the session (or the runtime it runs on) would change a file another process owns.
fn follow_refuses_command(command: BuiltinCommand) -> bool {
    matches!(
        command,
        BuiltinCommand::Name
            | BuiltinCommand::Compact
            | BuiltinCommand::Model
            | BuiltinCommand::Tree
            | BuiltinCommand::Fork
            | BuiltinCommand::Clone
            | BuiltinCommand::Cd
            | BuiltinCommand::Reload
            | BuiltinCommand::Import
            | BuiltinCommand::Loop
            | BuiltinCommand::Subagent
            | BuiltinCommand::SubagentCancel
            | BuiltinCommand::SubagentRetry
            | BuiltinCommand::Plugin
            | BuiltinCommand::Mcp
    )
}

/// Whether a submitted line may run in the follow view. Only the slash commands that
/// do not write pass; a prompt, a `!` command and an extension command do not.
pub(super) fn follow_allows_submit(text: &str) -> bool {
    if !text.starts_with('/') {
        return false;
    }
    let (name, has_args) = match text.split_once(' ') {
        Some((name, _)) => (name, true),
        None => (text, false),
    };
    match BuiltinCommand::lookup(name) {
        Some(command) => (!has_args || command.with_args()) && !follow_refuses_command(command),
        None => false,
    }
}

/// The prompt actions the follow view refuses: they write the session or queue a run.
pub(super) fn follow_refuses_action(action: &Action) -> bool {
    matches!(
        action,
        Action::ModelForward
            | Action::ModelBackward
            | Action::ModelSelect
            | Action::ThinkingForward
            | Action::ThinkingBackward
            | Action::SessionTree
            | Action::ForkOpen
            | Action::FollowUp
            | Action::Dequeue
    )
}

impl Mode {
    /// Whether the mode follows a session another process holds.
    pub(super) fn is_following(&self) -> bool {
        self.follow.is_some()
    }

    /// The notice for a write the follow view refuses. The text stays in the prompt.
    pub(super) fn refuse_read_only(&mut self) {
        let holder = self
            .follow
            .as_ref()
            .map(Follow::holder)
            .unwrap_or_else(|| holder_name(0));
        self.show_warning(&format!(
            "Read-only: {holder} is running this session. {} stops it.",
            key_text("app.clear")
        ));
    }

    /// Ctrl-C in the follow view: SIGINT to the holder. The caller has already
    /// handled a second press within the double-press window (it quits).
    pub(super) fn follow_interrupt(&mut self, now: Instant) {
        let Some(follow) = self.follow.as_ref() else {
            return;
        };
        let (pid, signal) = (follow.pid, follow.signal);
        if pid == 0 {
            self.show_warning("The lock does not name its process, so it cannot be stopped.");
            return;
        }
        match signal(pid) {
            Ok(()) => {
                self.last_sigint = Some(now);
                self.show_status(&format!("Sent SIGINT to process {pid}."));
            }
            Err(error) => {
                self.show_error(&format!("Could not send SIGINT to process {pid}: {error}"));
            }
        }
    }

    /// Once a second: re-renders the session when the file grew, and takes the session
    /// over when the lock is free. Nothing runs while a session operation is in flight.
    pub(super) fn tick_follow(&mut self) {
        let Some(follow) = self.follow.as_mut() else {
            return;
        };
        let now = Instant::now();
        if now < follow.next_check {
            return;
        }
        follow.next_check = now + FOLLOW_CHECK;
        if follow.leaving || self.session_op.is_some() {
            return;
        }
        let (path, known_len) = (follow.path.clone(), follow.len);
        if lock_holder(&path).is_none() {
            self.follow_leave();
            return;
        }
        let Ok(len) = std::fs::metadata(&path).map(|m| m.len()) else {
            return;
        };
        if len != known_len {
            self.follow_refresh(&path, len);
        }
    }

    /// Re-reads the file and re-renders it through the resume path
    /// (`render_current_session_state`). The holder keeps writing the file, so
    /// the new manager reads it without the lock and never writes.
    fn follow_refresh(&mut self, path: &Path, len: u64) {
        let session_dir = self.session.session_manager().session_dir().to_path_buf();
        let manager = SessionManager::open_read_only(path, Some(session_dir), None);
        let messages = manager.build_context().messages;
        *self.session.session_manager() = manager;
        self.session.agent().set_messages(messages);
        if let Some(follow) = self.follow.as_mut() {
            follow.len = len;
        }
        self.render_current_session_state();
        self.dirty.set(true);
    }

    /// The lock is free: the view asks the switch path to open the session as a normal
    /// one. `finish_switch` completes it, or the view waits again when the switch is refused.
    fn follow_leave(&mut self) {
        let Some(follow) = self.follow.as_mut() else {
            return;
        };
        follow.leaving = true;
        let path = follow.path.clone();
        self.handle_resume_session(path, None);
        if self.session_op.is_none() {
            // The switch did not start (no runtime to switch with): try again next check.
            if let Some(follow) = self.follow.as_mut() {
                follow.leaving = false;
            }
        }
    }
}
