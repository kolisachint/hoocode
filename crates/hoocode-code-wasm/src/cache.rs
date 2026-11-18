//! The precompiled `.cwasm` cache.
//!
//! A component is compiled once and kept as native code next to the others.
//! The file name is `<sha256 of the wasm bytes>-<wasmtime version>-<engine
//! config hash>.cwasm`, so a new wasm, a new wasmtime or a changed engine
//! setting each get their own file.
//!
//! Deserializing a `.cwasm` runs native code, so the folder must be ours: owned
//! by this user and closed to group and others. If it is not, or a write fails,
//! the component is compiled in memory and nothing is cached.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use wasmtime::component::Component;
use wasmtime::Engine;

use crate::error::WasmError;

/// The wasmtime release this host is built against. Part of every cache key.
pub const WASMTIME_VERSION: &str = "48.0.9";

/// An opened cache folder that passed the ownership and mode check.
pub(crate) struct Cache {
    dir: PathBuf,
    /// The uid of this process, as the files it creates are owned.
    uid: u32,
}

impl Cache {
    /// Create the folder if needed and check it. `Err` names why it cannot be
    /// used; the caller then runs without a cache.
    pub(crate) fn open(dir: &Path) -> Result<Self, String> {
        create_cache_dir(dir).map_err(|e| e.to_string())?;
        let uid = probe_owner(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = fs::metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            if !private_to(meta.uid(), meta.mode(), uid) {
                return Err(format!(
                    "{} is not owned by this user or is open to others",
                    dir.display()
                ));
            }
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            uid,
        })
    }

    /// Whether `file` belongs to this user, so it may be deserialized.
    fn owns(&self, file: &Path) -> bool {
        file_owner(file).is_ok_and(|uid| uid == self.uid)
    }
}

/// Create the cache folder, owner-only on unix.
pub fn create_cache_dir(dir: &Path) -> Result<(), WasmError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(|source| WasmError::Io {
        path: dir.to_path_buf(),
        source,
    })
}

/// True when a folder with this owner and mode is ours alone: owned by `uid`,
/// and no group or other bits set.
#[cfg(unix)]
fn private_to(owner: u32, mode: u32, uid: u32) -> bool {
    owner == uid && mode & 0o077 == 0
}

/// The owner of a file, as a uid. On platforms without uids every file counts
/// as owned by the same user.
#[cfg(unix)]
fn file_owner(path: &Path) -> io::Result<u32> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).map(|m| m.uid())
}

#[cfg(not(unix))]
fn file_owner(_path: &Path) -> io::Result<u32> {
    Ok(0)
}

/// The uid this process creates files with: the owner of a file it makes in
/// `dir`. The probe is removed again.
fn probe_owner(dir: &Path) -> io::Result<u32> {
    let probe = dir.join(format!(".owner-probe-{}", std::process::id()));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    drop(file);
    let owner = file_owner(&probe);
    let _ = fs::remove_file(&probe);
    owner
}

/// Hash of the engine settings that change compiled code. Part of every key.
fn engine_config_hash() -> String {
    let description = format!(
        "wasmtime={WASMTIME_VERSION};component-model;epoch-interruption;cranelift;arch={}",
        std::env::consts::ARCH
    );
    hex(&Sha256::digest(description.as_bytes()))[..16].to_string()
}

/// Load a component from `wasm`, using the precompiled cache when it holds a
/// usable entry. A corrupt entry is dropped and the component is compiled again.
/// With no cache, or when the cache cannot be written, the component is
/// compiled in memory.
pub fn load_component(
    engine: &Engine,
    cache: Option<&Cache>,
    wasm: &Path,
) -> Result<Component, WasmError> {
    let bytes = fs::read(wasm).map_err(|source| WasmError::Io {
        path: wasm.to_path_buf(),
        source,
    })?;
    let Some(cache) = cache else {
        return compile(engine, &bytes, wasm);
    };
    let key = hex(&Sha256::digest(&bytes));
    let file = cache.dir.join(format!(
        "{key}-{WASMTIME_VERSION}-{}.cwasm",
        engine_config_hash()
    ));

    if file.is_file() {
        if !cache.owns(&file) {
            crate::write_host_log(&format!(
                "wasm cache: {} is not owned by this user, compiling without the cache\n",
                file.display()
            ));
            return compile(engine, &bytes, wasm);
        }
        match deserialize_file(engine, &file) {
            Ok(component) => return Ok(component),
            Err(reason) => {
                crate::write_host_log(&format!(
                    "wasm cache: dropping {} ({reason}), recompiling\n",
                    file.display()
                ));
                let _ = fs::remove_file(&file);
            }
        }
    }

    let compiled = engine
        .precompile_component(&bytes)
        .map_err(|e| WasmError::Compile(format!("{}: {e}", wasm.display())))?;
    match write_atomic(&cache.dir, &file, &compiled) {
        Ok(()) => match deserialize_file(engine, &file) {
            Ok(component) => return Ok(component),
            Err(reason) => {
                crate::write_host_log(&format!(
                    "wasm cache: {} unusable after writing ({reason}), using the in-memory component\n",
                    file.display()
                ));
                let _ = fs::remove_file(&file);
            }
        },
        Err(error) => crate::write_host_log(&format!(
            "wasm cache: cannot write {}: {error}; using the in-memory component\n",
            file.display()
        )),
    }
    // SAFETY: `compiled` was produced by `precompile_component` on this engine
    // just above, in this process. Nothing else touched the bytes.
    unsafe { Component::deserialize(engine, &compiled) }
        .map_err(|e| WasmError::Compile(format!("{}: {e}", wasm.display())))
}

fn compile(engine: &Engine, bytes: &[u8], wasm: &Path) -> Result<Component, WasmError> {
    Component::new(engine, bytes)
        .map_err(|e| WasmError::Compile(format!("{}: {e}", wasm.display())))
}

fn deserialize_file(engine: &Engine, file: &Path) -> Result<Component, String> {
    // SAFETY: the file is in our own cache folder, which `Cache::open` checked
    // (owned by this user, 0700), and `Cache::owns` checked the file's owner. It
    // is named by the hash of the wasm it was compiled from. wasmtime checks its
    // own version and engine settings in the header and rejects a mismatch, so
    // a stale or corrupt file fails here instead of running wrong code.
    unsafe { Component::deserialize_file(engine, file) }.map_err(|e| e.to_string())
}

/// Write `bytes` to a temporary file in `dir`, then rename it to `file`, so a
/// reader never sees a half-written entry. The temporary file is removed on error.
fn write_atomic(dir: &Path, file: &Path, bytes: &[u8]) -> io::Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp: PathBuf = dir.join(format!(".tmp-{}-{nanos}", std::process::id()));
    let result = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, file));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn only_our_owner_and_no_other_bits_is_private() {
        assert!(private_to(1000, 0o700, 1000));
        assert!(private_to(1000, 0o500, 1000));
        assert!(!private_to(1000, 0o777, 1000));
        assert!(!private_to(1000, 0o750, 1000));
        assert!(!private_to(1000, 0o704, 1000));
        assert!(!private_to(0, 0o700, 1000), "another owner");
    }
}
