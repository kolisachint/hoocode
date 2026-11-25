//! The session lock: one writer per session file (SA-12), and per-entry flush.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{Content, Message, UserMessage};
use hoocode_code_session::{
    lock_holder, lock_path_for, SessionLock, SessionLockError, SessionManager,
    SESSION_FLUSH_DEADLINE,
};

fn user(text: &str) -> AgentMessage {
    AgentMessage::from_message(Message::User(UserMessage {
        content: vec![Content::text(text)].into(),
        timestamp: 0,
    }))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "session-lock-{tag}-{}-{}",
        std::process::id(),
        hoocode_code_session::create_session_id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn a_second_acquire_on_the_same_path_reports_our_pid() {
    let dir = temp_dir("second");
    let session = dir.join("a.jsonl");
    let first = SessionLock::acquire(&session).expect("first lock");
    assert_eq!(
        SessionLock::acquire(&session).err(),
        Some(SessionLockError::Held {
            pid: Some(std::process::id())
        })
    );
    drop(first);
    assert!(SessionLock::acquire(&session).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_lock_file_is_the_session_name_with_lock_appended() {
    let session = Path::new("/x/y/a.jsonl");
    assert_eq!(lock_path_for(session), PathBuf::from("/x/y/a.jsonl.lock"));
}

#[test]
fn lock_holder_follows_the_lock() {
    let dir = temp_dir("holder");
    let session = dir.join("b.jsonl");
    assert_eq!(lock_holder(&session), None, "no lock file yet");

    let held = SessionLock::acquire(&session).expect("lock");
    assert_eq!(lock_holder(&session), Some(std::process::id()));
    drop(held);
    assert_eq!(lock_holder(&session), None, "released lock is free");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_manager_on_a_locked_file_is_read_only() {
    let dir = temp_dir("readonly");
    let mut owner = SessionManager::create("/w", Some(dir.clone()));
    owner.set_flush_each_entry(true);
    owner.append_message(user("first"));
    owner.flush(SESSION_FLUSH_DEADLINE).expect("flushed");
    let file = owner.session_file().expect("file").to_path_buf();
    assert_eq!(owner.lock_held_by(), None);

    let mut other = SessionManager::open(&file, Some(dir.clone()), None);
    assert_eq!(other.lock_held_by(), Some(std::process::id()));
    let before = std::fs::read(&file).expect("bytes");
    other.append_message(user("second"));
    other.flush(SESSION_FLUSH_DEADLINE).expect("flushed");
    assert_eq!(std::fs::read(&file).expect("bytes"), before);

    drop(owner);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A child process holds the lock; SIGKILL releases it, and the next acquire
/// succeeds. Skipped when python3 is not installed.
#[test]
fn a_killed_holder_leaves_no_stale_lock() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipped: python3 not found");
        return;
    }
    let dir = temp_dir("killed");
    let session = dir.join("c.jsonl");
    let lock_file = lock_path_for(&session);
    let script = "import fcntl,sys,time\n\
        f=open(sys.argv[1],'a+')\n\
        fcntl.flock(f,fcntl.LOCK_EX)\n\
        f.seek(0);f.truncate();f.write(str(12345));f.flush()\n\
        print('ready',flush=True)\n\
        time.sleep(60)\n";
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(&lock_file)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn python3");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("read ready");
    assert_eq!(line.trim(), "ready");

    assert_eq!(
        SessionLock::acquire(&session).err(),
        Some(SessionLockError::Held { pid: Some(12345) })
    );
    assert_eq!(lock_holder(&session), Some(12345));

    child.kill().expect("kill");
    child.wait().expect("reap");
    assert!(SessionLock::acquire(&session).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn per_entry_flush_puts_the_header_and_first_entry_on_disk() {
    let dir = temp_dir("per-entry");
    let mut manager = SessionManager::create("/w", Some(dir.clone()));
    manager.set_flush_each_entry(true);
    manager.append_message(user("hello"));
    let file = manager.session_file().expect("file").to_path_buf();

    // No flush call: the append itself waited for the OS.
    let text = std::fs::read_to_string(&file).expect("file exists");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "header and the user entry: {text}");
    assert!(lines[0].contains("\"type\":\"session\""));
    assert!(lines[1].contains("hello"));
    drop(manager);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_session_never_written_leaves_no_lock_file() {
    let dir = temp_dir("unwritten");
    let unwritten = dir.join("a.jsonl");
    drop(SessionLock::acquire(&unwritten).expect("lock"));
    assert!(!lock_path_for(&unwritten).exists());

    let written = dir.join("b.jsonl");
    std::fs::write(&written, "{}\n").expect("session file");
    drop(SessionLock::acquire(&written).expect("lock"));
    assert!(lock_path_for(&written).exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_unlockable_session_still_writes_and_reports_the_error() {
    let dir = temp_dir("unlockable");
    let file = dir.join("a.jsonl");
    // A directory where the lock file goes: the lock cannot be taken (an I/O error).
    std::fs::create_dir_all(lock_path_for(&file)).expect("blocking directory");

    let mut manager = SessionManager::open(file.clone(), Some(dir.clone()), None);
    assert!(manager.lock_error().is_some(), "the lock error is reported");
    assert_eq!(manager.lock_held_by(), None, "no other process holds it");
    manager.set_flush_each_entry(true);
    manager.append_message(user("written without the lock"));

    let text = std::fs::read_to_string(&file).expect("the session was written");
    assert!(text.contains("written without the lock"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_locked_session_reports_no_lock_error() {
    let dir = temp_dir("lock-ok");
    let mut manager = SessionManager::create("/w", Some(dir.clone()));
    manager.append_message(user("hello"));
    assert_eq!(manager.lock_error(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_read_only_takes_no_lock_and_never_writes() {
    let dir = temp_dir("reader");
    let mut owner = SessionManager::create("/w", Some(dir.clone()));
    owner.set_flush_each_entry(true);
    owner.append_message(user("first"));
    let file = owner.session_file().expect("file").to_path_buf();
    drop(owner);

    // Another process holds the lock: a reader still loads the session.
    let held = SessionLock::acquire(&file).expect("lock");
    let mut reader = SessionManager::open_read_only(file.clone(), Some(dir.clone()), None);
    assert_eq!(
        reader.lock_held_by(),
        None,
        "a reader does not report a holder"
    );
    assert_eq!(reader.build_context().messages.len(), 1);
    let before = std::fs::read(&file).expect("bytes");
    reader.append_message(user("second"));
    reader.flush(SESSION_FLUSH_DEADLINE).expect("flushed");
    assert_eq!(std::fs::read(&file).expect("bytes"), before);

    // The reader took no lock: once the holder lets go, a writer gets it.
    drop(held);
    let writer = SessionManager::open(&file, Some(dir.clone()), None);
    assert_eq!(writer.lock_held_by(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_read_only_of_a_missing_file_writes_nothing() {
    let dir = temp_dir("reader-missing");
    let file = dir.join("none.jsonl");
    let mut reader = SessionManager::open_read_only(file.clone(), Some(dir.clone()), None);
    reader.append_message(user("never"));
    reader.flush(SESSION_FLUSH_DEADLINE).expect("flushed");
    assert!(!file.exists());
    assert!(!lock_path_for(&file).exists());
    let _ = std::fs::remove_dir_all(&dir);
}
