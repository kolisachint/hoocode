//! Installing, removing and updating plugins from marketplaces (part 2 of `docs/design/plugins.md`).
//!
//! An install copies the plugin's files into `~/.agents/plugins/<name>` (user) or
//! `<cwd>/.agents/plugins/<name>` (project). The source of each install, and whether its
//! marketplace is trusted, go in a record file in the hoocode data directory, outside any plugin
//! folder: a plugin's folder is its own content, and a change to it re-asks for project trust.
//! A plugin with no record (hand-placed in a plugin folder) keeps the loader's behaviour.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::load::LoadOptions;
use crate::manifest::{parse_agent_plugin_manifest, parse_claude_manifest, validate_name};
use crate::marketplace::{
    self, cache_path, ensure_cached, entry_path, read_index, Entry, EntrySource, Index,
    Marketplace, Registry, REGISTRY_FILE,
};
use crate::paths::{normalize, within};
use crate::trust::{fingerprint, load_default_trust};
use crate::PluginSet;

/// The record file, in the hoocode data directory.
pub const INSTALLS_FILE: &str = "plugin-installs.json";
/// The folder under `.agents` that holds plugins, and the user folder for their data.
const PLUGINS_DIR: &str = "plugins";
const PLUGIN_DATA_DIR: &str = "plugin-data";
/// Where installs are staged before they move into a plugin folder. Outside the plugin folders,
/// so a half-finished copy never loads.
const STAGING_DIR: &str = "plugin-staging";

/// Where a plugin is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    Project,
}

impl Scope {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "user" => Some(Self::User),
            "project" => Some(Self::Project),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// The folders an install reads and writes. [`Paths::for_cwd`] is the real layout;
/// tests build their own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// The project folder (normally the working directory).
    pub cwd: PathBuf,
    /// The user plugin folder (`~/.agents/plugins`).
    pub user_plugins: PathBuf,
    /// `PLUGIN_DATA` for each plugin (`~/.agents/plugin-data`).
    pub data_root: PathBuf,
    /// The hoocode data directory: the registry, the cache, the records and the trust file.
    pub state: PathBuf,
}

impl Paths {
    /// The real layout for `cwd`.
    pub fn for_cwd(cwd: &Path) -> Self {
        let agents = hoocode_code_paths::user_agents_dir();
        Self {
            cwd: cwd.to_path_buf(),
            user_plugins: agents.join(PLUGINS_DIR),
            data_root: agents.join(PLUGIN_DATA_DIR),
            state: hoocode_code_paths::agent_dir(),
        }
    }

    /// The folder a plugin installs into for `scope`.
    pub fn scope_dir(&self, scope: Scope) -> PathBuf {
        match scope {
            Scope::User => self.user_plugins.clone(),
            Scope::Project => self
                .cwd
                .join(hoocode_code_mcp::AGENTS_DIR_NAME)
                .join(PLUGINS_DIR),
        }
    }

    fn registry_path(&self) -> PathBuf {
        self.state.join(REGISTRY_FILE)
    }

    fn installs_path(&self) -> PathBuf {
        self.state.join(INSTALLS_FILE)
    }

    fn staging_dir(&self) -> PathBuf {
        self.state.join(STAGING_DIR)
    }

    /// The loader options for these folders. Installs from an untrusted marketplace are
    /// marked; an unreadable record file is reported, and the loader then trusts no plugin's
    /// marketplace.
    pub fn load_options<'a>(&self, trust: &'a hoocode_code_mcp::TrustStore) -> LoadOptions<'a> {
        let (untrusted_installs, installs_unreadable) = match read_records(&self.installs_path()) {
            Ok(records) => (
                records
                    .iter()
                    .filter(|r| !r.trusted)
                    .map(|r| r.root.clone())
                    .collect::<BTreeSet<PathBuf>>(),
                None,
            ),
            Err(error) => (BTreeSet::new(), Some(error)),
        };
        LoadOptions {
            project_folder: self.cwd.clone(),
            user_plugins_dir: self.user_plugins.clone(),
            data_root: self.data_root.clone(),
            trust,
            untrusted_installs,
            installs_unreadable,
        }
    }

    /// Load the plugins for these folders with the default trust store.
    pub fn load(&self) -> PluginSet {
        let trust = load_default_trust();
        crate::load_plugins(&self.load_options(&trust))
    }
}

