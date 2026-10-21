//! Direct Mission Control radar row checks (spec r5).
//!
//! Direct Mission Control radar row checks (spec r5, r7, r9). The clearance
//! hold is set by the clearance card (`set_waiting_for_clearance`), so the
//! chain shows the `waiting_row` for a held call.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{assistant, lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_tui_widgets::radar::{
    agent_live_row, chain_rows, hit_row, live_row, mission_rows, prose_rows, satellite_row,
    sub_hit_row, sub_live_row, sub_waiting_row, think_rows, waiting_row, FlightMark, FlightStats,
    HIT_INDENT, STEP_INDENT,
};
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::tool_chain_summary::ChainEntry;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView::Radar;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_code_tui_widgets::{AssistantMessageComponent, ThinkingDisplay};
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::Component;
use serde_json::json;

#[test]
fn waiting_head_row_holds_for_clearance() {
    let palette = Palette::default();
    let row = strip(&waiting_row(1, "Shell", "sleep 6", true, &palette, 80));
    assert!(row.contains("01"), "{row:?}");
    assert!(row.contains("◉"), "{row:?}");
    assert!(row.contains("Shell"), "{row:?}");
    assert!(row.contains("sleep 6"), "{row:?}");
    assert!(row.contains("holding for clearance"), "{row:?}");
}

#[test]
fn waiting_tail_row_reads_as_next() {
    let palette = Palette::default();
    let row = strip(&waiting_row(2, "Shell", "echo done", false, &palette, 80));
    assert!(row.contains("02"), "{row:?}");
    assert!(row.contains("○"), "{row:?}");
    assert!(!row.contains("◉"), "{row:?}");
    assert!(row.trim_end().ends_with("next"), "{row:?}");
}

fn landed(steps: usize, hits: usize) -> FlightStats {
    FlightStats {
        mark: FlightMark::Landed,
        steps,
        hits,
        landed_time: Some("0.8s".to_string()),
        started: None,
        cost: Some("$0.003".to_string()),
        chain: Vec::new(),
    }
}

#[test]
fn live_row_fills_its_bar_by_elapsed_share() {
    // 3s of ~6s is half of the eight cells.
    let palette = Palette::default();
    let row = strip(&live_row(
        1,
        "Shell",
        "sleep 6; echo done",
        3,
        Some(6),
        &palette,
        80,
    ));
    assert_eq!(
        row.trim_end(),
        "        ◐ 01 Shell  sleep 6; echo done  ████░░░░ 3s of ~6s"
    );
}

#[test]
fn live_row_without_an_estimate_reads_elapsed_only() {
    let palette = Palette::default();
    let row = strip(&live_row(1, "Shell", "sleep 6", 3, None, &palette, 80));
    assert_eq!(row.trim_end(), "        ◐ 01 Shell  sleep 6  3s");
}

#[test]
fn hit_row_indents_twelve_and_keeps_the_chip_inline() {
    let palette = Palette::default();
    let row = strip(&hit_row(
        4,
        "Shell",
        "echo boom; exit 3",
        "boom",
        &palette,
        80,
    ));
    assert_eq!(
        row.trim_end(),
        "            ⊗ 04 Shell  echo boom; exit 3  → boom  HIT"
    );
    assert!(row.trim_end().chars().count() <= 79, "{row:?}");
}

#[test]
fn two_digit_hits_grow_leftward_and_keep_the_columns() {
    let palette = Palette::default();
    let row = strip(&mission_rows(1, "wrap", &landed(12, 12), &palette, 80)[0]);
    assert!(
        row.trim_end().ends_with("12 steps 12✕   0.8s  $0.003"),
        "{row:?}"
    );
    assert_eq!(row.chars().count(), 79, "{row:?}");
}

#[test]
fn stepless_mission_leaves_the_steps_columns_blank() {
    let palette = Palette::default();
    let stats = FlightStats {
        landed_time: None,
        cost: Some("$0.000".to_string()),
        ..landed(0, 0)
    };
    let row = strip(&mission_rows(3, "just talking", &stats, &palette, 80)[0]);
    assert_eq!(
        row.trim_end(),
        " M3 ●  just talking                                                      $0.000"
    );
}

fn plain(text: &str) -> Vec<String> {
    vec![text.to_string()]
}

#[test]
fn prose_indents_eight_under_a_mission_and_twelve_under_a_hit() {
    let palette = Palette::default();
    let ink = palette.ink();
    let under_mission = strip(&prose_rows(&plain("It wraps text."), STEP_INDENT, ink, 80)[0]);
    assert_eq!(under_mission.trim_end(), "        It wraps text.");
    let under_hit = strip(&prose_rows(&plain("It wraps text."), HIT_INDENT, ink, 80)[0]);
    assert_eq!(under_hit.trim_end(), "            It wraps text.");
}

