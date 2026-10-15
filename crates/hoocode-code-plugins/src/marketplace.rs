//! Marketplaces: Claude's `.claude-plugin/marketplace.json`, read only (`docs/design/plugins.md`).
//!
//! A marketplace is a git repository. It is cloned into the hoocode data directory with the
//! `git` command, so the user's git credentials apply, and re-fetched when its index is older
//! than [`REFRESH_SECS`]. Nothing is ever written back to a marketplace.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::manifest::validate_name;
use crate::paths::{normalize, within};

/// The official Anthropic directory. Pre-trusted: installs from it count as trusted.
pub const OFFICIAL_NAME: &str = "claude-plugins-official";
/// Its repository. Not stated in the design docs; this is the assumed location.
pub const OFFICIAL_URL: &str = "https://github.com/anthropics/claude-plugins-official";
/// How old a cached index may be before a command fetches it again: one day.
pub const REFRESH_SECS: u64 = 24 * 60 * 60;

/// The registry file, in the hoocode data directory.
pub const REGISTRY_FILE: &str = "plugin-marketplaces.json";
/// The cache folder, in the hoocode data directory. One clone per marketplace, named by it.
pub const CACHE_DIR: &str = "plugin-marketplace-cache";
/// The index file inside a marketplace repository.
pub const INDEX_PATH: &str = ".claude-plugin/marketplace.json";

/// A marketplace the user has added (or the built-in official one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marketplace {
    pub name: String,
    /// The git URL or local path the cache was cloned from.
    pub url: String,
    /// Installs from a trusted marketplace need no further grant. Only the official directory
    /// is trusted; a marketplace the user adds is not.
    pub trusted: bool,
    /// Unix seconds of the last fetch, or 0.
    pub fetched_at: u64,
}

/// The marketplaces the user added. The official directory is always listed (see [`Registry::all`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    pub added: Vec<Marketplace>,
}

impl Registry {
    /// Read the registry at `path`. A missing file is an empty registry.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?;
        let mut added = Vec::new();
        for item in value
            .get("marketplaces")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(name), Some(url)) = (
                item.get("name").and_then(Value::as_str),
                item.get("url").and_then(Value::as_str),
            ) else {
                continue;
            };
            added.push(Marketplace {
                name: name.to_owned(),
                url: url.to_owned(),
                trusted: item
                    .get("trusted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                fetched_at: item.get("fetchedAt").and_then(Value::as_u64).unwrap_or(0),
            });
        }
        Ok(Self { added })
    }

    /// Write the registry to `path`, atomically.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let items: Vec<Value> = self
            .added
            .iter()
            .map(|m| {
                serde_json::json!({
                    "name": m.name,
                    "url": m.url,
                    "trusted": m.trusted,
                    "fetchedAt": m.fetched_at,
                })
            })
            .collect();
        let text = serde_json::to_string_pretty(&serde_json::json!({ "marketplaces": items }))
            .map_err(|e| e.to_string())?;
        write_atomic(path, text.as_bytes())
    }

    /// Every marketplace: the official directory first, then the added ones.
    pub fn all(&self) -> Vec<Marketplace> {
        let mut out = vec![official(self.official_fetched_at())];
        out.extend(
            self.added
                .iter()
                .filter(|m| m.name != OFFICIAL_NAME)
                .cloned(),
        );
        out
    }

    /// The marketplace called `name`, if it is listed.
    pub fn find(&self, name: &str) -> Option<Marketplace> {
        self.all().into_iter().find(|m| m.name == name)
    }

    fn official_fetched_at(&self) -> u64 {
        self.added
            .iter()
            .find(|m| m.name == OFFICIAL_NAME)
            .map_or(0, |m| m.fetched_at)
    }

    /// Record a fetch of `name` at `at`. Returns false when it is not listed.
    pub fn set_fetched(&mut self, name: &str, at: u64) -> bool {
        if name == OFFICIAL_NAME && !self.added.iter().any(|m| m.name == name) {
            self.added.push(official(at));
            return true;
        }
        match self.added.iter_mut().find(|m| m.name == name) {
            Some(m) => {
                m.fetched_at = at;
                true
            }
            None => false,
        }
    }
}

