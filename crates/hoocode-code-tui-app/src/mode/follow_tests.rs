//! The follow view (SA-12 row 5): a session another process holds opens read-only, shows
//! its history, re-renders as the file grows, refuses sends, and becomes a normal session
//! when the lock is free.
//!
//! The holder is a writer in this process (`SessionManager` takes the file's lock like the
//! detached child does). Ctrl-C records its signal instead of sending it, so no process
//! is signalled.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, Content, StopReason, UserMessage};
use hoocode_code_agent_session::ReplaceResult;
use hoocode_code_session::{create_session_id, SessionManager};
use hoocode_tui_render::Component;
use hoocode_tui_util::strip_vt_control_characters;

use super::input::Action;
use super::test_support::follow_mode;
use super::*;
use crate::status_line::LeftSlot;

const ENTER: &str = "\r";
const CTRL_C: &str = "\x03";

/// The pid of the last Ctrl-C signal the view sent (recorded, not sent).
static SIGNALLED: AtomicU32 = AtomicU32::new(0);

fn record_signal(pid: u32) -> std::io::Result<()> {
    SIGNALLED.store(pid, Ordering::SeqCst);
    Ok(())
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "follow-{name}-{}-{}",
        std::process::id(),
        create_session_id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        content: vec![Content::text(text)].into(),
        timestamp: hoocode_ai_types::now_ms(),
    })
}

fn reply(text: &str) -> AgentMessage {
    AgentMessage::Assistant(AssistantMessage {
        content: vec![Content::text(text)],
        stop_reason: StopReason::Stop,
        ..Default::default()
    })
}

/// A session file held by a writer (the detached child), with one exchange in it.
/// The writer flushes each entry, as a detached run does.
fn held_session(name: &str) -> (SessionManager, PathBuf, PathBuf) {
    let dir = temp_dir(name);
    let file = dir.join("session.jsonl");
    let mut child = SessionManager::create("/work", Some(dir.clone()));
    child.set_session_file(&file);
    child.set_flush_each_entry(true);
    child.append_message(user("question"));
    child.append_message(reply("first answer"));
    (child, file, dir)
}

/// The mode that follows `file` while `child` holds it.
fn follower(dir: &Path, file: &Path) -> Mode {
    follow_mode(SessionManager::open(file, Some(dir.to_path_buf()), None))
}

/// Makes the next check of the file due now.
fn check_now(mode: &mut Mode) {
    mode.follow.as_mut().expect("following").next_check = Instant::now();
}

/// The chat as plain text, 100 columns.
fn chat_text(mode: &Mode) -> String {
    let mut lines = Vec::new();
    for child in mode.transcript.chat.borrow().children().iter() {
        lines.extend(child.borrow_mut().render(100));
    }
    strip_vt_control_characters(&lines.join("\n"))
}

fn notice(mode: &Mode) -> Option<String> {
    mode.status_line_data().notice.map(|n| n.message)
}

fn prompt_text(mode: &Mode) -> String {
    mode.editor.borrow().editor.get_text()
}

/// Sends one key the way the screen does: the prompt editor, then the actions it queued.
fn press(mode: &mut Mode, data: &str) {
    if !mode.leader_takes_key_at(data, Instant::now()) {
        mode.editor.borrow_mut().render(80);
        mode.editor.borrow_mut().handle_input(data);
    }
    let actions: Vec<Action> = mode.actions.borrow_mut().drain(..).collect();
    for action in actions {
        mode.handle_action(action);
    }
}

fn type_text(mode: &mut Mode, text: &str) {
    for c in text.chars() {
        press(mode, &c.to_string());
    }
}

#[test]
fn a_held_session_opens_read_only_and_a_send_is_refused() {
    let (child, file, dir) = held_session("read-only");
    let pid = std::process::id();
    let mut mode = follower(&dir, &file);

    assert!(mode.is_following());
    assert!(
        chat_text(&mode).contains("first answer"),
        "the history shows"
    );
    assert!(matches!(
        mode.status_line_data().left_slot(),
        LeftSlot::Following(p) if p == pid
    ));
    let before = mode.session.messages().len();

    type_text(&mut mode, "hello");
    press(&mut mode, ENTER);

    assert_eq!(prompt_text(&mode), "hello", "the text stays in the prompt");
    assert_eq!(
        notice(&mode),
        Some(format!(
            "Read-only: process {pid} is running this session. ctrl+c stops it."
        ))
    );
    assert!(!mode.prompt_pending, "no run starts");
    assert!(!mode.session.is_streaming());
    assert_eq!(mode.session.messages().len(), before);
    drop(child);
}

#[test]
fn growing_the_file_re_renders_the_new_entry_on_the_next_check() {
    let (mut child, file, dir) = held_session("grows");
    let mut mode = follower(&dir, &file);
    assert!(!chat_text(&mode).contains("second answer"));

    child.append_message(reply("second answer"));
    check_now(&mut mode);
    mode.tick_follow();

    assert!(chat_text(&mode).contains("second answer"), "re-rendered");
    assert!(chat_text(&mode).contains("first answer"), "history kept");
    drop(child);
}

#[test]
fn a_released_lock_turns_the_view_into_a_normal_session() {
    let (child, file, dir) = held_session("released");
    let pid = std::process::id();
    let mut mode = follower(&dir, &file);

    // The holder ends: its lock is free.
    drop(child);
    check_now(&mut mode);
    mode.tick_follow();
    // Tests have no runtime to switch with, so the switch does not start and the view
    // checks again on the next pass.
    assert!(mode.is_following());

    // The switch completes as the runtime reports it.
    mode.finish_switch(file.clone(), false, Ok(ReplaceResult::default()));

    assert!(!mode.is_following(), "the view is a normal session now");
    assert_eq!(
        notice(&mode),
        Some(format!("Process {pid} ended; the session is yours."))
    );
}

#[test]
fn ctrl_c_signals_the_holder_and_a_second_press_quits() {
    let (child, file, dir) = held_session("ctrl-c");
    let pid = std::process::id();
    let mut mode = follower(&dir, &file);
    mode.follow.as_mut().expect("following").signal = record_signal;

    press(&mut mode, CTRL_C);

    assert_eq!(
        SIGNALLED.load(Ordering::SeqCst),
        pid,
        "SIGINT to the holder"
    );
    assert_eq!(
        notice(&mode),
        Some(format!("Sent SIGINT to process {pid}."))
    );
    assert!(!mode.user_quit, "the first press does not quit");

    press(&mut mode, CTRL_C);
    assert!(mode.user_quit, "a second press in the window quits");
    drop(child);
}

#[test]
fn write_commands_are_refused_and_quit_still_works() {
    let (child, file, dir) = held_session("commands");
    let mut mode = follower(&dir, &file);

    type_text(&mut mode, "/name renamed");
    press(&mut mode, ENTER);
    assert_eq!(prompt_text(&mode), "/name renamed", "kept in the prompt");
    assert!(
        notice(&mode).is_some_and(|n| n.starts_with("Read-only: ")),
        "refused with the read-only notice"
    );
    assert_eq!(mode.session.session_manager().session_name(), None);

    mode.editor.borrow_mut().editor.set_text("");
    type_text(&mut mode, "/quit");
    press(&mut mode, ENTER);
    assert!(mode.user_quit, "/quit still works");
    drop(child);
}
