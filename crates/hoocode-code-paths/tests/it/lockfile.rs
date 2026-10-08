//! The proper-lockfile-compatible lock: a `<file>.lock` directory, fresh holders
//! are respected, stale ones (mtime older than the stale window) are taken over.

use std::fs::File;
use std::path::Path;
use std::time::{Duration, SystemTime};

use hoocode_code_paths::lockfile::{acquire_sync, lock_dir_for, LockError};

/// Makes `path`'s mtime `by` ago, as if its holder died long ago.
fn age(path: &Path, by: Duration) {
    let when = SystemTime::now() - by;
    File::open(path).unwrap().set_modified(when).unwrap();
}

#[test]
fn a_fresh_lock_directory_is_respected_and_not_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = lock_dir_for(&tmp.path().join("settings.json"));
    std::fs::create_dir(&lock).unwrap();
    assert!(matches!(acquire_sync(&lock), Err(LockError::Held)));
    assert!(lock.is_dir(), "a live holder's lock must stay");
}

#[test]
fn a_lock_released_during_the_wait_is_taken() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = lock_dir_for(&tmp.path().join("settings.json"));
    std::fs::create_dir(&lock).unwrap();
    let release = lock.clone();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        std::fs::remove_dir(release).unwrap();
    });
    let guard = acquire_sync(&lock);
    releaser.join().unwrap();
    assert!(guard.is_ok());
}

#[test]
fn a_stale_lock_directory_is_taken_over() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = lock_dir_for(&tmp.path().join("settings.json"));
    std::fs::create_dir(&lock).unwrap();
    age(&lock, Duration::from_secs(60));
    let guard = acquire_sync(&lock).expect("stale lock is taken over");
    assert!(lock.is_dir());
    drop(guard);
    assert!(!lock.exists(), "release removes the lock");
}

#[test]
fn our_lock_is_a_directory_not_a_file() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = lock_dir_for(&tmp.path().join("auth.json"));
    let guard = acquire_sync(&lock).unwrap();
    assert!(lock.is_dir());
    assert!(!lock.is_file());
    drop(guard);
    assert!(!lock.exists());
}

#[test]
fn a_stale_plain_file_from_the_old_fs4_lock_is_replaced_by_a_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = lock_dir_for(&tmp.path().join("settings.json"));
    std::fs::write(&lock, b"").unwrap();
    age(&lock, Duration::from_secs(60));
    let guard = acquire_sync(&lock).expect("old file lock is taken over");
    assert!(lock.is_dir());
    drop(guard);
    assert!(!lock.exists());
}

#[test]
fn a_fresh_plain_file_is_treated_as_a_live_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = lock_dir_for(&tmp.path().join("settings.json"));
    std::fs::write(&lock, b"").unwrap();
    assert!(matches!(acquire_sync(&lock), Err(LockError::Held)));
    assert!(lock.is_file());
}