fn official(fetched_at: u64) -> Marketplace {
    Marketplace {
        name: OFFICIAL_NAME.to_owned(),
        url: OFFICIAL_URL.to_owned(),
        trusted: true,
        fetched_at,
    }
}

/// What an entry's `source` says to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntrySource {
    /// A `./` path inside the marketplace repository.
    Path(String),
    /// A git repository, optionally a subfolder, at a ref or commit.
    Git {
        url: String,
        path: Option<String>,
        git_ref: Option<String>,
        sha: Option<String>,
    },
    /// A kind hoocode does not install (npm, for one). Named for the error.
    Unsupported(String),
}

/// One plugin a marketplace offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub description: String,
    pub version: String,
    pub source: EntrySource,
}

/// A parsed `marketplace.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    pub entries: Vec<Entry>,
    /// Entries that were skipped, each with its reason.
    pub warnings: Vec<String>,
}

impl Index {
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

/// Parse a `marketplace.json`. A bad marketplace name or a missing `plugins` list is an error;
/// a bad entry is skipped with a warning.
pub fn parse_index(text: &str) -> Result<Index, String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| format!("marketplace.json is not valid JSON: {e}"))?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or("marketplace.json has no \"name\"")?;
    validate_name(name).map_err(|e| format!("marketplace name: {e}"))?;
    let plugins = value
        .get("plugins")
        .and_then(Value::as_array)
        .ok_or("marketplace.json has no \"plugins\" list")?;

    let mut entries: Vec<Entry> = Vec::new();
    let mut warnings = Vec::new();
    for item in plugins {
        match parse_entry(item) {
            Ok(entry) => {
                if entries.iter().any(|e| e.name == entry.name) {
                    warnings.push(format!(
                        "plugin \"{}\" is listed twice; the second entry is skipped",
                        entry.name
                    ));
                } else {
                    entries.push(entry);
                }
            }
            Err(reason) => warnings.push(reason),
        }
    }
    Ok(Index {
        name: name.to_owned(),
        entries,
        warnings,
    })
}

fn parse_entry(item: &Value) -> Result<Entry, String> {
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .ok_or("an entry has no \"name\"; skipped")?;
    validate_name(name).map_err(|e| format!("entry \"{name}\": {e}; skipped"))?;
    let text = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let source = match item.get("source") {
        Some(Value::String(path)) => EntrySource::Path(path.clone()),
        Some(object @ Value::Object(_)) => {
            let kind = object.get("source").and_then(Value::as_str).unwrap_or("");
            let field = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
            match kind {
                "github" => {
                    let repo = field("repo")
                        .ok_or_else(|| format!("entry \"{name}\": github source has no repo"))?;
                    EntrySource::Git {
                        url: format!("https://github.com/{repo}.git"),
                        path: None,
                        git_ref: field("ref"),
                        sha: field("sha"),
                    }
                }
                "url" => EntrySource::Git {
                    url: field("url")
                        .ok_or_else(|| format!("entry \"{name}\": url source has no url"))?,
                    path: None,
                    git_ref: field("ref"),
                    sha: field("sha"),
                },
                "git-subdir" => EntrySource::Git {
                    url: field("url")
                        .ok_or_else(|| format!("entry \"{name}\": git-subdir source has no url"))?,
                    path: field("path"),
                    git_ref: field("ref"),
                    sha: field("sha"),
                },
                other => EntrySource::Unsupported(other.to_owned()),
            }
        }
        _ => return Err(format!("entry \"{name}\" has no source; skipped")),
    };
    Ok(Entry {
        name: name.to_owned(),
        description: text("description"),
        version: text("version"),
        source,
    })
}

