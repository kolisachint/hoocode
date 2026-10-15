//! What the model's plugin tools do (`docs/design/plugins.md`, part 3).
//!
//! Searching reads every listed marketplace, fetching a stale one as `/plugin` does. Installing
//! goes through [`install::install`], and only from a trusted marketplace: an install from any
//! other one is for the user to make with `/plugin install`. Adding a marketplace is not here at
//! all.

use std::path::Path;

use hoocode_code_mcp::TrustStore;

use crate::install::{self, Installed, Paths, Scope};
use crate::marketplace::{cache_path, ensure_cached, read_index, Entry, Registry, REGISTRY_FILE};
use crate::{load_plugins, set_current, PluginSet};

/// A plugin a marketplace offers, with the marketplace it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub marketplace: String,
    pub trusted: bool,
    pub entry: Entry,
}

/// The plugins offered by the listed marketplaces (only `only`, when given). Each marketplace is
/// fetched when stale. One that cannot be read is skipped, with a note, as a search should.
fn offered(paths: &Paths, only: Option<&str>) -> Result<(Vec<Found>, Vec<String>), String> {
    let registry_path = paths.state.join(REGISTRY_FILE);
    let mut registry = Registry::load(&registry_path)?;
    let mut found = Vec::new();
    let mut notes = Vec::new();
    for marketplace in registry.all() {
        if only.is_some_and(|name| name != marketplace.name) {
            continue;
        }
        let fetch = match ensure_cached(&paths.state, &marketplace, false) {
            Ok(fetch) => fetch,
            Err(error) => {
                notes.push(format!("skipped {}: {error}", marketplace.name));
                continue;
            }
        };
        if let Some(at) = fetch.fetched_at {
            registry.set_fetched(&marketplace.name, at);
            registry.save(&registry_path)?;
        }
        if let Some(warning) = fetch.warning {
            notes.push(warning);
        }
        match read_index(&cache_path(&paths.state, &marketplace.name)) {
            Ok(index) => found.extend(index.entries.into_iter().map(|entry| Found {
                marketplace: marketplace.name.clone(),
                trusted: marketplace.trusted,
                entry,
            })),
            Err(error) => notes.push(format!("skipped {}: {error}", marketplace.name)),
        }
    }
    Ok((found, notes))
}

/// The plugins whose name or description contains `query` (case-insensitive; an empty query
/// matches all), from every listed marketplace, with notes on any marketplace that was skipped.
pub fn search(paths: &Paths, query: &str) -> Result<(Vec<Found>, Vec<String>), String> {
    let needle = query.trim().to_lowercase();
    let (found, notes) = offered(paths, None)?;
    let matches = found
        .into_iter()
        .filter(|f| {
            needle.is_empty()
                || f.entry.name.to_lowercase().contains(&needle)
                || f.entry.description.to_lowercase().contains(&needle)
        })
        .collect();
    Ok((matches, notes))
}

/// Install `spec` (`name` or `name@marketplace`) from a trusted marketplace. The plugin must be
/// in exactly one listed marketplace, and that marketplace must be trusted.
pub fn install_trusted(paths: &Paths, spec: &str, scope: Scope) -> Result<Installed, String> {
    let (name, wanted) = install::parse_spec(spec)?;
    let (found, _notes) = offered(paths, wanted.as_deref())?;
    let mut matches: Vec<_> = found.into_iter().filter(|f| f.entry.name == name).collect();
    let found = match matches.len() {
        0 => {
            return Err(match &wanted {
                Some(marketplace) => {
                    format!("marketplace \"{marketplace}\" has no plugin named \"{name}\"")
                }
                None => format!("no marketplace has a plugin named \"{name}\""),
            })
        }
        1 => matches.remove(0),
        _ => {
            let names: Vec<&str> = matches.iter().map(|f| f.marketplace.as_str()).collect();
            return Err(format!(
                "plugin \"{name}\" is in several marketplaces ({}); name one with {name}@<marketplace>",
                names.join(", ")
            ));
        }
    };
    if !found.trusted {
        return Err(format!(
            "{name} is in {}, which is not trusted, so the model does not install from it. \
             The user can install it with /plugin install {name}@{}",
            found.marketplace, found.marketplace
        ));
    }
    install::install(paths, &format!("{name}@{}", found.marketplace), scope)
}

/// Load the plugins for `paths`, reading the trust file at `trust_path`. An unreadable trust
/// file trusts nothing, as the loader does at session start.
pub fn load(paths: &Paths, trust_path: &Path) -> PluginSet {
    let trust = TrustStore::load(trust_path).unwrap_or_else(|_| TrustStore::empty(trust_path));
    load_plugins(&paths.load_options(&trust))
}

/// [`load`], and make the result the current plugins, so the skill loader and `/plugin list` see
/// a change a tool made.
pub fn reload(paths: &Paths, trust_path: &Path) -> PluginSet {
    let set = load(paths, trust_path);
    set_current(set.clone());
    set
}
