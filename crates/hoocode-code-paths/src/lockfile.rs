//! proper-lockfile-compatible locks: the lock hoocode-ts takes on a file it shares
//! with us (`auth.json`, `settings.json`). `mkdir <file>.lock` holds the lock, and a
//! lock directory whose mtime is older than `stale` is taken over. Both tools use
//! the same directory, so neither can mistake the other's lock for a file.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Sync lock: `lockSync` (stale after 10 s), retried 10 times 20 ms apart.
pub const SYNC_STALE: Duration = Duration::from_secs(10);
pub const SYNC_ATTEMPTS: u32 = 10;
pub const SYNC_DELAY: Duration = Duration::from_millis(20);
/// Async lock: `lock(..., {retries: 10, factor: 2, minTimeout: 100, maxTimeout: 10000,
/// randomize: true}, stale: 30000})`.
pub const ASYNC_STALE: Duration = Duration::from_secs(30);
pub const ASYNC_RETRIES: u32 = 10;

/// Why a sync lock could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another holder kept the lock for every retry (`ELOCKED`).
    Held,
    Io(std::io::Error),
}

/// The lock directory for `file`: `<file>.lock`.
pub fn lock_dir_for(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// A held lock. Removes the lock directory on drop (`release()`).
#[derive(Debug)]
pub struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

pub enum LockAttempt {
    Acquired(LockGuard),
    Locked,
}

fn is_stale(lock_dir: &Path, stale: Duration) -> bool {
    std::fs::metadata(lock_dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|mtime| SystemTime::now().duration_since(mtime).ok())
        .is_some_and(|age| age > stale)
}

/// One acquisition: `mkdir lock_dir`. An existing lock older than `stale` is
/// removed and taken over. That includes a plain file at the lock path, which
/// an earlier build of this crate left behind when it locked with fs4.
pub fn try_lock(lock_dir: &Path, stale: Duration) -> std::io::Result<LockAttempt> {
    match std::fs::create_dir(lock_dir) {
        Ok(()) => Ok(LockAttempt::Acquired(LockGuard(lock_dir.to_path_buf()))),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            if !is_stale(lock_dir, stale) {
                return Ok(LockAttempt::Locked);
            }
            let _ = if lock_dir.is_dir() {
                std::fs::remove_dir(lock_dir)
            } else {
                std::fs::remove_file(lock_dir)
            };
            match std::fs::create_dir(lock_dir) {
                Ok(()) => Ok(LockAttempt::Acquired(LockGuard(lock_dir.to_path_buf()))),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(LockAttempt::Locked),
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    }
}

/// `lockSync`: up to [`SYNC_ATTEMPTS`] tries, [`SYNC_DELAY`] apart. A lock held
/// by someone else is never removed (unless stale).
pub fn acquire_sync(lock_dir: &Path) -> Result<LockGuard, LockError> {
    for attempt in 1..=SYNC_ATTEMPTS {
        match try_lock(lock_dir, SYNC_STALE).map_err(LockError::Io)? {
            LockAttempt::Acquired(guard) => return Ok(guard),
            LockAttempt::Locked if attempt < SYNC_ATTEMPTS => std::thread::sleep(SYNC_DELAY),
            LockAttempt::Locked => {}
        }
    }
    Err(LockError::Held)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_dir_is_the_file_name_with_a_lock_suffix() {
        assert_eq!(
            lock_dir_for(Path::new("/h/.hoocode/settings.json")),
            PathBuf::from("/h/.hoocode/settings.json.lock")
        );
    }
}
