//! The trust store: which folders and plugins may start their MCP servers.
//!
//! A grant binds a trust key (a project folder, or `plugin:<id>`) to the set of servers that
//! was approved, through a fingerprint of `(name, command, args, url)`. Env and headers are not
//! part of it, and neither is `disabled`. The fingerprint uses the text as written in `mcp.json`:
//! a `${VAR}` stays unexpanded, so changing the variable's value (a rotated token) does not ask
//! for trust again (see [`crate::expand`]). When the declared set changes, the status becomes
//! [`TrustStatus::Changed`] with a [`ServerDiff`], and the caller asks again.
//!
//! The file lives under the hoocode data directory (`~/.hoocode/mcp-trust.json`), never in the
//! repository, so a repository cannot grant itself trust. It is written atomically with
//! owner-only permissions on Unix.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use hoocode_code_paths::lockfile::{self, LockError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{ServerDef, Source, Transport};

/// The trust file name inside the hoocode data directory.
pub const TRUST_FILE_NAME: &str = "mcp-trust.json";
const FORMAT_VERSION: u32 = 1;

/// The fields a grant binds to, for one server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedServer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl ApprovedServer {
    fn from_def(def: &ServerDef) -> Self {
        match &def.transport {
            Transport::Stdio { command, args, .. } => Self {
                command: Some(command.clone()),
                args: args.clone(),
                url: None,
            },
            Transport::Http { url, .. } => Self {
                command: None,
                args: Vec::new(),
                url: Some(url.clone()),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Grant {
    fingerprint: String,
    servers: BTreeMap<String, ApprovedServer>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct TrustFile {
    version: u32,
    grants: BTreeMap<String, Grant>,
}

/// What changed since the grant, by server name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerDiff {
    /// Declared now, not in the grant.
    pub added: Vec<String>,
    /// In the grant, no longer declared.
    pub removed: Vec<String>,
    /// Declared in both, with a different command, args or url.
    pub changed: Vec<String>,
}

impl ServerDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Whether a source's servers may start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustStatus {
    /// The user's own file: never needs a grant.
    NotRequired,
    /// Granted, and the declared set matches the grant (or the plugin came from a trusted
    /// marketplace, or it declares no servers).
    Trusted,
    /// No grant for this folder or plugin yet.
    Untrusted,
    /// A grant exists, but the declared set changed since. The diff says how.
    Changed(ServerDiff),
}

/// Fingerprint of a server set: `sha256:` over the sorted `(name, command, args, url)` tuples.
///
/// Order in the file does not matter. Env, headers and `disabled` are not part of it.
pub fn fingerprint(servers: &[ServerDef]) -> String {
    let mut lines: Vec<String> = servers
        .iter()
        .map(|def| {
            let approved = ApprovedServer::from_def(def);
            serde_json::json!([def.name, approved.command, approved.args, approved.url]).to_string()
        })
        .collect();
    lines.sort();
    let mut hasher = Sha256::new();
    for line in &lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// The change from an approved set to the declared one, by name.
pub fn diff(approved: &BTreeMap<String, ApprovedServer>, declared: &[ServerDef]) -> ServerDiff {
    let current: BTreeMap<&str, ApprovedServer> = declared
        .iter()
        .map(|def| (def.name.as_str(), ApprovedServer::from_def(def)))
        .collect();
    let mut out = ServerDiff::default();
    for (name, server) in &current {
        match approved.get(*name) {
            None => out.added.push((*name).to_owned()),
            Some(old) if old != server => out.changed.push((*name).to_owned()),
            Some(_) => {}
        }
    }
    for name in approved.keys() {
        if !current.contains_key(name.as_str()) {
            out.removed.push(name.clone());
        }
    }
    out
}

/// Errors from the trust file.
#[derive(Debug)]
pub enum TrustError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    /// The file was written by a newer format.
    UnsupportedVersion(u32),
    /// Another hoocode process holds the trust file's lock.
    Locked(PathBuf),
}

impl std::fmt::Display for TrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrustError::Io(e) => write!(f, "trust file: {e}"),
            TrustError::Parse(e) => write!(f, "trust file is not valid JSON: {e}"),
            TrustError::UnsupportedVersion(v) => {
                write!(f, "trust file version {v} is not supported")
            }
            TrustError::Locked(path) => write!(f, "trust file is locked: {}", path.display()),
        }
    }
}

