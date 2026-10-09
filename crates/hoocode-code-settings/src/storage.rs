//! `settings-storage.ts`: read-modify-write of one scope's raw settings JSON
//! under an exclusive lock.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hoocode_code_paths::lockfile::{self, LockError, LockGuard};
use hoocode_code_paths::CONFIG_DIR_NAME;

/// `SettingsScope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SettingsScope {
    Global,
    Project,
}

impl SettingsScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

/// Why a settings file could not be read or written.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// The file parsed but is not a JSON object.
    NotAnObject,
    /// Another process held the lock for every retry (`ELOCKED`).
    Locked(PathBuf),
    /// An entry of `scopedModels` that does not parse. It is skipped, and the
    /// message names its position.
    InvalidScopedModel(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
            Self::NotAnObject => f.write_str("settings must be a JSON object"),
            Self::Locked(path) => write!(f, "Lock file is already being held: {}", path.display()),
            Self::InvalidScopedModel(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// `SettingsError`: a load or write failure, kept for `drain_errors`.
#[derive(Debug)]
pub struct SettingsError {
    pub scope: SettingsScope,
    pub error: Error,
}

/// The update callback: current file content in, new content (or `None` to
/// leave the file alone) out.
pub type LockFn<'a> = &'a mut dyn FnMut(Option<&str>) -> Result<Option<String>, Error>;

/// `SettingsStorage`.
pub trait SettingsStorage: Send + Sync {
    fn with_lock(&self, scope: SettingsScope, f: LockFn<'_>) -> Result<(), Error>;
}

/// `FileSettingsStorage`: `<agentDir>/settings.json` and
/// `<cwd>/.hoocode/settings.json`.
///
/// Locking is hoocode-ts's proper-lockfile: a `settings.json.lock` directory beside
/// the settings file (see `hoocode_code_paths::lockfile`), so the two tools exclude
/// each other. Writes go through a temp file and a rename.
pub struct FileSettingsStorage {
    global_path: PathBuf,
    project_path: PathBuf,
}

impl FileSettingsStorage {
    pub fn new(cwd: impl AsRef<Path>, agent_dir: impl AsRef<Path>) -> Self {
        Self {
            global_path: agent_dir.as_ref().join("settings.json"),
            project_path: cwd.as_ref().join(CONFIG_DIR_NAME).join("settings.json"),
        }
    }

    pub fn path(&self, scope: SettingsScope) -> &Path {
        match scope {
            SettingsScope::Global => &self.global_path,
            SettingsScope::Project => &self.project_path,
        }
    }

    fn lock(path: &Path) -> Result<LockGuard, Error> {
        lockfile::acquire_sync(&lockfile::lock_dir_for(path)).map_err(|e| match e {
            LockError::Held => Error::Locked(path.to_path_buf()),
            LockError::Io(e) => Error::Io(e),
        })
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), Error> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| Error::Io(e.error))?;
    Ok(())
}

impl SettingsStorage for FileSettingsStorage {
    fn with_lock(&self, scope: SettingsScope, f: LockFn<'_>) -> Result<(), Error> {
        let path = self.path(scope);
        // Only lock (and so create the lock file) when the file exists or we
        // are about to write it.
        let mut guard = None;
        let current = if path.exists() {
            guard = Some(Self::lock(path)?);
            Some(std::fs::read_to_string(path)?)
        } else {
            None
        };
        if let Some(next) = f(current.as_deref())? {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            if guard.is_none() {
                guard = Some(Self::lock(path)?);
            }
            write_atomic(path, &next)?;
        }
        drop(guard);
        Ok(())
    }
}

/// `InMemorySettingsStorage`.
#[derive(Default)]
pub struct InMemorySettingsStorage {
    files: Mutex<[Option<String>; 2]>,
}

impl InMemorySettingsStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// The stored content of a scope.
    pub fn get(&self, scope: SettingsScope) -> Option<String> {
        self.files.lock().unwrap_or_else(|e| e.into_inner())[scope as usize].clone()
    }
}

impl SettingsStorage for InMemorySettingsStorage {
    fn with_lock(&self, scope: SettingsScope, f: LockFn<'_>) -> Result<(), Error> {
        let mut files = self.files.lock().unwrap_or_else(|e| e.into_inner());
        let slot = &mut files[scope as usize];
        if let Some(next) = f(slot.as_deref())? {
            *slot = Some(next);
        }
        Ok(())
    }
}
