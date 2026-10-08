//! `AuthStorageBackend`: where `auth.json` lives and how it is locked.

use hoocode_ai_oauth::BoxFuture;
use hoocode_code_paths::lockfile::{
    self, LockAttempt, LockError, LockGuard, ASYNC_RETRIES, ASYNC_STALE,
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Given the current file contents, return the contents to write (`None` leaves
/// the file untouched). An `Err` aborts without writing.
pub type LockFn<'a> = &'a mut dyn FnMut(Option<&str>) -> Result<Option<String>, String>;

/// The async variant of [`LockFn`] (an OAuth refresh runs while the lock is held).
pub type AsyncLockFn<'a> =
    Box<dyn FnOnce(Option<String>) -> BoxFuture<'a, Result<Option<String>, String>> + Send + 'a>;

/// `AuthStorageBackend`: read-modify-write under a lock.
pub trait AuthStorageBackend: Send + Sync {
    /// `withLock`.
    fn with_lock(&self, f: LockFn<'_>) -> Result<(), String>;
    /// `withLockAsync`.
    fn with_lock_async<'a>(&'a self, f: AsyncLockFn<'a>) -> BoxFuture<'a, Result<(), String>>;
}

/// `InMemoryAuthStorageBackend`.
#[derive(Default)]
pub struct InMemoryAuthStorageBackend {
    value: Mutex<Option<String>>,
}

impl AuthStorageBackend for InMemoryAuthStorageBackend {
    fn with_lock(&self, f: LockFn<'_>) -> Result<(), String> {
        let mut value = self.value.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(next) = f(value.as_deref())? {
            *value = Some(next);
        }
        Ok(())
    }

    fn with_lock_async<'a>(&'a self, f: AsyncLockFn<'a>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let current = self.value.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(next) = f(current).await? {
                *self.value.lock().unwrap_or_else(|e| e.into_inner()) = Some(next);
            }
            Ok(())
        })
    }
}

/// `FileAuthStorageBackend`: `auth.json` guarded by a `proper-lockfile` compatible
/// lock (an `auth.json.lock` directory), so hoocode and hoocode can share the file.
pub struct FileAuthStorageBackend {
    auth_path: PathBuf,
}

fn locked_error() -> String {
    "Lock file is already being held".to_string()
}

impl FileAuthStorageBackend {
    pub fn new(auth_path: impl Into<PathBuf>) -> Self {
        Self {
            auth_path: auth_path.into(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.auth_path
    }

    fn lock_dir(&self) -> PathBuf {
        lockfile::lock_dir_for(&self.auth_path)
    }

    /// `ensureParentDir` (mode 0700) + `ensureFileExists` (`{}`, mode 0600).
    fn ensure_file(&self) -> Result<(), String> {
        if let Some(dir) = self.auth_path.parent() {
            if !dir.exists() {
                create_private_dir(dir).map_err(|e| e.to_string())?;
            }
        }
        if !self.auth_path.exists() {
            write_private(&self.auth_path, "{}")?;
        }
        Ok(())
    }

    fn read(&self) -> Result<Option<String>, String> {
        match std::fs::read_to_string(&self.auth_path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn acquire_sync(&self) -> Result<LockGuard, String> {
        lockfile::acquire_sync(&self.lock_dir()).map_err(|e| match e {
            LockError::Held => locked_error(),
            LockError::Io(e) => e.to_string(),
        })
    }

    async fn acquire_async(&self) -> Result<LockGuard, String> {
        let lock_dir = self.lock_dir();
        for retry in 0..=ASYNC_RETRIES {
            if let LockAttempt::Acquired(guard) =
                lockfile::try_lock(&lock_dir, ASYNC_STALE).map_err(|e| e.to_string())?
            {
                return Ok(guard);
            }
            if retry < ASYNC_RETRIES {
                // `retry` package timeouts: min(random(1..2) * 100ms * 2^n, 10s).
                let factor: f64 = 1.0 + rand::random::<f64>();
                let ms = (factor * 100.0 * 2f64.powi(retry as i32)).min(10_000.0);
                tokio::time::sleep(Duration::from_millis(ms as u64)).await;
            }
        }
        Err(locked_error())
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// `writeFileSync` + `chmodSync(0o600)`.
fn write_private(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl AuthStorageBackend for FileAuthStorageBackend {
    fn with_lock(&self, f: LockFn<'_>) -> Result<(), String> {
        self.ensure_file()?;
        let _guard = self.acquire_sync()?;
        let current = self.read()?;
        if let Some(next) = f(current.as_deref())? {
            write_private(&self.auth_path, &next)?;
        }
        Ok(())
    }

    fn with_lock_async<'a>(&'a self, f: AsyncLockFn<'a>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.ensure_file()?;
            let _guard = self.acquire_async().await?;
            let current = self.read()?;
            if let Some(next) = f(current).await? {
                write_private(&self.auth_path, &next)?;
            }
            Ok(())
        })
    }
}