impl std::error::Error for TrustError {}

impl From<std::io::Error> for TrustError {
    fn from(e: std::io::Error) -> Self {
        TrustError::Io(e)
    }
}

/// The loaded trust file and the path it lives at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustStore {
    path: PathBuf,
    file: TrustFile,
}

impl TrustStore {
    /// `<hoocode data dir>/mcp-trust.json` (`HOOCODE_CODING_AGENT_DIR` moves the data dir).
    pub fn default_path() -> PathBuf {
        hoocode_code_paths::agent_dir().join(TRUST_FILE_NAME)
    }

    /// An empty store that will save to `path`.
    pub fn empty(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            file: TrustFile {
                version: FORMAT_VERSION,
                grants: BTreeMap::new(),
            },
        }
    }

    /// Read the store at `path`. A missing file is an empty store. A corrupt file is an error,
    /// so a broken file never silently grants or drops trust.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, TrustError> {
        let path = path.into();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Self::empty(path)),
            Err(e) => return Err(TrustError::Io(e)),
        };
        let file: TrustFile = serde_json::from_str(&text).map_err(TrustError::Parse)?;
        if file.version != FORMAT_VERSION {
            return Err(TrustError::UnsupportedVersion(file.version));
        }
        Ok(Self { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Approve the servers `servers` declares for `key`, replacing any earlier grant.
    /// `servers` should be all valid entries of the source (see [`crate::config::McpConfig::declared`]).
    pub fn grant(&mut self, key: &str, servers: &[ServerDef]) {
        let grant = Grant {
            fingerprint: fingerprint(servers),
            servers: servers
                .iter()
                .map(|def| (def.name.clone(), ApprovedServer::from_def(def)))
                .collect(),
        };
        self.file.grants.insert(key.to_owned(), grant);
    }

    /// Remove the grant for `key`. Returns whether there was one.
    pub fn revoke(&mut self, key: &str) -> bool {
        self.file.grants.remove(key).is_some()
    }

    /// Whether `key` has any grant.
    pub fn has_grant(&self, key: &str) -> bool {
        self.file.grants.contains_key(key)
    }

    /// Trust status of `source` whose declared servers are `declared`.
    pub fn status(&self, source: &Source, declared: &[ServerDef]) -> TrustStatus {
        let Some(key) = source.trust_key() else {
            return TrustStatus::NotRequired;
        };
        if declared.is_empty() {
            return TrustStatus::Trusted;
        }
        if let Source::Plugin {
            marketplace_trusted: true,
            ..
        } = source
        {
            return TrustStatus::Trusted;
        }
        match self.file.grants.get(&key) {
            None => TrustStatus::Untrusted,
            Some(grant) if grant.fingerprint == fingerprint(declared) => TrustStatus::Trusted,
            Some(grant) => TrustStatus::Changed(diff(&grant.servers, declared)),
        }
    }

    /// Write the store atomically (temp file in the same folder, then rename), owner-only.
    pub fn save(&self) -> Result<(), TrustError> {
        let mut text = serde_json::to_string_pretty(&self.file).map_err(TrustError::Parse)?;
        text.push('\n');
        write_atomic(&self.path, text.as_bytes())?;
        Ok(())
    }

    /// Load, apply `change`, and save, all under the `<file>.lock` lock that hoocode-ts also
    /// takes on its settings files. Use this for grants made from the UI, so two processes
    /// cannot lose each other's grant.
    pub fn update(path: &Path, change: impl FnOnce(&mut TrustStore)) -> Result<(), TrustError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _guard =
            lockfile::acquire_sync(&lockfile::lock_dir_for(path)).map_err(|e| match e {
                LockError::Held => TrustError::Locked(path.to_path_buf()),
                LockError::Io(e) => TrustError::Io(e),
            })?;
        let mut store = Self::load(path)?;
        change(&mut store);
        store.save()
    }
}

