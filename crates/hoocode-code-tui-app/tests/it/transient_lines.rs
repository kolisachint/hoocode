//! The transient rows above the prompt: one of the auto-retry notice, the
//! leader, the history search or the warning (spec S24: one at a time, in
//! that priority order), then extension statuses, then startup progress.
//! Each row is cut to the width.

use crate::support::{lock, plain};
use hoocode_code_tui_app::footer_data::FooterDataProvider;
use hoocode_code_tui_app::startup_progress::{self, StartupProgress};
use hoocode_code_tui_app::transient_lines::{LeaderLine, TransientLines};
use hoocode_code_tui_keybindings::LeaderEntry;

fn rows(t: &TransientLines, width: usize) -> Vec<String> {
    t.lines(width).iter().map(|l| plain(l)).collect()
}

fn entry(key: &str, description: &str) -> LeaderEntry {
    LeaderEntry {
        key: key.into(),
        action: format!("action:{key}"),
        description: description.into(),
    }
}

/// S24: the retry notice outranks the warning, so both set shows only the
/// retry row; extension statuses and startup progress still follow. Clearing
/// the retry notice brings the warning back.
#[test]
fn retry_suppresses_warning_statuses_and_progress_follow() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    data.set_extension_status("a", Some("alpha"));
    let mut t = TransientLines::new(data.clone());
    assert_eq!(rows(&t, 120), [" alpha"]);
    t.set_notice(Some("Memory above the soft limit".into()));
    t.set_retry_notice(Some(
        "retrying · attempt 2 of 6 · 529 upstream\noverloaded".into(),
    ));
    startup_progress::set(StartupProgress::Error {
        key: "e".into(),
        label: "Semantic search index unavailable".into(),
        message: "binary not found".into(),
    });
    let lines = rows(&t, 120);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(
        lines[0],
        " retrying · attempt 2 of 6 · 529 upstream overloaded"
    );
    assert_eq!(lines[1], " alpha");
    assert_eq!(
        lines[2],
        " Semantic search index unavailable: binary not found"
    );
    t.set_retry_notice(None);
    let lines = rows(&t, 120);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0], " Memory above the soft limit");
    t.set_notice(None);
    assert_eq!(rows(&t, 120).len(), 2);
    startup_progress::clear();
    data.dispose();
}

/// S24: the leader outranks the warning, so both set shows only the leader
/// row; clearing the leader brings the warning back.
#[test]
fn leader_suppresses_warning() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    let mut t = TransientLines::new(data.clone());
    t.set_notice(Some("Memory above the soft limit".into()));
    assert_eq!(rows(&t, 120), [" Memory above the soft limit"]);
    t.set_leader(Some(LeaderLine {
        text: "Ctrl+X then m for model".into(),
        which_key: vec![],
    }));
    assert_eq!(rows(&t, 120), [" Ctrl+X then m for model"]);
    t.set_leader(None);
    assert_eq!(rows(&t, 120), [" Memory above the soft limit"]);
    t.set_notice(None);
    assert!(rows(&t, 120).is_empty());
    data.dispose();
}

/// S24: the history search outranks the warning, so both set shows only the
/// search's two rows; clearing the search brings the warning back.
#[test]
fn search_suppresses_warning() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    let mut t = TransientLines::new(data.clone());
    t.set_notice(Some("Memory above the soft limit".into()));
    t.set_history_search(Some((
        "history: foo".into(),
        "up/down pick, esc close".into(),
    )));
    assert_eq!(rows(&t, 120), [" history: foo", " up/down pick, esc close"]);
    t.set_history_search(None);
    assert_eq!(rows(&t, 120), [" Memory above the soft limit"]);
    t.set_notice(None);
    assert!(rows(&t, 120).is_empty());
    data.dispose();
}

/// S24: the retry notice outranks the leader, including its which-key popup
/// rows, so both set shows only the retry row; clearing the retry notice
/// brings the leader rows back.
#[test]
fn retry_suppresses_leader_and_which_key() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    let mut t = TransientLines::new(data.clone());
    t.set_leader(Some(LeaderLine {
        text: "Ctrl+X then m for model".into(),
        which_key: vec![entry("m", "model"), entry("t", "thinking")],
    }));
    let leader_rows = rows(&t, 120);
    assert!(leader_rows.len() >= 2, "{leader_rows:?}");
    assert_eq!(leader_rows[0], " Ctrl+X then m for model");
    t.set_retry_notice(Some("retrying · attempt 2 of 6".into()));
    let lines = rows(&t, 120);
    assert_eq!(lines, [" retrying · attempt 2 of 6"], "{lines:?}");
    t.set_retry_notice(None);
    assert_eq!(rows(&t, 120), leader_rows);
    t.set_leader(None);
    assert!(rows(&t, 120).is_empty());
    data.dispose();
}

#[test]
fn progress_rows_fit_the_width() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let data = FooterDataProvider::new(dir.path());
    let t = TransientLines::new(data.clone());
    assert!(t.lines(120).is_empty());
    startup_progress::set(StartupProgress::Download {
        key: "alpha".into(),
        label: "alpha".into(),
        received_bytes: 3_670_016,
        total_bytes: Some(8_388_608),
    });
    let line = rows(&t, 120).pop().unwrap();
    assert!(
        line.contains("3.5 MB / 8.0 MB") && line.contains("44%"),
        "{line}"
    );
    startup_progress::set(StartupProgress::Download {
        key: "long".into(),
        label: "a-very-long-tool-name-".repeat(20),
        received_bytes: 5_000_000,
        total_bytes: Some(9_000_000),
    });
    assert!(rows(&t, 60).iter().all(|l| l.chars().count() <= 60));
    startup_progress::clear();
    data.dispose();
}