/// One installed plugin, as its record names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    /// The canonical plugin folder.
    pub root: PathBuf,
    pub scope: Scope,
    /// The marketplace it came from.
    pub marketplace: String,
    /// The entry's source, as the marketplace wrote it (for the listing).
    pub source: String,
    pub version: String,
    /// The marketplace's trust when this install happened.
    pub trusted: bool,
}

fn read_records(path: &Path) -> Result<Vec<Record>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?;
    let mut out = Vec::new();
    for item in value
        .get("installs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_owned);
        let (Some(name), Some(root), Some(scope)) = (
            text("name"),
            text("root"),
            text("scope").and_then(|s| Scope::parse(&s)),
        ) else {
            return Err(format!(
                "{} has a record without a name, root or scope",
                path.display()
            ));
        };
        out.push(Record {
            name,
            root: PathBuf::from(root),
            scope,
            marketplace: text("marketplace").unwrap_or_default(),
            source: text("source").unwrap_or_default(),
            version: text("version").unwrap_or_default(),
            trusted: item
                .get("trusted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    Ok(out)
}

fn write_records(path: &Path, records: &[Record]) -> Result<(), String> {
    let items: Vec<Value> = records
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "root": r.root.to_string_lossy(),
                "scope": r.scope.label(),
                "marketplace": r.marketplace,
                "source": r.source,
                "version": r.version,
                "trusted": r.trusted,
            })
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::json!({ "installs": items }))
        .map_err(|e| e.to_string())?;
    marketplace::write_atomic(path, text.as_bytes())
}

/// The records of this install, for the listing and for updates.
pub fn records(paths: &Paths) -> Result<Vec<Record>, String> {
    read_records(&paths.installs_path())
}

/// What an install did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub scope: Scope,
    pub marketplace: String,
    /// False when the marketplace is not trusted: the plugin's MCP servers need a grant.
    pub trusted: bool,
    /// Warnings from fetching the marketplace.
    pub notes: Vec<String>,
}

/// Split `name@marketplace` (or `name`).
pub fn parse_spec(spec: &str) -> Result<(String, Option<String>), String> {
    let (name, marketplace) = match spec.split_once('@') {
        Some((name, marketplace)) => (name, Some(marketplace.to_owned())),
        None => (spec, None),
    };
    validate_name(name).map_err(|e| format!("plugin name \"{name}\": {e}"))?;
    if let Some(marketplace) = &marketplace {
        validate_name(marketplace)
            .map_err(|e| format!("marketplace name \"{marketplace}\": {e}"))?;
    }
    Ok((name.to_owned(), marketplace))
}

/// Fetch (when needed) and read a marketplace's index. The registry records the fetch.
fn refresh(
    paths: &Paths,
    registry: &mut Registry,
    marketplace: &Marketplace,
    force: bool,
) -> Result<(Index, Vec<String>), String> {
    let fetch = ensure_cached(&paths.state, marketplace, force)?;
    let mut notes = Vec::new();
    if let Some(at) = fetch.fetched_at {
        registry.set_fetched(&marketplace.name, at);
        registry.save(&paths.registry_path())?;
    }
    if let Some(warning) = fetch.warning {
        notes.push(warning);
    }
    let index = read_index(&cache_path(&paths.state, &marketplace.name))?;
    Ok((index, notes))
}

/// Copy the entry's plugin files into `stage`, which must not exist yet.
fn stage_entry(
    paths: &Paths,
    marketplace: &Marketplace,
    entry: &Entry,
    stage: &Path,
) -> Result<(), String> {
    let cache = cache_path(&paths.state, &marketplace.name);
    match &entry.source {
        EntrySource::Path(relative) => {
            let source = entry_path(&cache, relative)?;
            copy_tree(&source, stage)
        }
        EntrySource::Git {
            url,
            path,
            git_ref,
            sha,
        } => {
            let clone = paths.staging_dir().join(format!("clone-{}", entry.name));
            let _ = fs::remove_dir_all(&clone);
            fs::create_dir_all(paths.staging_dir()).map_err(|e| e.to_string())?;
            marketplace::clone_url(url, &clone)?;
            let result = (|| {
                if let Some(revision) = sha.as_ref().or(git_ref.as_ref()) {
                    marketplace::checkout(&clone, revision)?;
                }
                let source = match path {
                    Some(sub) => {
                        let rel = sub.strip_prefix("./").unwrap_or(sub);
                        let candidate = normalize(&clone.join(rel));
                        if !within(&clone, &candidate) {
                            return Err(format!("path \"{sub}\" leaves the repository"));
                        }
                        candidate
                    }
                    None => clone.clone(),
                };
                copy_tree(&source, stage)
            })();
            let _ = fs::remove_dir_all(&clone);
            result
        }
        EntrySource::Unsupported(kind) => Err(format!(
            "source type \"{kind}\" is not supported (hoocode installs path and git sources)"
        )),
    }
}

