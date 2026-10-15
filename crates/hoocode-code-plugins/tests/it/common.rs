//! Shared helpers: fixture paths and a sandbox with a project folder, a user plugin folder and
//! a trust file, all in a temporary directory.

use std::path::{Path, PathBuf};

use hoocode_code_mcp::TrustStore;
use hoocode_code_plugins::{load_plugins, LoadOptions, LoadedPlugin, PluginSet};

/// `tests/fixtures` of this crate.
pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The fixture user plugin folder. Loading reads it in place; data goes to the sandbox.
pub fn user_fixtures() -> PathBuf {
    fixtures().join("plugins/user")
}

pub struct Sandbox {
    _dir: tempfile::TempDir,
    /// The project folder (`<cwd>`).
    pub repo: PathBuf,
    /// The user plugin folder used by the loader.
    pub user_plugins: PathBuf,
    /// The data root used by the loader.
    pub data_root: PathBuf,
    /// The trust file (where a project grant is stored).
    pub trust_path: PathBuf,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let sandbox = Self {
            repo: base.join("repo"),
            user_plugins: user_fixtures(),
            data_root: base.join("home/.agents/plugin-data"),
            trust_path: base.join("home/.hoocode/mcp-trust.json"),
            _dir: dir,
        };
        std::fs::create_dir_all(&sandbox.repo).unwrap();
        sandbox
    }

    /// The path of the project plugins folder (`<repo>/.agents/plugins`).
    pub fn project_plugins(&self) -> PathBuf {
        self.repo.join(".agents/plugins")
    }

    /// Copy a fixture plugin folder into the project plugins folder.
    pub fn add_project_plugin(&self, fixture_name: &str) -> PathBuf {
        let from = fixtures().join("plugins/project").join(fixture_name);
        let to = self.project_plugins().join(fixture_name);
        copy_dir(&from, &to);
        to
    }

    /// Load with the given trust store.
    pub fn load(&self, trust: &TrustStore) -> PluginSet {
        load_plugins(&self.options(trust))
    }

    pub fn options<'a>(&self, trust: &'a TrustStore) -> LoadOptions<'a> {
        LoadOptions {
            project_folder: self.repo.clone(),
            user_plugins_dir: self.user_plugins.clone(),
            data_root: self.data_root.clone(),
            trust,
            untrusted_installs: std::collections::BTreeSet::new(),
            installs_unreadable: None,
        }
    }

    /// The trust file as it is on disk now.
    pub fn trust(&self) -> TrustStore {
        TrustStore::load(&self.trust_path).unwrap()
    }
}

/// The loaded plugin named `name`. Panics when it did not load.
pub fn plugin<'a>(set: &'a PluginSet, name: &str) -> &'a LoadedPlugin {
    set.plugins
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("plugin {name} did not load: {:?}", set.diagnostics))
}

pub fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Copy a folder tree (files and folders, not symlinks).
pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
