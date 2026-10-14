//! Folder trust for project plugins. The grant is a fingerprint of the project's plugin folders,
//! kept in the trust file `hoocode-code-mcp` owns (`docs/design/mcp.md`: trust records are shared
//! with plugins). A change to any file in those folders asks again.

use std::fs;
use std::path::{Path, PathBuf};

use hoocode_code_mcp::{TrustError, TrustStore};
use sha2::{Digest, Sha256};

use crate::ProjectTrustPrompt;

/// The trust file in the hoocode data directory. A file that cannot be read trusts nothing: the
/// result is an empty store, so project plugins stay off and no approval is in force.
pub fn load_default_trust() -> TrustStore {
    let path = TrustStore::default_path();
    TrustStore::load(&path).unwrap_or_else(|_| TrustStore::empty(&path))
}

/// The plugin folders in `dir`: each child folder that holds a manifest, in name order.
pub(crate) fn plugin_roots(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut roots: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.is_dir()
                && (path.join("plugin.json").is_file()
                    || path.join(".claude-plugin").join("plugin.json").is_file())
        })
        .collect();
    roots.sort();
    roots
}

/// A hash of the plugin folders: each folder's name, and the path and bytes of every file in it.
/// Symlinked folders are not followed, so a link cannot change the grant silently.
pub(crate) fn fingerprint(roots: &[PathBuf]) -> String {
    let mut hasher = Sha256::new();
    for root in roots {
        hasher.update(
            root.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_bytes(),
        );
        hasher.update(b"\n");
        let mut files = Vec::new();
        collect_files(root, &mut files);
        for file in files {
            let rel = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .into_owned();
            let bytes = fs::read(&file).unwrap_or_default();
            hasher.update(rel.as_bytes());
            hasher.update(b"\0");
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();
    for path in paths {
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => collect_files(&path, out),
            Ok(_) => out.push(path),
            Err(_) => {}
        }
    }
}

/// Record the user's trust in the project plugins described by `prompt`, in the trust file at
/// `trust_path`. The next [`crate::load_plugins`] loads them.
pub fn trust_project_plugins(
    trust_path: &Path,
    prompt: &ProjectTrustPrompt,
) -> Result<(), TrustError> {
    let folder = prompt.folder.display().to_string();
    TrustStore::update(trust_path, |store| {
        store.grant_project_plugins(&folder, &prompt.fingerprint);
    })
}

/// Record the user's approval of the `allowed-tools` list `tools` for the plugin skill `skill`
/// (`<plugin>:<skill>`), in the trust file at `trust_path`.
pub fn approve_skill_tools(
    trust_path: &Path,
    skill: &str,
    tools: &[String],
) -> Result<(), TrustError> {
    TrustStore::update(trust_path, |store| store.approve_skill_tools(skill, tools))
}