/// What a marketplace argument means: `owner/repo` is a GitHub repository; anything else is a
/// git URL or a local path, used as written (a local path is made absolute).
pub fn normalize_source(text: &str) -> String {
    let text = text.trim();
    let shorthand = text.split('/').count() == 2
        && !text.contains(':')
        && !text.starts_with('.')
        && !text.starts_with('/')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'));
    if shorthand {
        return format!("https://github.com/{text}.git");
    }
    let path = Path::new(text);
    if path.exists() {
        return fs::canonicalize(path)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| text.to_owned());
    }
    text.to_owned()
}

/// Outcome of [`ensure_cached`].
#[derive(Debug, PartialEq, Eq)]
pub struct Fetch {
    /// The new fetch time, when the cache was cloned or fetched.
    pub fetched_at: Option<u64>,
    /// A refresh that failed while an older cache still exists. The cache is used.
    pub warning: Option<String>,
}

/// The cache folder of marketplace `name` under `data_dir`.
pub fn cache_path(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join(CACHE_DIR).join(name)
}

/// Make sure the cache of `marketplace` exists and, unless `force` is false and it is fresh,
/// is up to date. Clones on first use. A failed refresh falls back to the old cache when
/// `force` is false; with `force`, the error is returned.
pub fn ensure_cached(
    data_dir: &Path,
    marketplace: &Marketplace,
    force: bool,
) -> Result<Fetch, String> {
    let now = now_secs();
    let dir = cache_path(data_dir, &marketplace.name);
    if !dir.join(INDEX_PATH).is_file() {
        clone_checked(data_dir, marketplace)?;
        return Ok(Fetch {
            fetched_at: Some(now),
            warning: None,
        });
    }
    if !force && now.saturating_sub(marketplace.fetched_at) < REFRESH_SECS {
        return Ok(Fetch {
            fetched_at: None,
            warning: None,
        });
    }
    match git(&["fetch", "--quiet", "origin"], Some(&dir))
        .and_then(|()| git(&["reset", "--quiet", "--hard", "FETCH_HEAD"], Some(&dir)))
    {
        Ok(()) => Ok(Fetch {
            fetched_at: Some(now),
            warning: None,
        }),
        Err(error) if force => Err(format!("cannot refresh {}: {error}", marketplace.name)),
        Err(error) => Ok(Fetch {
            fetched_at: None,
            warning: Some(format!(
                "could not refresh {}, using the cached index: {error}",
                marketplace.name
            )),
        }),
    }
}

/// Clone `marketplace` into a staging folder, check its index name, then move it into place.
fn clone_checked(data_dir: &Path, marketplace: &Marketplace) -> Result<(), String> {
    let cache_root = data_dir.join(CACHE_DIR);
    fs::create_dir_all(&cache_root).map_err(|e| e.to_string())?;
    let staging = cache_root.join(format!(".{}.cloning", marketplace.name));
    let _ = fs::remove_dir_all(&staging);
    let result = clone_url(&marketplace.url, &staging).and_then(|()| {
        let index = read_index(&staging)?;
        if index.name != marketplace.name {
            return Err(format!(
                "the repository's index is named \"{}\", not \"{}\"",
                index.name, marketplace.name
            ));
        }
        Ok(())
    });
    match result {
        Ok(()) => {
            let target = cache_path(data_dir, &marketplace.name);
            let _ = fs::remove_dir_all(&target);
            fs::rename(&staging, &target).map_err(|e| e.to_string())
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

/// Read and parse the index of a checked-out marketplace folder.
pub fn read_index(root: &Path) -> Result<Index, String> {
    let text = fs::read_to_string(root.join(INDEX_PATH))
        .map_err(|e| format!("no {INDEX_PATH} in this marketplace: {e}"))?;
    parse_index(&text)
}

/// Resolve an entry's `./` path inside `root`. `None` when it leaves the marketplace.
pub fn entry_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let rest = relative
        .strip_prefix("./")
        .ok_or_else(|| format!("source \"{relative}\" must be a ./ path"))?;
    let candidate = normalize(&root.join(rest));
    if within(root, &candidate) {
        Ok(candidate)
    } else {
        Err(format!("source \"{relative}\" leaves the marketplace"))
    }
}

/// Clone `url` into a new folder `dest` (a full clone, so a commit can be checked out).
pub fn clone_url(url: &str, dest: &Path) -> Result<(), String> {
    check_git_value(url, "repository")?;
    let dest_text = dest.to_string_lossy().into_owned();
    git(&["clone", "--quiet", "--", url, &dest_text], None)
}

/// Check out `revision` (a branch, tag or commit) in `dir`.
pub fn checkout(dir: &Path, revision: &str) -> Result<(), String> {
    check_git_value(revision, "revision")?;
    git(&["checkout", "--quiet", revision], Some(dir))
}

/// Refuse a value from a marketplace that git would read as something other than a plain
/// repository or revision: an option (`--upload-pack=...`), or a transport helper that runs a
/// program (`ext::`, `fd::`, any `name::`). Plain paths and `scheme://` URLs pass.
pub fn check_git_value(text: &str, what: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err(format!("the {what} is empty"));
    }
    if text.starts_with('-') {
        return Err(format!(
            "the {what} \"{text}\" starts with '-', so git would read it as an option"
        ));
    }
    if let Some(at) = text.find("::") {
        let helper = &text[..at];
        if !helper.is_empty()
            && helper
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return Err(format!(
                "the {what} \"{text}\" names a git transport helper, which is refused"
            ));
        }
    }
    Ok(())
}

