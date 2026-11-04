//! The transient rows above the prompt: the warning, the retry notice,
//! extension statuses and startup progress, in that order, one row each,
//! cut to the width.

use crate::support::{lock, plain};
use hoocode_code_tui_app::footer_data::FooterDataProvider;
use hoocode_code_tui_app::startup_progress::{self, StartupProgress};
use hoocode_code_tui_app::transient_lines::TransientLines;

fn rows(t: &TransientLines, width: usize) -> Vec<String> {
    t.lines(width).iter().map(|l| plain(l)).collect()
}

#[test]
fn warning_retry_statuses_then_progress_in_order() {
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
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(lines[0], " Memory above the soft limit");
    assert_eq!(
        lines[1],
        " retrying · attempt 2 of 6 · 529 upstream overloaded"
    );
    assert_eq!(lines[2], " alpha");
    assert_eq!(
        lines[3],
        " Semantic search index unavailable: binary not found"
    );
    t.set_retry_notice(None);
    t.set_notice(None);
    assert_eq!(rows(&t, 120).len(), 2);
    startup_progress::clear();
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
