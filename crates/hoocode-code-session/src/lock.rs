//! One writer per session file (SA-12).
//!
//! A session file is guarded by a sidecar lock file: the session path with
//! `.lock` appended to its full name (`a.jsonl` → `a.jsonl.lock`). The lock is
//! an advisory `flock` held by the open file descriptor, so the OS drops it
//! when the process dies and a crash leaves no stale lock. The lock file is
//! never deleted: removing it would race with a new opener.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Tries after the first, and the wait between them: about 100 ms in all.
const ACQUIRE_RETRIES: u32 = 5;
const ACQUIRE_RETRY_GAP: std::time::Duration = std::time::Duration::from_millis(20);

/// The sidecar lock file for `session_path`.
pub fn lock_path_for(session_path: &Path) -> PathBuf {
    let mut name = session_path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// Why a session lock could not be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLockError {
    /// Another open file description holds the lock. `pid` is the holder's
    /// process id, or `None` when the lock file does not say.
    Held { pid: Option<u32> },
    /// The lock file could not be created, opened or locked.
    Io(String),
}

impl std::fmt::Display for SessionLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionLockError::Held { pid: Some(pid) } => {
                write!(f, "session is in use by process {pid}")
            }
            SessionLockError::Held { pid: None } => {
                write!(f, "session is in use by another process")
            }
            SessionLockError::Io(message) => write!(f, "session lock failed: {message}"),
        }
    }
}

impl std::error::Error for SessionLockError {}

/// The message shown when a session is in use by another process.
pub fn session_in_use_message(pid: u32, session_path: &Path) -> String {
    if pid == 0 {
        format!(
            "Session is in use by another process: {}",
            session_path.display()
        )
    } else {
        format!(
            "Session is in use by process {pid}: {}",
            session_path.display()
        )
    }
}

/// An exclusive lock on one session file, held while this value lives. The
/// open file is what holds the `flock`; it is never read after acquiring.
#[derive(Debug)]
pub struct SessionLock {
    _file: File,
    session_path: PathBuf,
}

impl Drop for SessionLock {
    /// A session that was never written leaves no lock file behind. The lock
    /// is still held here, so no other process is writing that session.
    fn drop(&mut self) {
        if !self.session_path.exists() {
            let _ = std::fs::remove_file(lock_path_for(&self.session_path));
        }
    }
}

impl SessionLock {
    /// Takes the lock for `session_path`, writing this process id into the
    /// lock file. Fails with [`SessionLockError::Held`] when another process
    /// (or another open file in this process) holds it.
    pub fn acquire(session_path: &Path) -> Result<SessionLock, SessionLockError> {
        let path = lock_path_for(session_path);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(io_error)?;
            }
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io_error)?;
        // `lock_holder` probes by taking the lock for a moment (the parent's status poll does
        // so every second), so a held lock is tried a few times before it counts as held.
        let mut tries = 0;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if tries < ACQUIRE_RETRIES => {
                    tries += 1;
                    std::thread::sleep(ACQUIRE_RETRY_GAP);
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(SessionLockError::Held {
                        pid: read_pid(&mut file),
                    });
                }
                Err(TryLockError::Error(e)) => return Err(io_error(e)),
            }
        }
        file.set_len(0).map_err(io_error)?;
        file.seek(SeekFrom::Start(0)).map_err(io_error)?;
        write!(file, "{}", std::process::id()).map_err(io_error)?;
        file.flush().map_err(io_error)?;
        Ok(SessionLock {
            _file: file,
            session_path: session_path.to_path_buf(),
        })
    }
}

/// The pid of the process holding the lock for `session_path`: `Some(pid)`
/// (0 when the lock file is unreadable) while another open file holds it,
/// `None` when the lock is free or there is no lock file.
pub fn lock_holder(session_path: &Path) -> Option<u32> {
    let path = lock_path_for(session_path);
    let mut file = File::open(&path).ok()?;
    match file.try_lock() {
        // Dropping the file releases the probe lock.
        Ok(()) => None,
        Err(TryLockError::WouldBlock) => Some(read_pid(&mut file).unwrap_or(0)),
        Err(TryLockError::Error(_)) => None,
    }
}

fn read_pid(file: &mut File) -> Option<u32> {
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

fn io_error(e: std::io::Error) -> SessionLockError {
    SessionLockError::Io(e.to_string())
}
