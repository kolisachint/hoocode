//! `@file` completion through the app's in-process finder (reliability 1.4).
//! These are the cases that used to need `fd` on PATH; they now run anywhere,
//! because the finder never spawns a process.

use hoocode_code_tui_app::interactive_mode::at_file_finder;
use hoocode_tui_components::{
    AutocompleteProvider, AutocompleteSuggestions, CombinedAutocompleteProvider,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "hoocode-at-file-{}-{}",
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

/// Asks for suggestions the way the editor does, then waits out the background
/// walk (if this query started one) and asks again, as `poll_autocomplete` would.
fn suggest(
    p: &CombinedAutocompleteProvider,
    line: &str,
    col: usize,
    force: bool,
) -> Option<AutocompleteSuggestions> {
    let first = p.get_suggestions(&[line.to_string()], 0, col, force);
    if !p.file_walk_pending() {
        return first;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while p.file_walk_pending() {
        assert!(Instant::now() < deadline, "the @file walk never finished");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(p.take_ready());
    p.get_suggestions(&[line.to_string()], 0, col, force)
}

fn values(r: &Option<AutocompleteSuggestions>) -> Vec<String> {
    r.as_ref()
        .map(|r| r.items.iter().map(|i| i.value.clone()).collect())
        .unwrap_or_default()
}

struct Case {
    _root: TempDir,
    base: PathBuf,
    outside: PathBuf,
}

fn case() -> Case {
    let root = TempDir::new();
    let base = root.0.join("cwd");
    let outside = root.0.join("outside");
    fs::create_dir_all(&base).unwrap();
    fs::create_dir_all(&outside).unwrap();
    Case {
        _root: root,
        base,
        outside,
    }
}

impl Case {
    fn provider(&self) -> CombinedAutocompleteProvider {
        self.provider_at(&self.base)
    }
    fn provider_at(&self, base: &Path) -> CombinedAutocompleteProvider {
        CombinedAutocompleteProvider::new(vec![], base, Some(at_file_finder()))
    }
    fn at(&self, line: &str) -> Vec<String> {
        values(&suggest(
            &self.provider(),
            line,
            line.chars().count(),
            false,
        ))
    }
}

#[test]
fn returns_all_files_and_folders_for_empty_at_query() {
    let c = case();
    setup_folder(&c.base, &["src"], &[("README.md", "readme")]);
    let mut v = c.at("@");
    v.sort();
    assert_eq!(v, ["@README.md", "@src/"]);
}

#[test]
fn matches_file_with_extension_in_query() {
    let c = case();
    setup_folder(&c.base, &[], &[("file.txt", "content")]);
    assert!(c.at("@file.txt").contains(&"@file.txt".to_string()));
}

#[test]
fn filters_are_case_insensitive() {
    let c = case();
    setup_folder(&c.base, &["src"], &[("README.md", "readme")]);
    assert_eq!(c.at("@re"), ["@README.md"]);
}

#[test]
fn ranks_directories_before_files() {
    let c = case();
    setup_folder(&c.base, &["src"], &[("src.txt", "text")]);
    let v = c.at("@src");
    assert_eq!(v.first().map(String::as_str), Some("@src/"));
    assert!(v.contains(&"@src.txt".to_string()));
}

#[test]
fn returns_nested_file_paths() {
    let c = case();
    setup_folder(&c.base, &[], &[("src/index.ts", "export {};\n")]);
    assert!(c.at("@index").contains(&"@src/index.ts".to_string()));
}

#[test]
fn matches_deeply_nested_paths() {
    let c = case();
    setup_folder(
        &c.base,
        &[],
        &[
            ("packages/tui/src/autocomplete.ts", "export {};"),
            ("packages/ai/src/autocomplete.ts", "export {};"),
        ],
    );
    let v = c.at("@tui/src/auto");
    assert!(v.contains(&"@packages/tui/src/autocomplete.ts".to_string()));
    assert!(!v.contains(&"@packages/ai/src/autocomplete.ts".to_string()));
}

#[test]
fn matches_directory_in_middle_of_path() {
    let c = case();
    setup_folder(
        &c.base,
        &[],
        &[
            ("src/components/Button.tsx", "export {};"),
            ("src/utils/helpers.ts", "export {};"),
        ],
    );
    let v = c.at("@components/");
    assert!(v.contains(&"@src/components/Button.tsx".to_string()));
    assert!(!v.contains(&"@src/utils/helpers.ts".to_string()));
}

#[test]
fn scopes_fuzzy_search_to_relative_directories_recursively() {
    let c = case();
    setup_folder(
        &c.outside,
        &[],
        &[
            ("nested/alpha.ts", "export {};"),
            ("nested/deeper/also-alpha.ts", "export {};"),
            ("nested/deeper/zzz.ts", "export {};"),
        ],
    );
    let v = c.at("@../outside/a");
    assert!(v.contains(&"@../outside/nested/alpha.ts".to_string()));
    assert!(v.contains(&"@../outside/nested/deeper/also-alpha.ts".to_string()));
    assert!(!v.contains(&"@../outside/nested/deeper/zzz.ts".to_string()));
}

#[test]
fn quotes_paths_with_spaces() {
    let c = case();
    setup_folder(
        &c.base,
        &["my folder"],
        &[("my folder/test.txt", "content")],
    );
    assert!(c.at("@my").contains(&"@\"my folder/\"".to_string()));
}

#[test]
fn includes_hidden_paths_but_excludes_git() {
    let c = case();
    setup_folder(
        &c.base,
        &[".hoocode", ".github", ".git"],
        &[
            (".hoocode/config.json", "{}"),
            (".github/workflows/ci.yml", "name: ci"),
            (".git/config", "[core]"),
        ],
    );
    let v = c.at("@");
    assert!(v.contains(&"@.hoocode/".to_string()));
    assert!(v.contains(&"@.github/".to_string()));
    assert!(!v.iter().any(|x| x == "@.git" || x.starts_with("@.git/")));
}

#[test]
fn respects_gitignore_inside_a_repository() {
    let c = case();
    setup_folder(
        &c.base,
        &[".git", "target"],
        &[
            (".gitignore", "target/\n*.log\n"),
            ("target/debug/app", "bin"),
            ("build.log", "log"),
            ("src/lib.rs", "code"),
        ],
    );
    let v = c.at("@");
    assert!(v.contains(&"@src/lib.rs".to_string()));
    assert!(!v.iter().any(|x| x.starts_with("@target")));
    assert!(!v.contains(&"@build.log".to_string()));
}

#[test]
fn honours_the_result_limit() {
    let c = case();
    let files: Vec<String> = (0..150).map(|i| format!("file{i:03}.txt")).collect();
    let files: Vec<(&str, &str)> = files.iter().map(|f| (f.as_str(), "x")).collect();
    setup_folder(&c.base, &[], &files);
    let v = c.at("@file");
    assert_eq!(v.len(), 20, "shown suggestions are capped at 20");
}

#[cfg(unix)]
#[test]
fn follows_symlinked_directories() {
    let c = case();
    setup_folder(&c.base, &[], &[("dir/some_file.txt", "real")]);
    setup_folder(&c.outside, &[], &[("some_file.txt", "symlinked")]);
    std::os::unix::fs::symlink("../outside", c.base.join("symlinked_dir")).unwrap();
    let v = c.at("@some");
    assert!(v.contains(&"@dir/some_file.txt".to_string()));
    assert!(v.contains(&"@symlinked_dir/some_file.txt".to_string()));
}

#[cfg(unix)]
#[test]
fn returns_symlinked_directories_by_name() {
    let c = case();
    setup_folder(&c.outside, &[], &[("nested/file.txt", "symlinked")]);
    std::os::unix::fs::symlink("../outside", c.base.join("symlinked_dir")).unwrap();
    assert!(c.at("@symlinked").contains(&"@symlinked_dir/".to_string()));
}

#[cfg(unix)]
#[test]
fn returns_symlinked_files() {
    let c = case();
    setup_folder(&c.base, &[], &[("original.txt", "content")]);
    std::os::unix::fs::symlink("original.txt", c.base.join("link.txt")).unwrap();
    assert!(c.at("@link").contains(&"@link.txt".to_string()));
}

#[test]
fn suggestions_do_not_depend_on_the_query_appearing_in_cwd() {
    let c = case();
    let root = c.base.parent().unwrap();
    let normal = root.join("cwd-normal");
    let in_path = root.join("cwd-plan-repro");
    for base in [&normal, &in_path] {
        setup_folder(
            base,
            &["packages/coding-agent/examples/extensions/plan-mode"],
            &[
                (
                    "packages/coding-agent/examples/extensions/plan-mode/README.md",
                    "readme",
                ),
                ("packages/tui/docs/plan.md", "plan"),
            ],
        );
    }
    let norm = |base: &Path| {
        let r = suggest(&c.provider_at(base), "@plan", 5, false);
        let mut v: Vec<String> = r
            .map(|r| {
                r.items
                    .iter()
                    .map(|i| {
                        format!(
                            "{} :: {}",
                            i.label,
                            i.description.clone().unwrap_or_default()
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    let a = norm(&normal);
    assert_eq!(norm(&in_path), a);
    assert!(a.contains(
        &"plan-mode/ :: packages/coding-agent/examples/extensions/plan-mode".to_string()
    ));
    assert!(a.contains(&"plan.md :: packages/tui/docs/plan.md".to_string()));
}

#[test]
fn continues_inside_quoted_at_paths() {
    let c = case();
    setup_folder(
        &c.base,
        &[],
        &[
            ("my folder/test.txt", "content"),
            ("my folder/other.txt", "content"),
        ],
    );
    let line = "@\"my folder/\"";
    let r = suggest(&c.provider(), line, line.chars().count() - 1, false);
    assert!(r.is_some());
    let v = values(&r);
    assert!(v.contains(&"@\"my folder/test.txt\"".to_string()));
    assert!(v.contains(&"@\"my folder/other.txt\"".to_string()));
}

#[test]
fn applies_quoted_at_completion_without_duplicating_closing_quote() {
    let c = case();
    setup_folder(&c.base, &[], &[("my folder/test.txt", "content")]);
    let p = c.provider();
    let line = "@\"my folder/te\"";
    let col = line.chars().count() - 1;
    let r = suggest(&p, line, col, false).expect("suggestions");
    let item = r
        .items
        .iter()
        .find(|i| i.value == "@\"my folder/test.txt\"")
        .expect("test.txt");
    let applied = p.apply_completion(&[line.to_string()], 0, col, item, &r.prefix);
    assert_eq!(applied.lines[0], "@\"my folder/test.txt\" ");
}

#[test]
fn no_finder_means_no_at_suggestions() {
    let c = case();
    setup_folder(&c.base, &[], &[("file.txt", "content")]);
    let p = CombinedAutocompleteProvider::new(vec![], &c.base, None);
    assert!(suggest(&p, "@file", 5, false).is_none());
}
