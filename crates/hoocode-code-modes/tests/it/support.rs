//! Hermetic env for the mode tests. Mode files resolve from the agent dir
//! (`~/.hoocode/modes/<mode>/system.md` by default) and the subagent depth is
//! read from the process env, so a test that resolves a mode pins both.
//! The guard serializes on the process env and restores it on drop.

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

static ENV: Mutex<()> = Mutex::new(());

pub struct Hermetic {
    _lock: MutexGuard<'static, ()>,
    _agent_dir: tempfile::TempDir,
    saved: Vec<(String, Option<OsString>)>,
}

/// Points the agent dir at an empty temp dir and clears the subagent depth.
pub fn hermetic() -> Hermetic {
    let lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let agent_dir = tempfile::tempdir().unwrap();
    let mut saved = Vec::new();
    let mut set = |key: String, value: Option<&Path>| {
        saved.push((key.clone(), std::env::var_os(&key)));
        match value {
            Some(v) => std::env::set_var(&key, v),
            None => std::env::remove_var(&key),
        }
    };
    for prefix in hoocode_code_paths::ENV_PREFIXES {
        set(
            format!("{prefix}{}", hoocode_code_paths::ENV_AGENT_DIR),
            Some(agent_dir.path()),
        );
        set(format!("{prefix}SUBAGENT_DEPTH"), None);
    }
    Hermetic {
        _lock: lock,
        _agent_dir: agent_dir,
        saved,
    }
}

impl Drop for Hermetic {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(v) => std::env::set_var(&key, v),
                None => std::env::remove_var(&key),
            }
        }
    }
}