/// The manifest in a staged plugin: it must parse and name the plugin the marketplace lists.
fn check_manifest(root: &Path, name: &str) -> Result<(), String> {
    let agent = root.join("plugin.json");
    let claude = root.join(".claude-plugin").join("plugin.json");
    let manifest_name = if agent.is_file() {
        parse_agent_plugin_manifest(&read(&agent)?)?.name
    } else if claude.is_file() {
        parse_claude_manifest(&read(&claude)?)?.name
    } else {
        return Err("the plugin has no plugin.json or .claude-plugin/plugin.json".to_owned());
    };
    if manifest_name != name {
        return Err(format!(
            "the manifest names \"{manifest_name}\", but the marketplace lists \"{name}\""
        ));
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Copy a folder tree, without `.git`. Symbolic links are copied as links on Unix.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let read_dir =
        fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))?;
    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let source = entry.path();
        let target = to.join(&name);
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_tree(&source, &target)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            {
                let link = fs::read_link(&source).map_err(|e| e.to_string())?;
                std::os::unix::fs::symlink(link, &target).map_err(|e| e.to_string())?;
            }
        } else {
            fs::copy(&source, &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Install `spec` (`name` or `name@marketplace`) into `scope`. Without a marketplace, every
/// listed marketplace is searched, and a name in two of them is an error.
pub fn install(paths: &Paths, spec: &str, scope: Scope) -> Result<Installed, String> {
    let (name, marketplace_name) = parse_spec(spec)?;
    let mut registry = Registry::load(&paths.registry_path())?;
    let candidates: Vec<Marketplace> = match &marketplace_name {
        Some(wanted) => vec![registry.find(wanted).ok_or_else(|| {
            format!("no marketplace named \"{wanted}\"; add it with /plugin marketplace add")
        })?],
        None => registry.all(),
    };

    let mut found: Vec<(Marketplace, Index)> = Vec::new();
    let mut notes = Vec::new();
    for marketplace in candidates {
        match refresh(paths, &mut registry, &marketplace, false) {
            Ok((index, warnings)) => {
                notes.extend(warnings);
                if index.entry(&name).is_some() {
                    found.push((marketplace, index));
                }
            }
            Err(error) if marketplace_name.is_some() => return Err(error),
            // Searching: an unreachable marketplace cannot be the one that has it.
            Err(error) => notes.push(format!("skipped {}: {error}", marketplace.name)),
        }
    }
    if found.len() > 1 {
        let names: Vec<&str> = found.iter().map(|(m, _)| m.name.as_str()).collect();
        return Err(format!(
            "plugin \"{name}\" is in several marketplaces ({}); name one with {name}@<marketplace>",
            names.join(", ")
        ));
    }
    let Some((marketplace, index)) = found.into_iter().next() else {
        return Err(match marketplace_name {
            Some(wanted) => format!("marketplace \"{wanted}\" has no plugin named \"{name}\""),
            None => format!("no marketplace has a plugin named \"{name}\""),
        });
    };
    let entry = index
        .entry(&name)
        .cloned()
        .ok_or("internal: entry vanished")?;

    let scope_dir = paths.scope_dir(scope);
    let dest = scope_dir.join(&name);
    if dest.exists() {
        return Err(format!(
            "{name} is already installed at {}; remove it or run /plugin update {name}",
            dest.display()
        ));
    }
    let stage = paths.staging_dir().join(format!("install-{name}"));
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(paths.staging_dir()).map_err(|e| e.to_string())?;
    let staged = stage_entry(paths, &marketplace, &entry, &stage)
        .and_then(|()| check_manifest(&stage, &name));
    if let Err(error) = staged {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }

    // The record goes first, so a folder that appears always has its record. If the move fails,
    // the record is taken back out.
    fs::create_dir_all(&scope_dir).map_err(|e| e.to_string())?;
    let root = hoocode_code_paths::canonicalize_path(&scope_dir).join(&name);
    let record = Record {
        name: name.clone(),
        root: root.clone(),
        scope,
        marketplace: marketplace.name.clone(),
        source: source_text(&entry.source),
        version: entry.version.clone(),
        trusted: marketplace.trusted,
    };
    let mut records = records(paths)?;
    records.retain(|r| r.root != root);
    records.push(record);
    write_records(&paths.installs_path(), &records)?;
    if let Err(error) = fs::rename(&stage, &dest) {
        let _ = fs::remove_dir_all(&stage);
        records.retain(|r| r.root != root);
        write_records(&paths.installs_path(), &records)?;
        return Err(format!("cannot move the plugin into place: {error}"));
    }

    Ok(Installed {
        name,
        version: entry.version,
        root,
        scope,
        marketplace: marketplace.name,
        trusted: marketplace.trusted,
        notes,
    })
}

fn source_text(source: &EntrySource) -> String {
    match source {
        EntrySource::Path(path) => path.clone(),
        EntrySource::Git { url, path, .. } => match path {
            Some(path) => format!("{url} {path}"),
            None => url.clone(),
        },
        EntrySource::Unsupported(kind) => kind.clone(),
    }
}

/// What a remove did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removed {
    pub name: String,
    pub scope: Scope,
    /// The plugin's `PLUGIN_DATA` folder. It is kept, and this says where it is.
    pub data_dir: PathBuf,
}

/// Remove an installed plugin. Without a scope, it must be installed in one scope only.
pub fn remove(paths: &Paths, name: &str, scope: Option<Scope>) -> Result<Removed, String> {
    validate_name(name).map_err(|e| format!("plugin name \"{name}\": {e}"))?;
    let mut records = records(paths)?;
    let matches: Vec<Record> = records
        .iter()
        .filter(|r| r.name == name && scope.is_none_or(|s| s == r.scope))
        .cloned()
        .collect();
    let record = match matches.as_slice() {
        [] => return Err(format!("{name} is not installed")),
        [one] => one.clone(),
        _ => {
            return Err(format!(
                "{name} is installed in both scopes; pass --scope user or project"
            ))
        }
    };
    if record.root.exists() {
        fs::remove_dir_all(&record.root)
            .map_err(|e| format!("cannot remove {}: {e}", record.root.display()))?;
    }
    records.retain(|r| r.root != record.root);
    write_records(&paths.installs_path(), &records)?;
    Ok(Removed {
        name: record.name.clone(),
        scope: record.scope,
        data_dir: paths.data_root.join(&record.name),
    })
}

/// The result of updating one plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    Updated {
        name: String,
        from: String,
        to: String,
    },
    UpToDate {
        name: String,
    },
    Failed {
        name: String,
        reason: String,
    },
}