/// Run `git` with the given arguments. Credential prompts are off: a prompt would hang the
/// TUI, and the user's credential helper still answers. The error is the first line of stderr.
pub fn git(args: &[&str], dir: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new("git");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    let output = command
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let first = stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("git failed");
    Err(first.trim().to_owned())
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Write `bytes` to `path` through a temporary file in the same folder. The temporary name is
/// unique to this process and call, so two writers never share one file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}-{n}.tmp", std::process::id()));
    let result = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, path));
    result.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_values_that_are_options_or_helpers_are_refused() {
        assert!(check_git_value("--upload-pack=touch x", "repository").is_err());
        assert!(check_git_value("-b", "revision").is_err());
        assert!(check_git_value("ext::sh -c touch% x", "repository").is_err());
        assert!(check_git_value("fd::3", "repository").is_err());
        assert!(check_git_value("", "revision").is_err());
        assert!(check_git_value("https://example.com/a::b.git", "repository").is_ok());
        assert!(check_git_value("file:///tmp/repo", "repository").is_ok());
        assert!(check_git_value("/tmp/repo", "repository").is_ok());
        assert!(check_git_value("v1.2.3", "revision").is_ok());
    }

    #[test]
    fn clone_and_checkout_refuse_option_values_before_running_git() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        let error = clone_url("--upload-pack=touch pwned", &dest).unwrap_err();
        assert!(error.contains("option"), "{error}");
        assert!(clone_url("ext::sh -c touch pwned", &dest).is_err());
        let error = checkout(dir.path(), "--orphan=x").unwrap_err();
        assert!(error.contains("option"), "{error}");
        assert!(!dest.exists());
    }

    #[test]
    fn a_bad_entry_name_is_skipped_not_joined_into_a_path() {
        let text = r#"{"name": "mk", "plugins": [
            {"name": "../escape", "source": "./plugins/x"},
            {"name": "good", "source": "./plugins/good"}]}"#;
        let index = parse_index(text).unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].name, "good");
        assert_eq!(index.warnings.len(), 1);
    }

    // A test-only race: plain threads, since no runtime is involved here.
    #[allow(clippy::disallowed_methods)]
    #[test]
    fn concurrent_writers_never_leave_a_torn_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("records.json");
        let writers: Vec<_> = (0..8u8)
            .map(|id| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        let payload = vec![b'a' + id; 4096];
                        write_atomic(&path, &payload).unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let text = fs::read(&path).unwrap();
        assert_eq!(text.len(), 4096);
        assert!(text.iter().all(|b| *b == text[0]), "a torn write");
        let leftovers = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1, "a temporary file was left behind");
    }
}
