//! The defaults folder (`themes/defaults/`): the bundled copies written on
//! a version change, never read as themes, and a failed write is a warning.

use hoocode_code_tui_theme::{
    builtin_theme_source, picker_theme_list_in, refresh_theme_defaults, theme_warnings,
    theme_watch_files, write_theme_defaults, THEME_SCHEMA_JSON,
};

use crate::global_theme_lock;

const DEFAULTS: &str = "defaults";

#[test]
fn writes_the_builtins_and_schema_then_leaves_a_current_folder_alone() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("themes").join(DEFAULTS);
    assert_eq!(write_theme_defaults(&dir, "1.0.0"), Ok(true));
    assert_eq!(
        std::fs::read_to_string(dir.join("dark.json")).unwrap(),
        builtin_theme_source("dark").unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("light.json")).unwrap(),
        builtin_theme_source("light").unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("theme-schema.json")).unwrap(),
        THEME_SCHEMA_JSON
    );

    // A user's edit to a copy stays until the version changes.
    std::fs::write(dir.join("dark.json"), "{}").unwrap();
    assert_eq!(write_theme_defaults(&dir, "1.0.0"), Ok(false));
    assert_eq!(
        std::fs::read_to_string(dir.join("dark.json")).unwrap(),
        "{}"
    );
}

#[test]
fn a_new_version_refreshes_every_copy() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(DEFAULTS);
    write_theme_defaults(&dir, "1.0.0").unwrap();
    std::fs::write(dir.join("light.json"), "stale").unwrap();
    assert_eq!(write_theme_defaults(&dir, "1.1.0"), Ok(true));
    assert_eq!(
        std::fs::read_to_string(dir.join("light.json")).unwrap(),
        builtin_theme_source("light").unwrap()
    );
}

#[test]
fn a_missing_copy_is_written_again_at_the_next_start() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(DEFAULTS);
    write_theme_defaults(&dir, "1.0.0").unwrap();
    std::fs::remove_file(dir.join("theme-schema.json")).unwrap();
    assert_eq!(write_theme_defaults(&dir, "1.0.0"), Ok(true));
    assert!(dir.join("theme-schema.json").is_file());
}

#[test]
fn the_copies_are_never_listed_as_themes() {
    let root = tempfile::tempdir().unwrap();
    let themes = root.path().join("themes");
    write_theme_defaults(&themes.join(DEFAULTS), "1.0.0").unwrap();
    let names: Vec<String> = picker_theme_list_in(&themes)
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["auto", "dark", "light"]);
}

#[test]
fn a_folder_that_cannot_be_written_is_an_error_not_a_panic() {
    let root = tempfile::tempdir().unwrap();
    // `themes` is a file, so nothing can be written under it.
    let themes = root.path().join("themes");
    std::fs::write(&themes, "not a folder").unwrap();
    let error = write_theme_defaults(&themes.join(DEFAULTS), "1.0.0").unwrap_err();
    assert!(error.contains("themes"), "{error}");
    // Nothing is written outside the folder: the root holds only `themes`.
    let entries: Vec<_> = std::fs::read_dir(root.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn a_custom_theme_watches_the_bases_it_inherits_from() {
    let root = tempfile::tempdir().unwrap();
    let themes = root.path().join("themes");
    std::fs::create_dir_all(&themes).unwrap();
    std::fs::write(
        themes.join("mine.json"),
        r#"{"name":"mine","inherits":"base"}"#,
    )
    .unwrap();
    std::fs::write(
        themes.join("base.json"),
        r#"{"name":"base","inherits":"dark"}"#,
    )
    .unwrap();
    let files = theme_watch_files(&themes.join("mine.json"));
    assert_eq!(files, [themes.join("mine.json"), themes.join("base.json")]);

    // A built-in base is not a file to watch, and a cycle stops the walk.
    std::fs::write(themes.join("base.json"), r#"{"inherits":"mine"}"#).unwrap();
    let files = theme_watch_files(&themes.join("mine.json"));
    assert_eq!(files.len(), 2, "{files:?}");
}

#[test]
fn a_defaults_failure_is_a_warning_that_clears_once_the_folder_works() {
    let _g = global_theme_lock();
    let root = tempfile::tempdir().unwrap();
    std::env::set_var("HOOCODE_CODING_AGENT_DIR", root.path());
    // `themes` is a file, so the copies cannot be written.
    std::fs::write(root.path().join("themes"), "x").unwrap();
    assert!(!refresh_theme_defaults("1.0.0"));
    assert!(theme_warnings()
        .iter()
        .any(|w| w.starts_with("Theme defaults were not written")));

    std::fs::remove_file(root.path().join("themes")).unwrap();
    assert!(refresh_theme_defaults("1.0.0"));
    assert!(root.path().join("themes/defaults/dark.json").is_file());
    assert!(!theme_warnings()
        .iter()
        .any(|w| w.contains("Theme defaults")));
    std::env::remove_var("HOOCODE_CODING_AGENT_DIR");
}