/// Update installed plugins (all, or `name`) from their marketplaces. The marketplace is fetched
/// first. A plugin whose files did not change is left alone.
pub fn update(paths: &Paths, name: Option<&str>) -> Result<Vec<UpdateOutcome>, String> {
    let records = records(paths)?;
    let targets: Vec<Record> = records
        .into_iter()
        .filter(|r| name.is_none_or(|n| n == r.name))
        .collect();
    if let Some(name) = name {
        if targets.is_empty() {
            return Err(format!("{name} is not installed"));
        }
    }
    let mut outcomes = Vec::new();
    for record in targets {
        let outcome = match update_one(paths, &record) {
            Ok(outcome) => outcome,
            Err(reason) => UpdateOutcome::Failed {
                name: record.name.clone(),
                reason,
            },
        };
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

fn update_one(paths: &Paths, record: &Record) -> Result<UpdateOutcome, String> {
    let mut registry = Registry::load(&paths.registry_path())?;
    let marketplace = registry
        .find(&record.marketplace)
        .ok_or_else(|| format!("marketplace \"{}\" is no longer listed", record.marketplace))?;
    let (index, _notes) = refresh(paths, &mut registry, &marketplace, true)?;
    let entry = index
        .entry(&record.name)
        .cloned()
        .ok_or_else(|| format!("{} is no longer in {}", record.name, marketplace.name))?;

    // Named after the plugin: the fingerprint hashes the folder name, so the comparison below
    // needs both folders to have it.
    let stage = paths.staging_dir().join("update").join(&record.name);
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(stage.parent().unwrap_or(&stage)).map_err(|e| e.to_string())?;
    fs::create_dir_all(paths.staging_dir()).map_err(|e| e.to_string())?;
    let staged = stage_entry(paths, &marketplace, &entry, &stage)
        .and_then(|()| check_manifest(&stage, &record.name));
    if let Err(error) = staged {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }

    if record.root.is_dir()
        && fingerprint(std::slice::from_ref(&stage))
            == fingerprint(std::slice::from_ref(&record.root))
    {
        let _ = fs::remove_dir_all(&stage);
        return Ok(UpdateOutcome::UpToDate {
            name: record.name.clone(),
        });
    }

    // Swap: the old folder goes aside, the new one moves in, then the old one is removed. If
    // the new folder cannot move in, the old one goes back.
    let old = paths.staging_dir().join(format!("old-{}", record.name));
    let _ = fs::remove_dir_all(&old);
    fs::rename(&record.root, &old).map_err(|e| format!("cannot replace the plugin: {e}"))?;
    if let Err(error) = fs::rename(&stage, &record.root) {
        let _ = fs::rename(&old, &record.root);
        let _ = fs::remove_dir_all(&stage);
        return Err(format!("cannot move the new version into place: {error}"));
    }
    let _ = fs::remove_dir_all(&old);

    let mut records = records(paths)?;
    for r in records.iter_mut().filter(|r| r.root == record.root) {
        r.version = entry.version.clone();
        r.source = source_text(&entry.source);
        r.trusted = marketplace.trusted;
    }
    write_records(&paths.installs_path(), &records)?;
    Ok(UpdateOutcome::Updated {
        name: record.name.clone(),
        from: record.version.clone(),
        to: entry.version,
    })
}

/// Add a marketplace by git URL, `owner/repo`, or local path. It is not trusted (only the
/// official directory is). Returns its name, read from the repository's index.
pub fn marketplace_add(paths: &Paths, source: &str) -> Result<String, String> {
    let url = marketplace::normalize_source(source);
    let mut registry = Registry::load(&paths.registry_path())?;
    if let Some(existing) = registry.all().into_iter().find(|m| m.url == url) {
        return Err(format!("{url} is already listed as \"{}\"", existing.name));
    }

    let cache_root = paths.state.join(marketplace::CACHE_DIR);
    fs::create_dir_all(&cache_root).map_err(|e| e.to_string())?;
    let staging = cache_root.join(".adding");
    let _ = fs::remove_dir_all(&staging);
    let index = match marketplace::clone_url(&url, &staging).and_then(|()| read_index(&staging)) {
        Ok(index) => index,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("cannot add {url}: {error}"));
        }
    };
    let name = index.name.clone();
    if registry.all().iter().any(|m| m.name == name) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("a marketplace named \"{name}\" is already listed"));
    }
    let target = cache_path(&paths.state, &name);
    let _ = fs::remove_dir_all(&target);
    fs::rename(&staging, &target).map_err(|e| e.to_string())?;

    registry.added.push(Marketplace {
        name: name.clone(),
        url,
        trusted: false,
        fetched_at: marketplace::now_secs(),
    });
    registry.save(&paths.registry_path())?;
    Ok(name)
}

