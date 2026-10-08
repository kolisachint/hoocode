//! Target `settings_json`: global and project `settings.json` (`hoocode-code-settings`).
//!
//! The input is the global file, then a NUL byte, then the project file (no NUL means
//! no project file). Must not panic; a broken file must surface as a settings error,
//! not a crash. Runs in memory, so no files are touched.

use std::sync::Arc;

use hoocode_code_settings::{
    InMemorySettingsStorage, SettingsManager, SettingsScope, SettingsStorage,
};

pub fn run(data: &[u8]) {
    let (global, project) = match data.iter().position(|&b| b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &data[data.len()..]),
    };
    let storage = InMemorySettingsStorage::new();
    seed(&storage, SettingsScope::Global, global);
    seed(&storage, SettingsScope::Project, project);

    let mut manager = SettingsManager::from_storage(Arc::new(storage));
    let _ = manager.settings();
    let _ = manager.default_model();
    let _ = manager.theme();
    let _ = manager.drain_errors();
    manager.reload();
    let _ = manager.settings();
}

fn seed(storage: &InMemorySettingsStorage, scope: SettingsScope, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes).into_owned();
    storage
        .with_lock(scope, &mut |_: Option<&str>| Ok(Some(text.clone())))
        .expect("in-memory write");
}