#[test]
fn prose_wraps_before_the_last_column_and_never_cuts() {
    let palette = Palette::default();
    let text = "one long line of prose that keeps going past the edge";
    let rows = prose_rows(&plain(text), STEP_INDENT, palette.ink(), 24);
    assert!(rows.len() >= 3, "{rows:?}");
    let mut words = Vec::new();
    for row in &rows {
        let row = strip(row);
        assert!(!row.contains('…'), "{row:?}");
        assert!(row.starts_with(&" ".repeat(STEP_INDENT)), "{row:?}");
        assert!(row.chars().count() <= 23, "{row:?}");
        words.extend(row.split_whitespace().map(str::to_string));
    }
    assert_eq!(words.join(" "), text);
}

#[test]
fn a_long_prompt_wraps_under_its_first_word_and_the_stats_stay_on_the_first_row() {
    let palette = Palette::default();
    let prompt = "fix the overflow bug in the wrap helper so long mission prompts never get cut\nthen run the whole suite";
    let rows: Vec<String> = mission_rows(1, prompt, &landed(3, 0), &palette, 60)
        .iter()
        .map(|row| strip(row))
        .collect();
    assert!(rows.len() >= 3, "{rows:#?}");
    // Stats on the first row only.
    assert!(
        rows[0].trim_end().ends_with("3 steps  0✕   0.8s  $0.003"),
        "{rows:#?}"
    );
    assert!(rows[0].starts_with(" M1 ●  fix the"), "{rows:#?}");
    for row in &rows[1..] {
        assert!(!row.contains("steps"), "{rows:#?}");
        // Hung under the prompt text, which starts at column 7.
        assert!(row.starts_with("       "), "{row:?}");
        assert!(!row.starts_with("        "), "{row:?}");
    }
    for row in &rows {
        assert!(!row.contains('…'), "{rows:#?}");
        assert!(row.chars().count() <= 59, "{row:?}");
    }
    let flat = rows
        .iter()
        .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(" ");
    for word in prompt.split_whitespace() {
        assert!(flat.contains(word), "{word}: {flat}");
    }
    assert!(
        rows.last().unwrap().contains("the whole suite"),
        "{rows:#?}"
    );
}

fn entry(tool: &str, subject: &str, is_error: bool, is_partial: bool) -> ChainEntry {
    ChainEntry {
        tool: tool.to_string(),
        subject: subject.to_string(),
        is_error,
        is_partial,
        output_lines: 2,
        outcome: None,
    }
}

#[test]
fn the_chain_row_shows_the_shape_while_flying_and_the_summary_once_landed() {
    let palette = Palette::default();
    let mut stats = FlightStats {
        mark: FlightMark::InFlight,
        steps: 4,
        chain: vec![
            entry("Read", "a.rs", false, false),
            entry("Read", "b.rs", false, false),
            entry("Edit", "a.rs", false, false),
            entry("Shell", "cargo test", true, false),
            entry("Shell", "cargo test", false, true),
        ],
        ..Default::default()
    };
    let live = chain_rows(&stats, &palette, 80);
    assert_eq!(live.len(), 1, "{live:?}");
    assert_eq!(
        strip(&live[0]).trim_end(),
        "        ◐ Read ×2 › Edit › Shell✗ › Shell …"
    );

    stats.mark = FlightMark::Landed;
    stats.chain = vec![
        entry("Edit", "a.rs", false, false),
        entry("Edit", "b.rs", false, false),
        entry("Edit", "c.rs", false, false),
        entry("Shell", "cargo fmt", false, false),
        entry("Shell", "cargo test", false, false),
    ];
    let settled = chain_rows(&stats, &palette, 80);
    assert_eq!(settled.len(), 1, "{settled:?}");
    let text = strip(&settled[0]);
    assert!(
        text.starts_with("        ● Edited 3 files · ran 2 commands"),
        "{text:?}"
    );
    assert!(text.trim_end().ends_with("5 calls · 10 lines"), "{text:?}");
    // No calls, no row.
    stats.chain.clear();
    assert!(chain_rows(&stats, &palette, 80).is_empty());
}

#[test]
fn a_wide_chain_row_wraps_instead_of_cutting() {
    let palette = Palette::default();
    let tools = ["Read", "Edit", "Shell", "CodeSearch", "WebFetch", "Write"];
    let stats = FlightStats {
        mark: FlightMark::InFlight,
        chain: tools
            .iter()
            .map(|tool| entry(tool, "x", false, false))
            .collect(),
        ..Default::default()
    };
    let rows: Vec<String> = chain_rows(&stats, &palette, 30)
        .iter()
        .map(|row| strip(row))
        .collect();
    assert!(rows.len() >= 2, "{rows:#?}");
    for row in &rows {
        assert!(row.chars().count() <= 29, "{row:?}");
        assert!(!row.contains("… "), "{row:?}");
    }
    let flat = rows.join(" ");
    for tool in tools {
        assert!(flat.contains(tool), "{tool}: {rows:#?}");
    }
    assert!(rows.last().unwrap().trim_end().ends_with('…'), "{rows:#?}");
}