/// The marketplaces listed: the official one and the ones added.
pub fn marketplace_list(paths: &Paths) -> Result<Vec<Marketplace>, String> {
    Ok(Registry::load(&paths.registry_path())?.all())
}

/// Fetch marketplaces now (all, or `name`). One line per marketplace, for the report.
pub fn marketplace_refresh(paths: &Paths, name: Option<&str>) -> Result<Vec<String>, String> {
    let mut registry = Registry::load(&paths.registry_path())?;
    let targets: Vec<Marketplace> = registry
        .all()
        .into_iter()
        .filter(|m| name.is_none_or(|n| n == m.name))
        .collect();
    if targets.is_empty() {
        return Err(match name {
            Some(name) => format!("no marketplace named \"{name}\""),
            None => "no marketplaces are listed".to_owned(),
        });
    }
    let mut lines = Vec::new();
    for marketplace in targets {
        match refresh(paths, &mut registry, &marketplace, true) {
            Ok((index, _)) => lines.push(format!(
                "{}: {} plugins",
                marketplace.name,
                index.entries.len()
            )),
            Err(error) => lines.push(format!("{}: {error}", marketplace.name)),
        }
    }
    Ok(lines)
}

/// The trust file the session uses (`TrustStore::default_path`).
pub fn default_trust_path() -> PathBuf {
    hoocode_code_mcp::TrustStore::default_path()
}

