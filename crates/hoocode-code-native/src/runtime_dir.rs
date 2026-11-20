//! The per-session runtime dir: one 0700 folder holding every helper socket.
//!
//! `$XDG_RUNTIME_DIR/hoocode-<rand8>/`, else `<temp>/hoocode-<rand8>/`
//! (spec D4). Removed at session end (`Supervisor::shutdown`) and when the
//! supervisor drops.

use std::path::{Path, PathBuf};

use crate::error::NativeError;

/// Sockets must fit the OS sun_path limit (macOS 104, Linux 108): stay under 100.
pub const MAX_SOCKET_PATH_BYTES: usize = 100;

/// Internal (not in `--help`, tests only): when set to an absolute path, the supervisor uses
/// it as the session runtime dir (created when missing) instead of a fresh random folder, so
/// an integration test can watch for the helpers' sockets and logs. It is read from the
/// process environment only, never from plugin config, so a plugin cannot switch it on. Never
/// set it anywhere else.
pub const INTERNAL_RUNTIME_DIR_ENV: &str = "HOOCODE_INTERNAL_NATIVE_RUNTIME_DIR";

/// The test override dir, when [`INTERNAL_RUNTIME_DIR_ENV`] names an absolute path. The dir is
/// created (mode 0700) when missing; like the random dir, it is removed at session end.
pub fn internal_runtime_dir() -> Option<PathBuf> {
    let dir = std::env::var_os(INTERNAL_RUNTIME_DIR_ENV).map(PathBuf::from)?;
    if !dir.is_absolute() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir.is_dir().then_some(dir)
}

/// The random suffix of the runtime dir: 8 hex characters.
pub fn random_suffix() -> String {
    let mut bytes = [0u8; 4];
    rand::fill(&mut bytes);
    crate::agents::hex_encode(&bytes)
}

/// Make the session runtime dir, mode 0700. A fresh random name per session, so
/// sessions never share a helper.
pub fn create_runtime_dir() -> Result<PathBuf, NativeError> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("hoocode-{}", random_suffix()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|error| {
                NativeError::Io(format!(
                    "cannot create runtime dir {}: {error}",
                    dir.display()
                ))
            })?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&dir).map_err(|error| {
            NativeError::Io(format!(
                "cannot create runtime dir {}: {error}",
                dir.display()
            ))
        })?;
    }
    Ok(dir)
}

/// The socket path for one helper: `<dir>/<plugin>.sock`. When that is 100
/// bytes or more, a shorter hash name is used instead; when even that does not
/// fit, starting the helper fails with a clear error.
pub fn socket_path_for(dir: &Path, plugin: &str) -> Result<PathBuf, NativeError> {
    let direct = dir.join(format!("{plugin}.sock"));
    if path_bytes(&direct) < MAX_SOCKET_PATH_BYTES {
        return Ok(direct);
    }
    let short = dir.join(format!("{}.sock", fnv1a_hex(plugin)));
    if path_bytes(&short) < MAX_SOCKET_PATH_BYTES {
        return Ok(short);
    }
    Err(NativeError::Spawn(format!(
        "the socket path for plugin {plugin:?} does not fit in {MAX_SOCKET_PATH_BYTES} bytes"
    )))
}

/// FNV-1a over the plugin name: a stable short fallback socket name.
fn fnv1a_hex(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn path_bytes(path: &Path) -> usize {
    path.as_os_str().as_encoded_bytes().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_use_the_plugin_socket() {
        let dir = Path::new("/tmp/hoocode-abcdef12");
        assert_eq!(
            socket_path_for(dir, "browser").unwrap(),
            dir.join("browser.sock")
        );
    }

    #[test]
    fn long_names_fall_back_to_a_short_hash() {
        let dir = Path::new("/tmp/hoocode-abcdef12");
        let long = "p".repeat(200);
        let fallback = socket_path_for(dir, &long).unwrap();
        assert!(path_bytes(&fallback) < MAX_SOCKET_PATH_BYTES);
        assert_ne!(fallback, dir.join(format!("{long}.sock")));
        // Stable: the same plugin always gets the same fallback.
        assert_eq!(fallback, socket_path_for(dir, &long).unwrap());
    }

    #[test]
    fn an_overlong_dir_fails_clearly() {
        let dir = PathBuf::from(format!("/tmp/{}", "d".repeat(200)));
        assert!(matches!(
            socket_path_for(&dir, "browser"),
            Err(NativeError::Spawn(_))
        ));
    }
}
