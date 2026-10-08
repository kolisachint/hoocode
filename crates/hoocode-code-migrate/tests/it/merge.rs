//! The merge rules of naming-and-paths.md §3, on temp folders (no HOME, no env).

use std::fs;
use std::path::{Path, PathBuf};

use hoocode_code_migrate::{
    merge_dirs, Action, MergeResult, Options, Outcome, HOME_SKIP, MARKER_NAME, PROJECT_SKIP,
};
use serde_json::{json, Value};

const HOME_OPTS: Options = Options {
    skip: HOME_SKIP,
    dry_run: false,
    save_report: true,
};

struct Dirs {
    _tmp: tempfile::TempDir,
    old: PathBuf,
    new: PathBuf,
}

impl Dirs {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join(".cortexcode");
        let new = tmp.path().join(".hoocode");
        fs::create_dir_all(&old).unwrap();
        Self {
            _tmp: tmp,
            old,
            new,
        }
    }
}

fn put(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn json_at(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn merged(result: MergeResult) -> Outcome {
    match result {
        MergeResult::Merged(outcome) => outcome,
        other => panic!("expected a merge, got {other:?}"),
    }
}

fn action<'a>(outcome: &'a Outcome, path: &str) -> &'a Action {
    &outcome
        .entries
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| panic!("no entry for {path}: {:?}", outcome.entries))
        .action
}

/// The backup folder of a merge, found by its name prefix.
fn backup_file(dest: &Path, rel: &str) -> PathBuf {
    let dir = fs::read_dir(dest)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("backup-cortexcode-")
        })
        .expect("a backup folder");
    dir.join(rel)
}

#[test]
fn settings_merge_key_by_key_with_the_old_folder_winning_leaves() {
    let d = Dirs::new();
    put(
        &d.old.join("settings.json"),
        &json!({"theme": "light", "nested": {"b": 2}, "list": [1]}).to_string(),
    );
    put(
        &d.new.join("settings.json"),
        &json!({"theme": "dark", "nested": {"a": 1}, "list": [9, 9], "keep": "me"}).to_string(),
    );
    let outcome = merged(merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap());
    assert_eq!(
        json_at(&d.new.join("settings.json")),
        json!({"theme": "light", "nested": {"a": 1, "b": 2}, "list": [1], "keep": "me"})
    );
    assert!(matches!(
        action(&outcome, "settings.json"),
        Action::Replaced { .. }
    ));
    // The file as it was is kept in the backup folder.
    assert_eq!(
        json_at(&backup_file(&d.new, "settings.json")),
        json!({"theme": "dark", "nested": {"a": 1}, "list": [9, 9], "keep": "me"})
    );
}

#[test]
fn auth_merges_per_provider_and_keeps_the_lock_clean() {
    let d = Dirs::new();
    put(
        &d.old.join("auth.json"),
        &json!({"openai": {"key": "old-folder"}, "google": {"key": "g"}}).to_string(),
    );
    put(
        &d.new.join("auth.json"),
        &json!({"openai": {"key": "new-folder"}, "anthropic": {"key": "keep"}}).to_string(),
    );
    merged(merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap());
    assert_eq!(
        json_at(&d.new.join("auth.json")),
        json!({
            "openai": {"key": "old-folder"},
            "google": {"key": "g"},
            "anthropic": {"key": "keep"}
        })
    );
    assert!(!d.new.join("auth.json.lock").exists());
    assert!(!d.new.join(".merge.lock").exists());
}

#[test]
fn other_files_copy_and_a_different_same_named_file_is_backed_up() {
    let d = Dirs::new();
    put(&d.old.join("sessions/a.jsonl"), "A\n");
    put(&d.old.join("sessions/b.jsonl"), "B\n");
    put(&d.new.join("sessions/a.jsonl"), "old-a\n");
    let outcome = merged(merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap());
    assert_eq!(
        fs::read_to_string(d.new.join("sessions/a.jsonl")).unwrap(),
        "A\n"
    );
    assert_eq!(
        fs::read_to_string(d.new.join("sessions/b.jsonl")).unwrap(),
        "B\n"
    );
    assert_eq!(
        fs::read_to_string(backup_file(&d.new, "sessions/a.jsonl")).unwrap(),
        "old-a\n"
    );
    assert!(matches!(
        action(&outcome, "sessions/a.jsonl"),
        Action::Replaced { .. }
    ));
    assert!(matches!(
        action(&outcome, "sessions/b.jsonl"),
        Action::Added { backup: None, .. }
    ));
}

#[test]
fn identical_files_are_unchanged_and_not_backed_up() {
    let d = Dirs::new();
    put(&d.old.join("themes/t.json"), "{}");
    put(&d.new.join("themes/t.json"), "{}");
    let outcome = merged(merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap());
    assert_eq!(*action(&outcome, "themes/t.json"), Action::Unchanged);
    let backups = fs::read_dir(&d.new)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("backup-")
        })
        .count();
    assert_eq!(backups, 0);
}