#[test]
fn thinking_wraps_under_its_gist() {
    let palette = Palette::default();
    let gist = "The helper is 13 lines and wraps at the word boundary every time";
    let rows: Vec<String> = think_rows(3, Some(gist), &palette, 40)
        .iter()
        .map(|row| strip(row))
        .collect();
    assert!(rows.len() >= 2, "{rows:#?}");
    assert!(
        rows[0].starts_with("        ✻ thought 3s · The helper"),
        "{rows:#?}"
    );
    let hang = "        ✻ thought 3s · ".chars().count();
    for row in &rows[1..] {
        assert!(row.starts_with(&" ".repeat(hang)), "{row:?}");
    }
    for row in &rows {
        assert!(!row.contains('…'), "{row:?}");
    }
    let flat = rows
        .iter()
        .map(|row| row.trim_start_matches("        ✻ thought 3s · "))
        .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(flat, gist);
}

#[test]
fn prose_moves_to_the_hit_text_when_the_chain_above_turns_into_a_hit() {
    let _g = lock();
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let options = ToolExecutionOptions {
        view: Radar,
        ..Default::default()
    };
    // The chain above is still running when the prose message appears.
    let call = Rc::new(RefCell::new(ToolExecutionComponent::new(
        "Shell",
        "live-1",
        json!({"command": "bun test"}),
        options,
        None,
        &cwd,
    )));
    let mut chain = ToolChainComponent::new(Radar);
    chain.add(call.clone());
    let chain = Rc::new(RefCell::new(chain));

    let message = assistant(vec![Content::text("It wraps text.")]);
    let mut prose = AssistantMessageComponent::new(Some(&message), ThinkingDisplay::Omit);
    prose.set_radar_above(Some(Rc::downgrade(&chain)));
    // The transcript shows the prose once the mission has ended.
    prose.set_radar_prose_shown(true);
    let before = strip(&prose.render(40).join("\n"));
    assert!(before.starts_with("        It wraps text."), "{before:?}");

    // The running call fails: the row above is now a hit.
    call.borrow_mut().update_result(
        ToolResult {
            content: vec![Content::text("ASSERTION FAILED")],
            details: json!({}),
            is_error: true,
        },
        false,
    );
    let after = strip(&prose.render(40).join("\n"));
    assert!(after.starts_with("            It wraps text."), "{after:?}");
}

#[test]
fn narrow_rows_keep_the_last_column_blank_and_never_overflow() {
    let palette = Palette::default();
    let stats = landed(12, 12);
    let live = FlightStats {
        mark: FlightMark::InFlight,
        steps: 3,
        hits: 1,
        landed_time: None,
        started: None,
        cost: None,
        chain: vec![entry("Read", "a.rs", false, false)],
    };
    for width in [1usize, 10, 12, 20, 24, 40] {
        let mut rows = mission_rows(12, "a long prompt that goes on", &stats, &palette, width);
        rows.extend(mission_rows(1, "wrap", &live, &palette, width));
        rows.extend(chain_rows(&live, &palette, width));
        rows.extend(chain_rows(&stats, &palette, width));
        rows.extend([
            hit_row(4, "Shell", "echo boom; exit 3", "boom", &palette, width),
            live_row(
                1,
                "Shell",
                "sleep 6; echo done",
                3,
                Some(6),
                &palette,
                width,
            ),
            waiting_row(1, "Shell", "sleep 6", true, &palette, width),
            agent_live_row(
                1,
                "general-purpose",
                "Compute a product",
                2,
                &palette,
                width,
            ),
            satellite_row(1, "Shell", "sleep 60", "job-1", 4, &palette, width),
        ]);
        rows.extend(think_rows(
            3,
            Some("The helper is 13 lines"),
            &palette,
            width,
        ));
        rows.extend(prose_rows(
            &plain("It wraps text and goes on"),
            HIT_INDENT,
            palette.ink(),
            width,
        ));
        for row in rows {
            let text = strip(&row);
            assert!(
                text.chars().count() < width.max(1),
                "width {width}: {text:?}"
            );
        }
    }
}

#[test]
fn sub_step_rows_read_under_their_agent_at_the_step_indent() {
    let palette = Palette::default();
    let live = strip(&sub_live_row(1, 1, "Read", "src/lib.rs", 2, &palette, 80));
    assert!(live.starts_with(&" ".repeat(STEP_INDENT)), "{live:?}");
    assert!(live.contains("◐ 01.1 Read  src/lib.rs"), "{live:?}");
    let hit = strip(&sub_hit_row(
        1,
        2,
        "Read",
        "missing.rs",
        "no such file",
        &palette,
        80,
    ));
    assert!(hit.starts_with(&" ".repeat(STEP_INDENT)), "{hit:?}");
    assert!(hit.contains("⊗ 01.2 Read  missing.rs"), "{hit:?}");
    assert!(hit.contains("HIT"), "{hit:?}");
}