/// Write `content` to `path` through a temp file in the same folder, fsync, then rename over
/// `path`. The file is owner-only (0600) on Unix.
pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(content)?;
    tmp.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    tmp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    {
        // Make the rename itself durable.
        if let Ok(dir_file) = std::fs::File::open(dir) {
            let _ = dir_file.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ServerDef, Source, Transport};
    use std::collections::BTreeMap;

    fn stdio(name: &str, command: &str, args: &[&str]) -> ServerDef {
        ServerDef {
            name: name.to_owned(),
            source: Source::User {
                path: PathBuf::from("/x"),
            },
            transport: Transport::Stdio {
                command: command.to_owned(),
                args: args.iter().map(|a| (*a).to_owned()).collect(),
                env: BTreeMap::new(),
                cwd: None,
            },
            disabled: false,
        }
    }

    fn project_source(folder: &Path) -> Source {
        Source::Project {
            folder: folder.to_path_buf(),
            path: folder.join(".agents/mcp.json"),
        }
    }

    #[test]
    fn fingerprint_ignores_order_env_and_disabled() {
        let a = stdio("a", "x", &["1"]);
        let mut b = stdio("b", "y", &[]);
        let base = fingerprint(&[a.clone(), b.clone()]);
        assert_eq!(base, fingerprint(&[b.clone(), a.clone()]));

        if let Transport::Stdio { env, .. } = &mut b.transport {
            env.insert("TOKEN".into(), "secret".into());
        }
        b.disabled = true;
        assert_eq!(base, fingerprint(&[a.clone(), b]));
    }

    #[test]
    fn fingerprint_changes_with_command_args_url_or_name() {
        let base = fingerprint(&[stdio("a", "x", &["1"])]);
        assert_ne!(base, fingerprint(&[stdio("a", "z", &["1"])]));
        assert_ne!(base, fingerprint(&[stdio("a", "x", &["2"])]));
        assert_ne!(base, fingerprint(&[stdio("b", "x", &["1"])]));
        assert!(base.starts_with("sha256:"));
    }

    #[test]
    fn fingerprint_uses_the_unexpanded_text() {
        // Expansion happens at connect time; the parsed defs, and so the fingerprint, keep `${VAR}`.
        let mut server = stdio("a", "npx", &["${PKG}"]);
        if let Transport::Stdio { env, .. } = &mut server.transport {
            env.insert("TOKEN".into(), "${TOKEN}".into());
        }
        let defs = [server.clone()];
        let before = fingerprint(&defs);
        let v1 = |name: &str| match name {
            "PKG" => Some("pkg-1.0".to_owned()),
            "TOKEN" => Some("token-1".to_owned()),
            _ => None,
        };
        let v2 = |name: &str| match name {
            "PKG" => Some("pkg-2.0".to_owned()),
            "TOKEN" => Some("token-2".to_owned()),
            _ => None,
        };
        let expanded_1 = server.transport.expanded(&v1).unwrap();
        let expanded_2 = server.transport.expanded(&v2).unwrap();
        assert_ne!(expanded_1, expanded_2);
        assert_eq!(
            fingerprint(&defs),
            before,
            "expanding must not change the fingerprint"
        );
        assert_eq!(fingerprint(&[stdio("a", "npx", &["${PKG}"])]), before);
        assert_ne!(fingerprint(&[stdio("a", "npx", &["pkg-1.0"])]), before);
    }

    #[test]
    fn user_source_needs_no_grant_and_untrusted_project_asks() {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustStore::empty(dir.path().join("trust.json"));
        let user = Source::User {
            path: dir.path().join("u.json"),
        };
        let servers = [stdio("a", "x", &[])];
        assert_eq!(store.status(&user, &servers), TrustStatus::NotRequired);

        let folder = dir.path().join("repo");
        let project = project_source(&folder);
        assert_eq!(store.status(&project, &servers), TrustStatus::Untrusted);
        assert_eq!(store.status(&project, &[]), TrustStatus::Trusted);
    }

    #[test]
    fn grant_trusts_until_the_server_set_changes_and_reports_the_diff() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("repo");
        let source = project_source(&folder);
        let key = source.trust_key().unwrap();
        let mut store = TrustStore::empty(dir.path().join("trust.json"));

        let approved = vec![
            stdio("keep", "same", &[]),
            stdio("edit", "old", &["a"]),
            stdio("drop", "gone", &[]),
        ];
        store.grant(&key, &approved);
        assert!(store.has_grant(&key));
        assert_eq!(store.status(&source, &approved), TrustStatus::Trusted);

        // Env-only change does not re-ask.
        let mut env_changed = approved.clone();
        if let Transport::Stdio { env, .. } = &mut env_changed[0].transport {
            env.insert("X".into(), "1".into());
        }
        assert_eq!(store.status(&source, &env_changed), TrustStatus::Trusted);

        // Add one, change one, drop one.
        let next = vec![
            stdio("keep", "same", &[]),
            stdio("edit", "new", &["a"]),
            stdio("fresh", "added", &[]),
        ];
        match store.status(&source, &next) {
            TrustStatus::Changed(d) => {
                assert_eq!(d.added, vec!["fresh".to_owned()]);
                assert_eq!(d.removed, vec!["drop".to_owned()]);
                assert_eq!(d.changed, vec!["edit".to_owned()]);
            }
            other => panic!("expected Changed, got {other:?}"),
        }

        // Re-granting the new set makes it trusted again; revoking drops it.
        store.grant(&key, &next);
        assert_eq!(store.status(&source, &next), TrustStatus::Trusted);
        assert!(store.revoke(&key));
        assert!(!store.revoke(&key));
        assert_eq!(store.status(&source, &next), TrustStatus::Untrusted);
    }

    #[test]
    fn marketplace_trusted_plugins_need_no_grant() {
        let store = TrustStore::empty("/unused/trust.json");
        let plugin = |marketplace_trusted| Source::Plugin {
            id: "p".into(),
            path: PathBuf::from("/unused/p/mcp.json"),
            marketplace_trusted,
        };
        assert_eq!(
            store.status(&plugin(true), &[stdio("a", "x", &[])]),
            TrustStatus::Trusted
        );
        let local = plugin(false);
        assert_eq!(
            store.status(&local, &[stdio("a", "x", &[])]),
            TrustStatus::Untrusted
        );
    }

    #[test]
    fn store_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/mcp-trust.json");
        let folder = dir.path().join("repo");
        let source = project_source(&folder);
        let key = source.trust_key().unwrap();
        let servers = [stdio("a", "x", &["y"])];

        TrustStore::update(&path, |store| store.grant(&key, &servers)).unwrap();
        let loaded = TrustStore::load(&path).unwrap();
        assert_eq!(loaded.status(&source, &servers), TrustStatus::Trusted);

        // Missing file is an empty store.
        let missing = TrustStore::load(dir.path().join("none.json")).unwrap();
        assert_eq!(missing.status(&source, &servers), TrustStatus::Untrusted);
    }

    #[test]
    fn corrupt_or_newer_store_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        std::fs::write(&path, "{nope").unwrap();
        assert!(matches!(TrustStore::load(&path), Err(TrustError::Parse(_))));
        std::fs::write(&path, r#"{"version": 9, "grants": {}}"#).unwrap();
        assert!(matches!(
            TrustStore::load(&path),
            Err(TrustError::UnsupportedVersion(9))
        ));
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["trust.json".to_owned()]);
    }

    #[cfg(unix)]
    #[test]
    fn saved_trust_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        let store = TrustStore::empty(&path);
        store.save().unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn update_fails_fast_when_locked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        let lock = lockfile::lock_dir_for(&path);
        std::fs::create_dir_all(&lock).unwrap();
        // A fresh lock directory is held by someone else.
        let result = TrustStore::update(&path, |_| {});
        assert!(matches!(result, Err(TrustError::Locked(_))), "{result:?}");
    }
}
