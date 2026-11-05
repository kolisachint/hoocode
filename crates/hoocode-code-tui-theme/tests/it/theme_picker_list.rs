//! The theme picker's list: `auto`, the built-ins, then the valid user
//! themes in the themes folder (top level only, never `defaults/`).

use std::path::Path;

use hoocode_code_tui_theme::{builtin_theme_source, picker_theme_list_in};
use serde_json::{json, Value};

fn dark_theme() -> Value {
    serde_json::from_str(builtin_theme_source("dark").unwrap()).unwrap()
}

fn write_theme(dir: &Path, file: &str, theme: &Value) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), serde_json::to_string_pretty(theme).unwrap()).unwrap();
}

fn names(list: &[hoocode_code_tui_theme::ThemeInfo]) -> Vec<String> {
    list.iter().map(|t| t.name.clone()).collect()
}

#[test]
fn the_picker_lists_auto_builtins_and_valid_user_themes_with_paths() {
    let root = tempfile::tempdir().unwrap();
    let themes = root.path().join("themes");

    let mut mine = dark_theme();
    mine["name"] = json!("mine");
    write_theme(&themes, "mine.json", &mine);
    // A file that does not load is left out of the list.
    std::fs::write(themes.join("broken.json"), "{ not json").unwrap();
    // A user file cannot take a built-in name.
    let mut shadow = dark_theme();
    shadow["name"] = json!("dark");
    write_theme(&themes, "dark.json", &shadow);
    // The defaults copies live in a subfolder the picker never reads.
    write_theme(&themes.join("defaults"), "solarized.json", &mine);

    let list = picker_theme_list_in(&themes);
    assert_eq!(
        names(&list),
        ["auto", "dark", "dusk", "high-contrast", "light", "mine"]
    );

    let auto = &list[0];
    assert!(auto.path.is_none());
    let dark = list.iter().find(|t| t.name == "dark").unwrap();
    assert!(dark.path.is_none(), "built-ins have no file");
    let mine_info = list.iter().find(|t| t.name == "mine").unwrap();
    assert_eq!(
        mine_info.path.as_deref(),
        Some(themes.join("mine.json").as_path())
    );
}

#[test]
fn a_missing_themes_folder_lists_only_auto_and_the_builtins() {
    let root = tempfile::tempdir().unwrap();
    let list = picker_theme_list_in(&root.path().join("no-such-dir"));
    assert_eq!(
        names(&list),
        ["auto", "dark", "dusk", "high-contrast", "light"]
    );
}

#[test]
fn user_themes_are_listed_by_name() {
    let root = tempfile::tempdir().unwrap();
    let themes = root.path().join("themes");
    for name in ["zebra", "Alpha"] {
        let mut theme = dark_theme();
        theme["name"] = json!(name);
        write_theme(&themes, &format!("{name}.json"), &theme);
    }
    let list = picker_theme_list_in(&themes);
    assert_eq!(
        names(&list),
        [
            "auto",
            "dark",
            "dusk",
            "high-contrast",
            "light",
            "Alpha",
            "zebra"
        ]
    );
}

#[test]
fn the_picker_lists_the_new_builtins() {
    let root = tempfile::tempdir().unwrap();
    let list = picker_theme_list_in(&root.path().join("no-such-dir"));
    for name in ["dusk", "high-contrast"] {
        let theme = list
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} is listed"));
        assert!(
            theme.path.is_none(),
            "{name} is built in, so it has no file"
        );
    }
}
