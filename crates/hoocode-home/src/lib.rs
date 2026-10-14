//! The user's home directory and the `~` display of it (decision 28 in
//! `docs/design/decisions-2026-10-09.md`, X7 in `docs/design/tui-activity.md`).
//!
//! One rule everywhere: `$HOME` when set and non-empty, else the passwd entry.
//! When neither gives a home, the answer is `None`. It is never `/`: callers
//! leave the path alone instead of pretending the root is home.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The home directory: `$HOME` if set and non-empty, else the passwd entry.
pub fn home_dir() -> Option<PathBuf> {
    home_from(std::env::var_os("HOME"), dirs::home_dir)
}

/// The rule behind [`home_dir`], with the environment and passwd lookups injected.
fn home_from(home: Option<OsString>, passwd: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    home.filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(passwd)
}

/// Path separators: `/` everywhere, and `\` as well on Windows.
const SEPARATORS: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };

/// `path` with the home directory shown as `~`. Only a whole leading
/// component is replaced: under `/home/al`, `/home/alice` stays as it is.
pub fn shorten_path(path: &str) -> String {
    let home = home_dir();
    shorten_under(path, home.as_deref(), SEPARATORS)
}

/// The rule behind [`shorten_path`], with the home directory and the
/// separators injected.
fn shorten_under(path: &str, home: Option<&Path>, seps: &[char]) -> String {
    let Some(home) = home.and_then(Path::to_str) else {
        return path.to_string();
    };
    let home = home.trim_end_matches(seps);
    if home.is_empty() {
        return path.to_string();
    }
    match path.strip_prefix(home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with(seps) => format!("~{rest}"),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_set_wins_over_passwd() {
        let home = home_from(Some("/h/alice".into()), || Some("/etc/passwd-home".into()));
        assert_eq!(home, Some(PathBuf::from("/h/alice")));
    }

    #[test]
    fn home_empty_falls_back_to_passwd() {
        let home = home_from(Some("".into()), || Some("/etc/passwd-home".into()));
        assert_eq!(home, Some(PathBuf::from("/etc/passwd-home")));
    }

    #[test]
    fn home_unset_falls_back_to_passwd() {
        let home = home_from(None, || Some("/etc/passwd-home".into()));
        assert_eq!(home, Some(PathBuf::from("/etc/passwd-home")));
    }

    #[test]
    fn home_never_becomes_root() {
        assert_eq!(home_from(None, || None), None);
        assert_eq!(home_from(Some("".into()), || None), None);
    }

    const UNIX: &[char] = &['/'];
    const WINDOWS: &[char] = &['/', '\\'];

    #[test]
    fn shorten_replaces_home_on_a_component_boundary() {
        let home = Path::new("/home/alice");
        assert_eq!(shorten_under("/home/alice", Some(home), UNIX), "~");
        assert_eq!(
            shorten_under("/home/alice/x.rs", Some(home), UNIX),
            "~/x.rs"
        );
        assert_eq!(shorten_under("/home/alice/", Some(home), UNIX), "~/");
    }

    #[test]
    fn shorten_does_not_match_a_partial_component() {
        assert_eq!(
            shorten_under("/home/alice", Some(Path::new("/home/al")), UNIX),
            "/home/alice"
        );
        assert_eq!(
            shorten_under("/home/alice/x", Some(Path::new("/home/al")), UNIX),
            "/home/alice/x"
        );
        // A backslash is an ordinary name character on POSIX.
        assert_eq!(
            shorten_under("/home/al\\x", Some(Path::new("/home/al")), UNIX),
            "/home/al\\x"
        );
    }

    #[test]
    fn shorten_ignores_a_trailing_slash_on_home() {
        assert_eq!(
            shorten_under("/home/alice/x", Some(Path::new("/home/alice/")), UNIX),
            "~/x"
        );
    }

    #[test]
    fn shorten_accepts_backslashes_on_windows_style_paths() {
        let home = Path::new(r"C:\Users\al");
        assert_eq!(shorten_under(r"C:\Users\al", Some(home), WINDOWS), "~");
        assert_eq!(
            shorten_under(r"C:\Users\al\proj", Some(home), WINDOWS),
            r"~\proj"
        );
        assert_eq!(
            shorten_under(r"C:\Users\alice\proj", Some(home), WINDOWS),
            r"C:\Users\alice\proj"
        );
        assert_eq!(
            shorten_under(
                r"C:\Users\al\proj",
                Some(Path::new(r"C:\Users\al\")),
                WINDOWS
            ),
            r"~\proj"
        );
    }

    #[test]
    fn shorten_without_a_home_leaves_the_path_alone() {
        assert_eq!(shorten_under("/etc/x", None, UNIX), "/etc/x");
        assert_eq!(
            shorten_under("/etc/x", Some(Path::new("/")), UNIX),
            "/etc/x"
        );
        assert_eq!(shorten_under("/", None, UNIX), "/");
    }

    #[test]
    fn shorten_empty_path_stays_empty() {
        assert_eq!(shorten_under("", Some(Path::new("/home/alice")), UNIX), "");
    }
}