#[test]
fn regenerated_folders_are_skipped_and_the_report_says_so() {
    let d = Dirs::new();
    put(&d.old.join("bin/fd"), "binary");
    put(&d.old.join("cache/x"), "cache");
    put(&d.old.join("embsearch/y"), "index");
    put(&d.old.join("cortex-debug.log"), "log");
    put(&d.old.join("prompts/p.md"), "prompt");
    let outcome = merged(merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap());
    for skipped in ["bin/", "cache/", "embsearch/", "cortex-debug.log"] {
        assert!(
            matches!(action(&outcome, skipped), Action::Skipped { .. }),
            "{skipped}"
        );
    }
    assert!(!d.new.join("bin").exists());
    assert!(!d.new.join("cache").exists());
    assert!(!d.new.join("cortex-debug.log").exists());
    assert_eq!(
        fs::read_to_string(d.new.join("prompts/p.md")).unwrap(),
        "prompt"
    );
    assert!(outcome.report().contains("skipped     bin/"));
}

#[test]
fn the_marker_stops_a_second_merge_and_the_lock_is_released() {
    let d = Dirs::new();
    put(&d.old.join("settings.json"), r#"{"a": 1}"#);
    merged(merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap());
    assert!(d.new.join(MARKER_NAME).exists());
    assert!(!d.new.join(".merge.lock").exists());
    put(&d.old.join("settings.json"), r#"{"a": 2}"#);
    match merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap() {
        MergeResult::AlreadyMerged { marker } => assert!(marker.ends_with(MARKER_NAME)),
        other => panic!("expected AlreadyMerged, got {other:?}"),
    }
    assert_eq!(json_at(&d.new.join("settings.json")), json!({"a": 1}));
}

#[test]
fn a_dry_run_names_the_work_and_writes_nothing() {
    let d = Dirs::new();
    put(&d.old.join("settings.json"), r#"{"a": 1}"#);
    put(&d.old.join("sessions/s.jsonl"), "S");
    let opts = Options {
        dry_run: true,
        ..HOME_OPTS
    };
    let outcome = merged(merge_dirs(&d.old, &d.new, &opts).unwrap());
    assert!(outcome.dry_run);
    assert_eq!(outcome.counts().added, 2);
    assert!(outcome.report().contains("nothing written"));
    assert!(!d.new.exists());
}

#[test]
fn a_missing_source_is_a_no_op() {
    let d = Dirs::new();
    let missing = d.old.parent().unwrap().join(".nope");
    assert!(matches!(
        merge_dirs(&missing, &d.new, &HOME_OPTS).unwrap(),
        MergeResult::NoSource
    ));
    assert!(!d.new.exists());
}

#[test]
fn a_partial_source_still_merges() {
    let d = Dirs::new();
    put(&d.old.join("models.json"), r#"{"providers": {}}"#);
    merged(merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap());
    assert_eq!(
        json_at(&d.new.join("models.json")),
        json!({"providers": {}})
    );
    assert!(!d.new.join("settings.json").exists());
}

#[test]
fn unreadable_json_on_either_side_is_left_alone() {
    let d = Dirs::new();
    put(&d.old.join("settings.json"), "{not json");
    put(&d.new.join("keybindings.json"), "{\"x\": 1}");
    put(&d.old.join("keybindings.json"), "[oops");
    put(&d.new.join("settings.json"), r#"{"theme": "dark"}"#);
    let outcome = merged(merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap());
    assert!(matches!(
        action(&outcome, "settings.json"),
        Action::Skipped { .. }
    ));
    assert!(matches!(
        action(&outcome, "keybindings.json"),
        Action::Skipped { .. }
    ));
    assert_eq!(
        json_at(&d.new.join("settings.json")),
        json!({"theme": "dark"})
    );
    assert_eq!(json_at(&d.new.join("keybindings.json")), json!({"x": 1}));
}

#[test]
fn project_merge_skips_dispatch_and_backs_up_inside_the_project() {
    let d = Dirs::new();
    put(&d.old.join("dispatch/t1/result.json"), "{}");
    put(&d.old.join("settings.json"), r#"{"theme": "light"}"#);
    put(&d.old.join("prompts/review.md"), "review");
    put(&d.new.join("settings.json"), r#"{"theme": "dark"}"#);
    let opts = Options {
        skip: PROJECT_SKIP,
        dry_run: false,
        save_report: false,
    };
    let outcome = merged(merge_dirs(&d.old, &d.new, &opts).unwrap());
    assert!(!d.new.join("dispatch").exists());
    assert_eq!(
        fs::read_to_string(d.new.join("prompts/review.md")).unwrap(),
        "review"
    );
    assert_eq!(
        json_at(&d.new.join("settings.json")),
        json!({"theme": "light"})
    );
    assert!(backup_file(&d.new, "settings.json").exists());
    assert!(matches!(
        action(&outcome, "dispatch/"),
        Action::Skipped { .. }
    ));
    // Project merges save no report file in the repo.
    let reports = fs::read_dir(&d.new)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("merge-report-")
        })
        .count();
    assert_eq!(reports, 0);
}

#[test]
fn the_home_merge_saves_its_report() {
    let d = Dirs::new();
    put(&d.old.join("settings.json"), r#"{"a": 1}"#);
    merged(merge_dirs(&d.old, &d.new, &HOME_OPTS).unwrap());
    let saved = fs::read_dir(&d.new)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("merge-report-") && n.ends_with(".txt"))
        .expect("a saved report");
    let text = fs::read_to_string(d.new.join(saved)).unwrap();
    assert!(text.contains("added"));
    assert!(text.contains("settings.json"));
}
