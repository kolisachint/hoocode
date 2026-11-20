//! Helper binary downloads: once per version, checksum verified, then installed.
//!
//! `~/.hoocode/plugin-bin/<plugin>/<version>/<file>` (spec D4). A present file
//! with a matching sha256 is reused; otherwise the manifest URL is fetched
//! through the proxy/CA-aware client, verified before an atomic rename, and
//! made executable. Archives (`.tar.gz`, `.zip`) are out of scope: an error.

use std::path::{Path, PathBuf};

use hoocode_code_plugins::NativeHelperEntry;
use sha2::{Digest, Sha256};

use crate::error::{plugin_bin_dir, NativeError};

/// Largest download accepted: 256 MiB of binary.
pub const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// URL endings that name an archive: plain binaries only.
const ARCHIVE_ENDINGS: [&str; 4] = [".tar.gz", ".tgz", ".zip", ".tar.bz2"];

/// The file name taken from a download URL: its last path segment. A URL
/// with no path, or a name that is empty or a dot segment, is an error.
pub fn file_name_from_url(url: &str) -> Result<String, NativeError> {
    let invalid =
        || NativeError::Download(format!("the binary URL has no usable file name: {url}"));
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let Some((_, path)) = after_scheme.split_once('/') else {
        return Err(invalid());
    };
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let name = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    if name.is_empty() || name == "." || name == ".." || name.contains(std::path::MAIN_SEPARATOR) {
        return Err(invalid());
    }
    Ok(name.to_owned())
}

/// Reject archive URLs before anything is fetched.
fn reject_archive(url: &str, plugin: &str) -> Result<(), NativeError> {
    let lower = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if ARCHIVE_ENDINGS.iter().any(|ending| lower.ends_with(ending)) {
        return Err(NativeError::Archive(format!(
            "the {plugin} helper ships as an archive ({url}): tier C installs plain binaries only"
        )));
    }
    Ok(())
}

/// The sha256 of `bytes`, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    crate::agents::hex_encode(&hasher.finalize())
}

/// Fail when `bytes` do not match the manifest `sha256` (any letter case).
pub fn verify_sha256(plugin: &str, bytes: &[u8], expected: &str) -> Result<(), NativeError> {
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected.trim()) {
        return Ok(());
    }
    Err(NativeError::Checksum {
        plugin: plugin.to_owned(),
        expected: expected.to_owned(),
        actual,
    })
}

/// Fetch `url` with `client`: 2xx only, capped at [`MAX_DOWNLOAD_BYTES`].
pub async fn fetch_bytes(
    client: &reqwest::Client,
    plugin: &str,
    url: &str,
) -> Result<Vec<u8>, NativeError> {
    let mut response = client.get(url).send().await.map_err(|error| {
        NativeError::Download(format!("cannot fetch the {plugin} helper: {error}"))
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(NativeError::Download(format!(
            "the {plugin} helper download failed: HTTP {status}"
        )));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_DOWNLOAD_BYTES {
            return Err(NativeError::Download(format!(
                "the {plugin} helper is {length} bytes: over the {MAX_DOWNLOAD_BYTES} byte limit"
            )));
        }
    }
    // Cap while reading too: Content-Length may lie or be missing.
    let mut bytes = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|error| {
            NativeError::Download(format!("cannot read the {plugin} helper: {error}"))
        })?;
        let Some(chunk) = chunk else {
            break;
        };
        if bytes.len() as u64 + chunk.len() as u64 > MAX_DOWNLOAD_BYTES {
            return Err(NativeError::Download(format!(
                "the {plugin} helper is over the {MAX_DOWNLOAD_BYTES} byte limit"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Ensure the downloaded binary for this platform: return its path, downloading
/// and installing it first when needed. `None` when this platform has no
/// `binaries` entry (the caller falls back to the mcp.json command).
///
/// `client` is injected so tests can pass a plain client at a local HTTP URL;
/// production builds it from
/// [`hoocode_ai_util::tls::http_client_builder`].
pub async fn ensure_plugin_binary(
    home: &Path,
    entry: &NativeHelperEntry,
    client: &reqwest::Client,
) -> Result<Option<PathBuf>, NativeError> {
    let Some(platform) = hoocode_code_plugins::native::current_platform_key() else {
        return Ok(None);
    };
    let Some(binary) = entry.helper.spec.binaries.get(platform) else {
        return Ok(None);
    };
    reject_archive(&binary.url, &entry.plugin)?;
    let name = file_name_from_url(&binary.url)?;
    let version = entry.version.as_deref().unwrap_or("unversioned");
    let dest = plugin_bin_dir(home, &entry.plugin, version).join(name);
    if let Ok(bytes) = std::fs::read(&dest) {
        if verify_sha256(&entry.plugin, &bytes, &binary.sha256).is_ok() {
            return Ok(Some(dest));
        }
    }
    let bytes = fetch_bytes(client, &entry.plugin, &binary.url).await?;
    verify_sha256(&entry.plugin, &bytes, &binary.sha256)?;
    install_bytes(&dest, &bytes)?;
    Ok(Some(dest))
}

/// Write `bytes` to `dest` atomically (temp file plus rename) and make it
/// executable. A checksum mismatch never reaches here, so a bad file is never
/// installed.
fn install_bytes(dest: &Path, bytes: &[u8]) -> Result<(), NativeError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            NativeError::Io(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    let tmp = dest.with_extension("tmp");
    std::fs::write(&tmp, bytes)
        .map_err(|error| NativeError::Io(format!("cannot write {}: {error}", tmp.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| NativeError::Io(format!("cannot chmod {}: {error}", tmp.display())))?;
    }
    std::fs::rename(&tmp, dest)
        .map_err(|error| NativeError::Io(format!("cannot install {}: {error}", dest.display())))?;
    Ok(())
}
