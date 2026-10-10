//! The footer: always two rows, and the fixed order in which parts drop when
//! the width is short. Ports of `footer-width.test.ts`, the footer half of
//! `startup-progress.test.ts`, and `subagent-footer.test.ts`.
//!
//! Retired 2026-10-09 (user decision: footer layout B). The pinned golden
//! (`fixtures/footer-gold.json`, hoocode's old layout) no longer describes the
//! footer, so its layout comparison is gone. The fixture is kept only as
//! history. The assertions below pin layout B instead.

use crate::support::{lock, plain};
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};
use hoocode_code_tui_app::footer::*;
use hoocode_code_tui_app::footer_data::FooterDataProvider;
use hoocode_code_tui_app::startup_progress::{self, StartupProgress};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

#[derive(Clone, Default)]
struct Stub {
    name: String,
    model: Option<FooterModel>,
    thinking: String,
    usage: (u64, u64, u64, u64, f64),
    ctx: Option<(u64, Option<f64>)>,
    cwd: String,
    reserve: u64,
    oauth: bool,
}

impl FooterSource for Stub {
    fn usage_totals(&self) -> (u64, u64, u64, u64, f64) {
        self.usage
    }
    fn context_usage(&self) -> Option<(u64, Option<f64>)> {
        self.ctx
    }
    fn model(&self) -> Option<FooterModel> {
        self.model.clone()
    }
    fn thinking_level(&self) -> String {
        self.thinking.clone()
    }
    fn cwd(&self) -> String {
        self.cwd.clone()
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn reserve_tokens(&self) -> u64 {
        self.reserve
    }
    fn is_using_oauth(&self) -> bool {
        self.oauth
    }
}

/// A provider whose cwd is a repo on `branch` (or no repo).
fn data_for(dir: &tempfile::TempDir, branch: Option<&str>) -> FooterDataProvider {
    if let Some(branch) = branch {
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(
            dir.path().join(".git/HEAD"),
            format!("ref: refs/heads/{branch}\n"),
        )
        .unwrap();
    }
    FooterDataProvider::new(dir.path())
}

fn footer(stub: Stub, providers: usize) -> FooterComponent {
    let data = FooterDataProvider::new(std::env::temp_dir().join("hoocode-footer-no-repo"));
    data.set_available_provider_count(providers);
    FooterComponent::new(Box::new(stub), data)
}

fn stub(name: &str) -> Stub {
    Stub {
        name: if name.is_empty() {
            "amber-harbor".into()
        } else {
            name.into()
        },
        model: Some(FooterModel {
            id: "test-model".into(),
            provider: "test".into(),
            context_window: 200_000,
            reasoning: false,
        }),
        thinking: "off".into(),
        ctx: Some((200_000, Some(12.3))),
        cwd: "/tmp/project".into(),
        reserve: 16_384,
        ..Default::default()
    }
}

/// The stub for the layout tests: a session, a model with effort, every kind
/// of token, and a context 34% full of a 200k window. The cwd is a path that
/// is not under `$HOME`, so it is shown as given.
fn full_stub() -> Stub {
    Stub {
        name: "refactor-auth".into(),
        model: Some(FooterModel {
            id: "sonnet-4-5".into(),
            provider: "test".into(),
            context_window: 200_000,
            reasoning: true,
        }),
        thinking: "high".into(),
        usage: (48_000, 3_100, 120_000, 0, 0.21),
        ctx: Some((200_000, Some(34.0))),
        cwd: "/work/hoocode".into(),
        reserve: 16_384,
        ..Default::default()
    }
}

/// `full_stub` in a repo on `main`.
fn full_footer(dir: &tempfile::TempDir) -> FooterComponent {
    FooterComponent::new(Box::new(full_stub()), data_for(dir, Some("main")))
}

/// One running subagent in the task store.
fn running_subagent() {
    let store = task_store();
    let id = store
        .create(
            "explore",
            CreateTaskOptions {
                source: Some(TaskSource::Subagent),
                ..Default::default()
            },
        )
        .id;
    store.update(
        id,
        TaskPatch {
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        },
    );
}

#[test]
fn keeps_all_lines_within_width_for_wide_session_names() {
    let _g = lock(None);
    let mut f = footer(stub(&"한글".repeat(30)), 1);
    let lines = f.render(93);
    assert_eq!(lines.len(), 2);
    for line in lines {
        assert!(visible_width(&line) <= 93);
    }
}

#[test]
fn hands_the_session_name_to_the_chip_once_the_box_is_wide_enough() {
    let _g = lock(None);
    let mut f = footer(stub("refactor-auth"), 1);
    f.set_session_chip_shown(true);
    assert!(!plain(&f.render(120)[0]).contains("refactor-auth"));
    assert!(plain(&f.render(40)[0]).contains("• refactor"));
}

#[test]
fn shows_the_auto_assigned_name_when_no_chip_is_set() {
    let _g = lock(None);
    assert!(plain(&footer(stub(""), 1).render(120)[0]).contains("amber-harbor"));
}

#[test]
fn keeps_stats_line_within_width_for_wide_model_and_provider_names() {
    let _g = lock(None);
    let mut s = stub("");
    s.model = Some(FooterModel {
        id: "模".repeat(30),
        provider: "공급자".into(),
        context_window: 200_000,
        reasoning: true,
    });
    s.thinking = "high".into();
    s.usage = (12_345, 6_789, 0, 0, 1.234);
    for line in footer(s, 2).render(60) {
        assert!(visible_width(&line) <= 60);
    }
}

#[test]
fn renders_without_task_related_content() {
    let _g = lock(Some("dark"));
    let out = footer(stub(""), 1).render(120).join("\n");
    assert!(!out.contains("[subagent:"));
    assert!(!out.contains("#1 "));
}

#[test]
fn footer_is_two_rows_in_every_state() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let data = data_for(&dir, Some("main"));
    data.set_extension_status("a", Some("alpha"));
    let mut f = FooterComponent::new(Box::new(full_stub()), data);
    f.set_notice(Some("UI stalled: no response for 2 s".into()));
    startup_progress::set(StartupProgress::Error {
        key: "e".into(),
        label: "Semantic search index unavailable".into(),
        message: "binary not found".into(),
    });
    running_subagent();
    for density in [FooterDensity::Full, FooterDensity::Line] {
        f.set_density(density);
        for w in 7..=130u16 {
            let rows = f.render(w);
            assert_eq!(rows.len(), 2, "{density:?} at {w}");
            for row in rows {
                assert!(visible_width(&row) <= w as usize, "{density:?} at {w}");
            }
        }
    }
    task_store().clear();
    startup_progress::clear();
}

