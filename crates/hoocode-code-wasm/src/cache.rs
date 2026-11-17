//! The precompiled `.cwasm` cache.
//!
//! A component is compiled once and kept as native code next to the others.
//! The file name is `<sha256 of the wasm bytes>-<wasmtime version>-<engine
//! config hash>.cwasm`, so a new wasm, a new wasmtime or a changed engine
//! setting each get their own file.

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
pub fn load_component(
    engine: &Engine,
    cache_dir: &Path,
    wasm: &Path,
) -> Result<Component, WasmError> {
    let bytes = fs::read(wasm).map_err(|source| WasmError::Io {
        path: wasm.to_path_buf(),
        source,
    })?;
    let key = hex(&Sha256::digest(&bytes));
    let file = cache_dir.join(format!(
        "{key}-{WASMTIME_VERSION}-{}.cwasm",
        engine_config_hash()
    ));

    if file.is_file() {
        match deserialize(engine, &file) {
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
    write_atomic(cache_dir, &file, &compiled)?;
    deserialize(engine, &file)
        .map_err(|reason| WasmError::Compile(format!("{}: {reason}", wasm.display())))
}

fn deserialize(engine: &Engine, file: &Path) -> Result<Component, String> {
    // SAFETY: the file is in our own cache folder (0700, owned by the user) and
    // is named by the hash of the wasm it was compiled from. wasmtime checks its
    // own version and engine settings in the header and rejects a mismatch, so
    // a stale or corrupt file fails here instead of running wrong code.
    unsafe { Component::deserialize_file(engine, file) }.map_err(|e| e.to_string())
}

/// Write `bytes` to a temporary file in `dir`, then rename it to `file`, so a
/// reader never sees a half-written entry.
fn write_atomic(dir: &Path, file: &Path, bytes: &[u8]) -> Result<(), WasmError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp: PathBuf = dir.join(format!(".tmp-{}-{nanos}", std::process::id()));
    let io_err = |path: &Path, source: io::Error| WasmError::Io {
        path: path.to_path_buf(),
        source,
    };
    fs::write(&tmp, bytes).map_err(|e| io_err(&tmp, e))?;
    fs::rename(&tmp, file).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_err(file, e)
    })
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
