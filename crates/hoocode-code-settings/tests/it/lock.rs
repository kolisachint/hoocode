//! Settings writes take hoocode-ts's proper-lockfile lock (`settings.json.lock`
//! directory), so the two tools exclude each other on a shared `~/.hoocode`.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use hoocode_code_settings::{Error, FileSettingsStorage, SettingsScope, SettingsStorage};

struct Dirs {
    _tmp: tempfile::TempDir,
    agent: PathBuf,
    storage: FileSettingsStorage,
}

impl Dirs {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let storage = FileSettingsStorage::new(&project, &agent);
        Self {
            _tmp: tmp,
            agent,
            storage,
        }
    }

    fn lock_dir(&self) -> PathBuf {
        self.agent.join("settings.json.lock")
    }
}

fn age(path: &Path, by: Duration) {
    let when = SystemTime::now() - by;
    File::open(path).unwrap().set_modified(when).unwrap();
}

#[test]
fn a_live_settings_lock_directory_blocks_the_write() {
    let d = Dirs::new();
    std::fs::create_dir(d.lock_dir()).unwrap();
    let result = d
        .storage
        .with_lock(SettingsScope::Global, &mut |_| Ok(Some("{}".into())));
    assert!(matches!(result, Err(Error::Locked(_))), "{result:?}");
    assert!(!d.agent.join("settings.json").exists());
    assert!(d.lock_dir().is_dir(), "the other tool's lock must stay");
}

#[test]
fn a_stale_settings_lock_directory_is_taken_over_and_removed_after_the_write() {
    let d = Dirs::new();
    std::fs::create_dir(d.lock_dir()).unwrap();
    age(&d.lock_dir(), Duration::from_secs(60));
    d.storage
        .with_lock(SettingsScope::Global, &mut |_| {
            Ok(Some(r#"{"theme":"dark"}"#.into()))
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(d.agent.join("settings.json")).unwrap(),
        r#"{"theme":"dark"}"#
    );
    assert!(!d.lock_dir().exists());
}

#[test]
fn our_settings_lock_is_a_directory_while_held() {
    let d = Dirs::new();
    // The lock is taken before the closure only when the file already exists.
    std::fs::write(d.agent.join("settings.json"), "{}").unwrap();
    let mut seen = None;
    d.storage
        .with_lock(SettingsScope::Global, &mut |_| {
            seen = Some((d.lock_dir().is_dir(), d.lock_dir().is_file()));
            Ok(Some("{}".into()))
        })
        .unwrap();
    assert_eq!(seen, Some((true, false)));
    assert!(!d.lock_dir().exists());
}

#[test]
fn a_stale_fs4_leftover_file_does_not_block_writes() {
    let d = Dirs::new();
    std::fs::write(d.lock_dir(), b"").unwrap();
    age(&d.lock_dir(), Duration::from_secs(60));
    d.storage
        .with_lock(SettingsScope::Global, &mut |_| Ok(Some("{}".into())))
        .unwrap();
    assert!(d.agent.join("settings.json").exists());
    assert!(!d.lock_dir().exists());
}
