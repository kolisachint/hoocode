//! `@file` autocomplete's file finder: an in-process walk with fd's rules, so
//! suggestions do not depend on an `fd` binary being installed (reliability
//! item 1.4).
//!
//! fd's rules, as the walk applies them: hidden entries are included, symlinks
//! are followed, `.gitignore`, `.ignore` and `.fdignore` are honoured (git
//! rules apply inside a git repository, as fd does by default), and `.git` is
//! skipped. Only files and directories are returned.
//!
//! Matching: a query without `/` matches the file name, a query with `/`
//! matches the path relative to the base. Matching is a substring test, and it
//! is case-insensitive unless the query has an uppercase letter (smart case).
//! The walk stops after `max_results` matches, so the first matches in sorted
//! walk order win, as fd's `--max-results` does.

use std::path::Path;

use ignore::WalkBuilder;

/// One match: a path relative to the search base, `/`-separated and without a
/// trailing slash, and whether it is a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundPath {
    pub path: String,
    pub is_directory: bool,
}

/// Find files and directories under `base` matching `query`, at most
/// `max_results` of them. An empty query matches everything.
pub fn find_paths(base: &Path, query: &str, max_results: usize) -> Vec<FoundPath> {
    let query = query.replace('\\', "/");
    let case_insensitive = !query.chars().any(char::is_uppercase);
    let needle = if case_insensitive {
        query.to_lowercase()
    } else {
        query.clone()
    };
    let match_full_path = query.contains('/');

    let mut walker = WalkBuilder::new(base);
    walker
        .hidden(false)
        .follow_links(true)
        .add_custom_ignore_filename(".fdignore")
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(|entry| entry.file_name() != ".git");

    let mut found = Vec::new();
    for entry in walker.build().flatten() {
        if entry.depth() == 0 {
            continue;
        }
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !(file_type.is_file() || file_type.is_dir()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(base) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");

        let candidate = if match_full_path {
            relative.clone()
        } else {
            entry.file_name().to_string_lossy().into_owned()
        };
        let candidate = if case_insensitive {
            candidate.to_lowercase()
        } else {
            candidate
        };
        if !candidate.contains(&needle) {
            continue;
        }

        found.push(FoundPath {
            path: relative,
            is_directory: file_type.is_dir(),
        });
        if found.len() >= max_results {
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("hoocode-file-finder-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Tree(dir)
        }

        fn file(&self, path: &str) {
            let full = self.0.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, "x").unwrap();
        }

        fn dir(&self, path: &str) {
            fs::create_dir_all(self.0.join(path)).unwrap();
        }

        fn paths(&self, query: &str) -> Vec<String> {
            let mut v: Vec<String> = find_paths(&self.0, query, 1000)
                .into_iter()
                .map(|f| f.path)
                .collect();
            v.sort();
            v
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn empty_query_lists_files_and_folders_without_the_base() {
        let t = Tree::new("empty");
        t.file("README.md");
        t.file("src/main.rs");
        assert_eq!(t.paths(""), ["README.md", "src", "src/main.rs"]);
    }

    #[test]
    fn reports_directories_as_directories() {
        let t = Tree::new("dirs");
        t.file("src/main.rs");
        let found = find_paths(&t.0, "src", 10);
        assert_eq!(
            found,
            [FoundPath {
                path: "src".into(),
                is_directory: true
            }]
        );
    }

    #[test]
    fn matches_name_case_insensitively_for_lowercase_queries() {
        let t = Tree::new("case");
        t.file("README.md");
        t.file("other.txt");
        assert_eq!(t.paths("re"), ["README.md"]);
    }

    #[test]
    fn smart_case_makes_an_uppercase_query_case_sensitive() {
        let t = Tree::new("smartcase");
        t.file("README.md");
        t.file("readme.txt");
        assert_eq!(t.paths("README"), ["README.md"]);
    }

    #[test]
    fn path_query_matches_the_relative_path_recursively() {
        let t = Tree::new("pathquery");
        t.file("packages/tui/src/autocomplete.ts");
        t.file("packages/ai/src/autocomplete.ts");
        assert_eq!(
            t.paths("tui/src/auto"),
            ["packages/tui/src/autocomplete.ts"]
        );
    }

    #[test]
    fn skips_git_but_includes_hidden_entries() {
        let t = Tree::new("hidden");
        t.file(".hoocode/config.json");
        t.file(".github/workflows/ci.yml");
        t.file(".git/config");
        let all = t.paths("");
        assert!(all.contains(&".hoocode".to_string()));
        assert!(all.contains(&".github/workflows/ci.yml".to_string()));
        assert!(!all.iter().any(|p| p == ".git" || p.starts_with(".git/")));
    }

    #[test]
    fn honours_gitignore_inside_a_repository() {
        let t = Tree::new("gitignore");
        t.dir(".git");
        t.file(".gitignore");
        fs::write(t.0.join(".gitignore"), "target/\n*.log\n").unwrap();
        t.file("target/debug/app");
        t.file("build.log");
        t.file("src/lib.rs");
        let all = t.paths("");
        assert!(all.contains(&"src/lib.rs".to_string()));
        assert!(!all.iter().any(|p| p.starts_with("target")));
        assert!(!all.contains(&"build.log".to_string()));
    }

    #[test]
    fn honours_fdignore() {
        let t = Tree::new("fdignore");
        t.file(".fdignore");
        fs::write(t.0.join(".fdignore"), "secret.txt\n").unwrap();
        t.file("secret.txt");
        t.file("public.txt");
        let all = t.paths("");
        assert!(all.contains(&"public.txt".to_string()));
        assert!(!all.contains(&"secret.txt".to_string()));
    }

    #[test]
    fn stops_after_max_results() {
        let t = Tree::new("limit");
        for i in 0..150 {
            t.file(&format!("file{i:03}.txt"));
        }
        assert_eq!(find_paths(&t.0, "file", 100).len(), 100);
        assert_eq!(find_paths(&t.0, "file", 5).len(), 5);
        assert_eq!(find_paths(&t.0, "file", 5)[0].path, "file000.txt");
    }

    #[cfg(unix)]
    #[test]
    fn follows_symlinked_directories_and_files() {
        let t = Tree::new("symlinks");
        t.file("real/some_file.txt");
        std::os::unix::fs::symlink("real", t.0.join("linked_dir")).unwrap();
        std::os::unix::fs::symlink("real/some_file.txt", t.0.join("link.txt")).unwrap();
        let all = t.paths("some");
        assert!(all.contains(&"real/some_file.txt".to_string()));
        assert!(all.contains(&"linked_dir/some_file.txt".to_string()));
        assert!(t.paths("link").contains(&"link.txt".to_string()));
        assert!(t.paths("linked").contains(&"linked_dir".to_string()));
    }
}
