//! Path containment and placeholder expansion (Agent Plugins §4.1, §7.2.1, §9.2).

use std::path::{Component, Path, PathBuf};

/// The placeholder for the plugin root, as written in `mcp.json`.
pub const PLUGIN_ROOT_PLACEHOLDER: &str = "${PLUGIN_ROOT}";
/// The placeholder for the plugin's data directory, as written in `mcp.json`.
pub const PLUGIN_DATA_PLACEHOLDER: &str = "${PLUGIN_DATA}";
/// The environment variable that carries the plugin root to a server process.
pub const PLUGIN_ROOT_VAR: &str = "PLUGIN_ROOT";
/// The environment variable that carries the plugin data directory to a server process.
pub const PLUGIN_DATA_VAR: &str = "PLUGIN_DATA";

/// `path` with `.` and `..` resolved, without touching the filesystem.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Whether `candidate` lies inside `root`. `root` is a directory the caller has already
/// canonicalized. The check is lexical first; then, through the deepest part of `candidate`
/// that exists, symlinks are followed, so a link out of the root is caught too.
pub fn within(root: &Path, candidate: &Path) -> bool {
    let root = normalize(root);
    let candidate = normalize(candidate);
    if !candidate.starts_with(&root) {
        return false;
    }
    let Ok(real_root) = root.canonicalize() else {
        return false;
    };
    let mut probe = candidate.as_path();
    loop {
        match probe.canonicalize() {
            Ok(real) => return real.starts_with(&real_root),
            Err(_) => match probe.parent() {
                Some(parent) => probe = parent,
                None => return false,
            },
        }
    }
}

/// A plugin-relative path (`./x`) resolved against `root`. `None` when the text does not start
/// with `./` or the result is outside `root`.
pub fn plugin_relative(root: &Path, text: &str) -> Option<PathBuf> {
    let rel = text.strip_prefix("./")?;
    let candidate = root.join(rel);
    within(root, &candidate).then(|| normalize(&candidate))
}

/// The placeholder that starts `text`, if any, with its value and the rest of the text.
/// The placeholder must be the whole text or be followed by `/`.
fn leading_placeholder<'a>(text: &'a str, placeholder: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(placeholder)?;
    if rest.is_empty() || rest.starts_with('/') {
        Some(rest)
    } else {
        None
    }
}

/// Resolve a `cwd` value (§7.2.1). `root` and `data` are the plugin root and data directory,
/// canonical. `None` when the form is not allowed or the result is outside its base directory.
pub fn resolve_cwd(text: &str, root: &Path, data: &Path) -> Option<PathBuf> {
    if text.starts_with("./") {
        return plugin_relative(root, text);
    }
    if let Some(rest) = leading_placeholder(text, PLUGIN_ROOT_PLACEHOLDER) {
        let candidate = join_placeholder_rest(root, rest);
        return within(root, &candidate).then(|| normalize(&candidate));
    }
    if let Some(rest) = leading_placeholder(text, PLUGIN_DATA_PLACEHOLDER) {
        let candidate = join_placeholder_rest(data, rest);
        return within(data, &candidate).then(|| normalize(&candidate));
    }
    None
}

/// `base` joined with the text after a placeholder (`""` or `"/sub/dir"`).
fn join_placeholder_rest(base: &Path, rest: &str) -> PathBuf {
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    if rest.is_empty() {
        base.to_path_buf()
    } else {
        base.join(rest)
    }
}

/// Replace every exact `${PLUGIN_ROOT}` and `${PLUGIN_DATA}` in one pass (§9.2). Text that a
/// replacement introduces is not scanned again. Other `${...}` text stays as written.
pub fn expand(text: &str, root: &str, data: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((at, token, value)) = next_placeholder(rest, root, data) {
        out.push_str(&rest[..at]);
        out.push_str(value);
        rest = &rest[at + token.len()..];
    }
    out.push_str(rest);
    out
}

fn next_placeholder<'a>(
    text: &str,
    root: &'a str,
    data: &'a str,
) -> Option<(usize, &'static str, &'a str)> {
    let root_hit = text
        .find(PLUGIN_ROOT_PLACEHOLDER)
        .map(|at| (at, PLUGIN_ROOT_PLACEHOLDER, root));
    let data_hit = text
        .find(PLUGIN_DATA_PLACEHOLDER)
        .map(|at| (at, PLUGIN_DATA_PLACEHOLDER, data));
    match (root_hit, data_hit) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_resolves_dots_without_touching_the_disk() {
        assert_eq!(
            normalize(Path::new("/a/b/./c/../d")),
            PathBuf::from("/a/b/d")
        );
    }

    #[test]
    fn within_is_lexical_for_paths_that_do_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(within(&root, &root.join("a/b/missing")));
        assert!(!within(&root, &root.join("../escape")));
        assert!(!within(&root, &root.join("a/../../escape")));
    }

    #[cfg(unix)]
    #[test]
    fn within_follows_symlinks_that_leave_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let root = root.canonicalize().unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert!(!within(&root, &root.join("link")));
        assert!(!within(&root, &root.join("link/new-file")));
        std::fs::create_dir_all(root.join("real")).unwrap();
        assert!(within(&root, &root.join("real")));
    }

    #[test]
    fn expand_is_single_pass_and_leaves_other_placeholders() {
        assert_eq!(
            expand("${PLUGIN_ROOT}/x ${PLUGIN_DATA}/y ${HOME}", "/r", "/d"),
            "/r/x /d/y ${HOME}"
        );
        // A replacement that looks like a placeholder is not expanded again.
        assert_eq!(
            expand("${PLUGIN_ROOT}", "${PLUGIN_DATA}", "/d"),
            "${PLUGIN_DATA}"
        );
        assert_eq!(expand("${PLUGIN_ROOT_X}", "/r", "/d"), "${PLUGIN_ROOT_X}");
    }

    #[test]
    fn cwd_forms_are_limited() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let data = dir.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let root = root.canonicalize().unwrap();
        let data = data.canonicalize().unwrap();
        let ok = |text: &str| resolve_cwd(text, &root, &data).is_some();
        assert!(ok("./sub"));
        assert!(ok("${PLUGIN_ROOT}"));
        assert!(ok("${PLUGIN_ROOT}/sub"));
        assert!(ok("${PLUGIN_DATA}"));
        assert!(ok("${PLUGIN_DATA}/cache"));
        assert!(!ok("../x"));
        assert!(!ok("./../x"));
        assert!(!ok("data"));
        assert!(!ok("/abs/path"));
        assert!(!ok("${PLUGIN_ROOT}/../x"));
        assert!(!ok("${PLUGIN_ROOT}x"));
        assert!(!ok("${PLUGIN_DATA}/../../x"));
        assert!(!ok("${HOME}"));
    }
}