#[test]
fn folder_name_is_visible_at_40_60_80_and_120_in_both_layouts() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let mut f = full_footer(&dir);
    for density in [FooterDensity::Full, FooterDensity::Line] {
        f.set_density(density);
        for w in [40u16, 60, 80, 120] {
            assert!(
                plain(&f.render(w)[0]).starts_with("hoocode"),
                "{density:?} at {w}"
            );
        }
    }
}

#[test]
fn folder_name_is_never_dropped_and_only_truncated_below_its_width() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let mut f = full_footer(&dir);
    for density in [FooterDensity::Full, FooterDensity::Line] {
        f.set_density(density);
        // The folder name is seven cells wide: whole from seven up.
        for w in 7..=130u16 {
            assert!(
                plain(&f.render(w)[0]).starts_with("hoocode"),
                "{density:?} at {w}"
            );
        }
        assert_eq!(plain(&f.render(7)[0]), "hoocode");
        let narrow = plain(&f.render(4)[0]);
        assert!(
            narrow.starts_with("hoo") && narrow.contains('…'),
            "{narrow:?}"
        );
        assert!(visible_width(&narrow) <= 4);
    }
}

#[test]
fn compact_keeps_the_folder_and_drops_the_path_session_and_tokens() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let mut f = full_footer(&dir);
    f.set_density(FooterDensity::Line);
    let rows: Vec<String> = f.render(120).iter().map(|r| plain(r)).collect();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].starts_with("hoocode  ⑂ main"));
    assert!(rows[0].ends_with("BUILD · ◍ peek"));
    assert!(!rows[0].contains("/work") && !rows[0].contains("refactor-auth"));
    assert!(rows[1].contains("sonnet-4-5") && rows[1].contains("34%"));
    assert!(!rows[1].contains('↑') && !rows[1].contains('$') && !rows[1].contains("of 200k"));
}

#[test]
fn row_one_matches_layout_b_at_120() {
    let _g = lock(Some("dark"));
    task_store().clear();
    let dir = tempfile::tempdir().unwrap();
    let mut f = full_footer(&dir);
    let line = plain(&f.render(120)[0]);
    assert_eq!(visible_width(&line), 120);
    assert!(line.starts_with("hoocode  ⑂ main • refactor-auth  /work/hoocode"));
    assert!(line.ends_with("BUILD · ◍ peek"));
}

#[test]
fn row_two_matches_layout_b_at_120_and_60() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let mut f = full_footer(&dir);
    assert_eq!(
        plain(&f.render(120)[1]),
        "sonnet-4-5 • high   ▰▰▰▱▱▱▱▱ 34% of 200k   ↑48k ↓3.1k R120k  $0.21"
    );
    // Cache goes first when the row is 60 wide.
    assert_eq!(
        plain(&f.render(60)[1]),
        "sonnet-4-5 • high   ▰▰▰▱▱▱▱▱ 34% of 200k   ↑48k ↓3.1k  $0.21"
    );
}