#[test]
fn a_sub_step_held_for_clearance_reads_as_a_hold_under_its_agent() {
    let palette = Palette::default();
    let held = strip(&sub_waiting_row(1, 1, "Shell", "rm -rf x", &palette, 80));
    assert!(held.starts_with(&" ".repeat(STEP_INDENT)), "{held:?}");
    assert!(held.contains("◉ 01.1 Shell  rm -rf x"), "{held:?}");
    assert!(held.contains("holding for clearance"), "{held:?}");
}

#[test]
fn a_call_held_for_clearance_reads_the_waiting_row() {
    // The permission gate parks a call that already started: the chain shows
    // the `◉ holding for clearance` row, not a live `◐` row.
    let _g = lock();
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let options = ToolExecutionOptions {
        view: Radar,
        ..Default::default()
    };
    let call = Rc::new(RefCell::new(ToolExecutionComponent::new(
        "Shell",
        "held-1",
        json!({"command": "ls"}),
        options,
        None,
        &cwd,
    )));
    call.borrow_mut().mark_execution_started();
    assert!(!call.borrow().is_waiting_for_clearance());
    assert!(call.borrow().is_held_by("Shell", "$ ls"));
    assert!(!call.borrow().is_held_by("Shell", "$ rm -rf target/"));
    call.borrow_mut().set_waiting_for_clearance(true);
    assert!(call.borrow().is_waiting_for_clearance());

    let mut chain = ToolChainComponent::new(Radar);
    chain.add(call.clone());
    let rows: Vec<String> = chain.render(80).iter().map(|row| strip(row)).collect();
    let held = rows
        .iter()
        .find(|row| row.contains("01 Shell"))
        .expect("a step row");
    assert!(held.contains("◉"), "{held:?}");
    assert!(held.contains("holding for clearance"), "{held:?}");
    assert!(!held.contains("◐"), "{held:?}");

    // Released (the card answered): the step is live again.
    call.borrow_mut().set_waiting_for_clearance(false);
    assert!(!call.borrow().is_waiting_for_clearance());
}

#[test]
fn a_bang_row_shows_its_header_only_in_radar() {
    use hoocode_code_tui_widgets::bash_execution::BashExecutionComponent;
    use std::cell::Cell;
    let _g = lock();
    let radar = Rc::new(Cell::new(true));
    let mut bang = BashExecutionComponent::new("ls", false);
    bang.set_radar_flag(radar.clone());
    bang.append_output("README.md\nsrc\n");
    bang.set_complete(Some(0), false, None, None);
    let radar_rows: Vec<String> = bang.render(80).iter().map(|row| strip(row)).collect();
    assert_eq!(radar_rows.len(), 1, "{radar_rows:?}");
    assert!(radar_rows[0].contains("ls"), "{radar_rows:?}");
    assert!(!radar_rows.iter().any(|row| row.contains("README.md")));

    // Peek keeps the output under the header.
    radar.set(false);
    let peek_rows: Vec<String> = bang.render(80).iter().map(|row| strip(row)).collect();
    assert!(
        peek_rows.iter().any(|row| row.contains("README.md")),
        "{peek_rows:?}"
    );
}

#[test]
fn the_unfocused_task_panel_hides_in_radar() {
    use hoocode_code_task_store::{task_store, CreateTaskOptions};
    use hoocode_code_tui_widgets::task_panel::TaskPanelComponent;
    let _g = lock();
    task_store().create("Read the migration plan", CreateTaskOptions::default());
    let mut panel = TaskPanelComponent::new();
    assert!(!panel.render(80).is_empty());
    panel.set_radar(true);
    assert!(panel.render(80).is_empty());
    panel.set_radar(false);
    assert!(!panel.render(80).is_empty());
}

#[test]
fn a_flying_mission_clock_reads_elapsed_without_a_refresh() {
    // The mission row's time is read off the start on every render, so it
    // reads the time at the moment of drawing (the redraw is the app's tick).
    let palette = Palette::default();
    let flight = FlightStats {
        mark: FlightMark::InFlight,
        steps: 1,
        hits: 0,
        landed_time: None,
        started: Some(std::time::Instant::now() - std::time::Duration::from_secs(13)),
        cost: None,
        chain: Vec::new(),
    };
    let row = strip(&mission_rows(9, "start a background job", &flight, &palette, 80)[0]);
    assert!(row.contains("13s"), "{row:?}");
}