/// Trust the project plugins that are waiting for it (`/plugin trust`), in the trust file at
/// `trust_path`. Returns their names, or `None` when no project plugins are waiting.
pub fn trust_project(paths: &Paths, trust_path: &Path) -> Result<Option<Vec<String>>, String> {
    let trust = hoocode_code_mcp::TrustStore::load(trust_path)
        .unwrap_or_else(|_| hoocode_code_mcp::TrustStore::empty(trust_path));
    let set = crate::load_plugins(&paths.load_options(&trust));
    let Some(prompt) = set.pending_project_trust else {
        return Ok(None);
    };
    crate::trust_project_plugins(trust_path, &prompt)
        .map_err(|e| format!("could not save the trust grant: {e}"))?;
    Ok(Some(prompt.plugins))
}

/// The `/plugin list` text: marketplaces, installed plugins with what they contribute, and every
/// skipped part with its reason. A plugin with no record is hand-placed.
pub fn listing(paths: &Paths, set: &PluginSet, records: &[Record]) -> String {
    let mut lines = Vec::new();
    lines.push("Marketplaces".to_owned());
    match marketplace_list(paths) {
        Ok(list) if !list.is_empty() => {
            for m in list {
                let trust = if m.trusted { "trusted" } else { "not trusted" };
                let fetched = if m.fetched_at == 0 {
                    "not fetched yet".to_owned()
                } else {
                    format!("fetched at unix {}", m.fetched_at)
                };
                lines.push(format!("  {} ({trust}, {fetched}): {}", m.name, m.url));
            }
        }
        Ok(_) => lines.push("  none".to_owned()),
        Err(error) => lines.push(format!("  cannot read: {error}")),
    }

    lines.push("Plugins".to_owned());
    if set.plugins.is_empty() && records.is_empty() {
        lines.push("  none installed".to_owned());
    }
    for plugin in &set.plugins {
        let scope = match plugin.scope {
            crate::PluginScope::User => "user",
            crate::PluginScope::Project => "project",
        };
        let installed = records.iter().find(|r| r.root == plugin.root);
        let origin = match installed {
            Some(r) => format!("installed from {}", r.marketplace),
            None => "hand-placed".to_owned(),
        };
        let mcp = if plugin.marketplace_trusted || plugin.mcp_servers.is_empty() {
            ""
        } else {
            ", MCP servers need trust (/mcp)"
        };
        lines.push(format!(
            "  {} ({scope}, {origin}{mcp}): {}",
            plugin.name,
            plugin.root.display()
        ));
    }
    for record in records {
        if !set.plugins.iter().any(|p| p.root == record.root) {
            lines.push(format!(
                "  {} (installed {}, not loaded): {}",
                record.name,
                record.scope.label(),
                record.root.display()
            ));
        }
    }
    if let Some(prompt) = &set.pending_project_trust {
        lines.push(format!(
            "Project plugins waiting for trust: {}. Run /plugin trust to load them.",
            prompt.plugins.join(", ")
        ));
    }
    if !set.diagnostics.is_empty() {
        lines.push("Skipped or rejected".to_owned());
        for d in &set.diagnostics {
            let label = match &d.plugin {
                Some(name) => format!("plugin {name}"),
                None => d.path.display().to_string(),
            };
            lines.push(format!("  {label}: {}", d.message));
        }
    }
    lines.join("\n")
}