#[test]
fn row_one_drops_path_then_session_then_subagents_then_branch_then_dial_then_chip() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let mut f = full_footer(&dir);
    running_subagent();
    let mut shortened_seen = false;
    for w in 7..=130u16 {
        let line = plain(&f.render(w)[0]);
        // In drop order. An item can only be missing when every item before
        // it is missing too.
        let parts = [
            ("path", line.contains('/')),
            ("session", line.contains("refactor-auth")),
            ("subagents", line.contains("◇1")),
            ("branch", line.contains("main")),
            ("dial", line.contains('◍')),
            ("chip", line.contains("BUILD")),
        ];
        assert!(line.contains("hoocode"), "folder at {w}: {line:?}");
        shortened_seen |= line.contains("…/hoocode");
        for (i, (name_i, shown_i)) in parts.iter().enumerate() {
            for (name_j, shown_j) in &parts[i + 1..] {
                if !shown_j {
                    assert!(
                        !shown_i,
                        "{name_i} dropped before {name_j} at {w}: {line:?}"
                    );
                }
            }
        }
    }
    assert!(
        shortened_seen,
        "the path should shorten from the left before it drops"
    );
    let full = plain(&f.render(130)[0]);
    assert!(full.contains("/work/hoocode") && full.contains("◇1 running"));
    task_store().clear();
}

#[test]
fn row_two_drops_cache_then_arrows_then_cost_then_window_and_keeps_model_and_percent() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let mut f = full_footer(&dir);
    for w in 7..=130u16 {
        let line = plain(&f.render(w)[1]);
        let parts = [
            ("cache", line.contains("R120k")),
            ("arrows", line.contains("↑48k")),
            ("cost", line.contains("$0.21")),
            ("window", line.contains("of 200k")),
        ];
        for (i, (name_i, shown_i)) in parts.iter().enumerate() {
            for (name_j, shown_j) in &parts[i + 1..] {
                if !shown_j {
                    assert!(
                        !shown_i,
                        "{name_i} dropped before {name_j} at {w}: {line:?}"
                    );
                }
            }
        }
        assert!(visible_width(&line) <= w as usize);
        // Model and percent are the last thing left: all parts gone is 32 wide.
        if w >= 32 {
            assert!(
                line.contains("sonnet-4-5") && line.contains("34%"),
                "{w}: {line:?}"
            );
        }
    }
}

#[test]
fn compact_row_two_is_model_gauge_and_percent() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let mut f = full_footer(&dir);
    f.set_density(FooterDensity::Line);
    assert_eq!(plain(&f.render(120)[1]), "sonnet-4-5 • high   ▰▰▰▱▱▱▱▱ 34%");
}

#[test]
fn transient_messages_are_read_through_the_accessor_not_as_rows() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let data = data_for(&dir, Some("main"));
    data.set_extension_status("a", Some("alpha"));
    let mut f = FooterComponent::new(Box::new(full_stub()), data);
    f.set_notice(Some("Memory above the soft limit".into()));
    startup_progress::set(StartupProgress::Error {
        key: "e".into(),
        label: "Semantic search index unavailable".into(),
        message: "binary not found".into(),
    });

    let transient = f.transient_lines(120);
    assert_eq!(transient.len(), 3);
    assert!(plain(&transient[0]).contains("Memory above the soft limit"));
    assert!(plain(&transient[1]).contains("alpha"));
    assert!(plain(&transient[2]).contains("Semantic search index unavailable: binary not found"));

    let rows: Vec<String> = f.render(120).iter().map(|r| plain(r)).collect();
    assert_eq!(rows.len(), 2);
    assert!(!rows
        .iter()
        .any(|r| r.contains("alpha") || r.contains("Memory")));
    startup_progress::clear();
}

fn last_line(f: &mut FooterComponent, width: u16) -> String {
    plain(f.transient_lines(width as usize).last().unwrap())
}

#[test]
fn startup_progress_lines_are_transient_in_order_and_clamped() {
    let _g = lock(Some("dark"));
    let dir = tempfile::tempdir().unwrap();
    let mut f = full_footer(&dir);
    assert!(f.transient_lines(120).is_empty());
    startup_progress::set(StartupProgress::Download {
        key: "fd".into(),
        label: "fd".into(),
        received_bytes: 3_670_016,
        total_bytes: Some(8_388_608),
    });
    assert_eq!(f.transient_lines(120).len(), 1);
    let line = last_line(&mut f, 120);
    assert!(line.contains("3.5 MB / 8.0 MB") && line.contains("44%"));

    startup_progress::set(StartupProgress::Download {
        key: "rg".into(),
        label: "ripgrep".into(),
        received_bytes: 1_048_576,
        total_bytes: None,
    });
    let line = last_line(&mut f, 120);
    assert!(line.contains("ripgrep") && line.contains("1.0 MB") && !line.contains('%'));

    startup_progress::set(StartupProgress::Error {
        key: "semantic-index".into(),
        label: "Semantic search index unavailable".into(),
        message: "binary not found".into(),
    });
    let lines: Vec<String> = f.transient_lines(120).iter().map(|l| plain(l)).collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("fd") && lines[1].contains("ripgrep"));
    assert!(lines[2].contains("Semantic search index unavailable: binary not found"));

    startup_progress::set(StartupProgress::Download {
        key: "long".into(),
        label: "a-very-long-tool-name-".repeat(20),
        received_bytes: 5_000_000,
        total_bytes: Some(9_000_000),
    });
    assert!(last_line(&mut f, 60).chars().count() <= 60);
    assert_eq!(f.render(120).len(), 2);
    startup_progress::clear();
}
