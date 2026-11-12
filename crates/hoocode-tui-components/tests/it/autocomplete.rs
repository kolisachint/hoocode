//! Case-for-case port of the pin's `test/autocomplete.test.ts`. The `@file`
//! cases live in `hoocode-code-tui-app` (`tests/it/at_file_completion.rs`), where
//! the app's in-process finder is available. They need no `fd`.

use hoocode_tui_components::{
    AutocompleteProvider, AutocompleteSuggestions, CombinedAutocompleteProvider,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "hoocode-autocomplete-ts-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn setup_folder(base: &Path, dirs: &[&str], files: &[(&str, &str)]) {
    for d in dirs {
        fs::create_dir_all(base.join(d)).unwrap();
    }
    for (path, contents) in files {
        let full = base.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, contents).unwrap();
    }
}

fn suggest(
    p: &CombinedAutocompleteProvider,
    line: &str,
    col: usize,
    force: bool,
) -> Option<AutocompleteSuggestions> {
    p.get_suggestions(&[line.to_string()], 0, col, force)
}

fn values(r: &Option<AutocompleteSuggestions>) -> Vec<String> {
    r.as_ref()
        .map(|r| r.items.iter().map(|i| i.value.clone()).collect())
        .unwrap_or_default()
}

// --- extractPathPrefix

#[test]
fn extracts_root_slash_from_hey_slash_when_forced() {
    let p = CombinedAutocompleteProvider::new(vec![], "/tmp", None);
    let r = suggest(&p, "hey /", 5, true).expect("suggestions for /");
    assert_eq!(r.prefix, "/");
}

#[test]
fn extracts_slash_a_when_forced() {
    let p = CombinedAutocompleteProvider::new(vec![], "/tmp", None);
    // May be None when nothing under / starts with A; the prefix is the point.
    if let Some(r) = suggest(&p, "/A", 2, true) {
        assert_eq!(r.prefix, "/A");
    }
}

#[test]
fn does_not_trigger_for_slash_commands() {
    let p = CombinedAutocompleteProvider::new(vec![], "/tmp", None);
    assert!(suggest(&p, "/model", 6, true).is_none());
}

#[test]
fn triggers_for_absolute_paths_after_slash_command_argument() {
    let p = CombinedAutocompleteProvider::new(vec![], "/tmp", None);
    let r = suggest(&p, "/command /", 10, true).expect("suggestions");
    assert_eq!(r.prefix, "/");
}

// --- dot-slash path completion

#[test]
fn preserves_dot_slash_prefix_when_completing_paths() {
    let t = TempDir::new();
    setup_folder(
        &t.0,
        &[],
        &[("update.sh", "#!/bin/bash"), ("utils.ts", "export {};")],
    );
    let p = CombinedAutocompleteProvider::new(vec![], &t.0, None);
    let r = suggest(&p, "./up", 4, true);
    assert!(r.is_some());
    assert!(
        values(&r).contains(&"./update.sh".to_string()),
        "{:?}",
        values(&r)
    );
}

#[test]
fn preserves_dot_slash_prefix_for_directory_completions() {
    let t = TempDir::new();
    setup_folder(&t.0, &["src"], &[("src/index.ts", "export {};")]);
    let p = CombinedAutocompleteProvider::new(vec![], &t.0, None);
    let r = suggest(&p, "./sr", 4, true);
    assert!(r.is_some());
    assert!(
        values(&r).contains(&"./src/".to_string()),
        "{:?}",
        values(&r)
    );
}

// --- quoted path completion

#[test]
fn quotes_paths_with_spaces_for_direct_completion() {
    let t = TempDir::new();
    setup_folder(&t.0, &["my folder"], &[("my folder/test.txt", "content")]);
    let p = CombinedAutocompleteProvider::new(vec![], &t.0, None);
    let r = suggest(&p, "my", 2, true);
    assert!(values(&r).contains(&"\"my folder/\"".to_string()));
}

#[test]
fn continues_completion_inside_quoted_paths() {
    let t = TempDir::new();
    setup_folder(
        &t.0,
        &[],
        &[
            ("my folder/test.txt", "content"),
            ("my folder/other.txt", "content"),
        ],
    );
    let p = CombinedAutocompleteProvider::new(vec![], &t.0, None);
    let line = "\"my folder/\"";
    let r = suggest(&p, line, line.chars().count() - 1, true);
    let v = values(&r);
    assert!(v.contains(&"\"my folder/test.txt\"".to_string()));
    assert!(v.contains(&"\"my folder/other.txt\"".to_string()));
}

#[test]
fn applies_quoted_completion_without_duplicating_closing_quote() {
    let t = TempDir::new();
    setup_folder(&t.0, &[], &[("my folder/test.txt", "content")]);
    let p = CombinedAutocompleteProvider::new(vec![], &t.0, None);
    let line = "\"my folder/te\"";
    let col = line.chars().count() - 1;
    let r = suggest(&p, line, col, true).expect("suggestions");
    let item = r
        .items
        .iter()
        .find(|i| i.value == "\"my folder/test.txt\"")
        .expect("test.txt");
    let applied = p.apply_completion(&[line.to_string()], 0, col, item, &r.prefix);
    assert_eq!(applied.lines[0], "\"my folder/test.txt\"");
}
