//! Direct Mission Control radar checks (R3): the mission block on the `┊` rail,
//! its batches, nested hit rows, thinking lines, the answer, the folded
//! summary and the turn end (LANDED, DID NOT LAND, IN FLIGHT, WAITING).

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_subagents::live;
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};
use hoocode_code_tui_widgets::radar::{
    mission_rows, prose_rows, think_rows, FlightItem, FlightMark, FlightStats, STEP_INDENT,
};
use hoocode_code_tui_widgets::tool_chain::ToolBlock;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView::Radar;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::Component;
use serde_json::{json, Value};
use std::time::Instant;

fn cwd() -> String {
    std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// A call as the transcript holds it: `finished` with an ok result, `failed`
/// with an error, or still running (no result).
fn call(tool: &str, args: Value, outcome: Option<(&str, bool)>) -> ToolBlock {
    let id = format!("{tool}-{args}");
    call_with_id(tool, &id, args, outcome)
}

fn call_with_id(tool: &str, id: &str, args: Value, outcome: Option<(&str, bool)>) -> ToolBlock {
    let mut component = ToolExecutionComponent::new(
        tool,
        id,
        args,
        ToolExecutionOptions {
            view: Radar,
            ..Default::default()
        },
        None,
        &cwd(),
    );
    if let Some((text, is_error)) = outcome {
        component.update_result(
            ToolResult {
                content: vec![Content::text(text)],
                details: json!({}),
                is_error,
            },
            false,
        );
    }
    Rc::new(RefCell::new(component))
}

fn ok(tool: &str, args: Value) -> ToolBlock {
    call(tool, args, Some(("ok", false)))
}

fn running(tool: &str, args: Value) -> ToolBlock {
    call(tool, args, None)
}

fn landed(steps: usize, hits: usize, items: Vec<FlightItem>) -> FlightStats {
    FlightStats {
        mark: FlightMark::Landed,
        steps,
        hits,
        landed_time: Some("42s".to_string()),
        started: None,
        cost: Some("$0.031".to_string()),
        tokens: Some("2.1k tokens".to_string()),
        items,
        ..Default::default()
    }
}

/// The mission's rows at `width`, plain.
fn rows(prompt: &str, stats: &FlightStats, width: usize) -> Vec<String> {
    mission_rows(1, prompt, stats, &Palette::default(), width)
        .iter()
        .map(|row| strip(row))
        .collect()
}

fn batch(blocks: Vec<ToolBlock>) -> FlightItem {
    FlightItem::Batch(blocks)
}

#[test]
fn the_turn_end_closes_the_rail_with_the_landing_words_and_stats() {
    let _g = lock();
    let stats = landed(
        9,
        1,
        vec![batch(vec![ok("Edit", json!({"file_path": "src/lex.rs"}))])],
    );
    let rows = rows("fix the escaped-quote test", &stats, 80);
    assert_eq!(
        rows.last().unwrap().trim_end(),
        "   ╰ ● LANDED · 9 steps · 1 hit · 42s · 2.1k tokens · $0.031",
        "{rows:#?}"
    );
}

#[test]
fn the_header_reads_mission_and_prompt_with_no_stats() {
    let _g = lock();
    let stats = landed(3, 0, Vec::new());
    let rows = rows("fix the escaped-quote test", &stats, 80);
    assert_eq!(
        rows[0].trim_end(),
        "▲ M1  fix the escaped-quote test",
        "{rows:#?}"
    );
}

#[test]
fn a_long_prompt_wraps_under_its_first_word_and_keeps_its_stats_off_the_header() {
    let _g = lock();
    let prompt = "fix the overflow bug in the wrap helper so long mission prompts never get cut\nthen run the whole suite";
    let stats = landed(3, 0, Vec::new());
    let header: Vec<String> = rows(prompt, &stats, 60);
    assert!(header.len() >= 4, "{header:#?}");
    assert!(header[0].starts_with("▲ M1  fix the"), "{header:#?}");
    for row in &header[1..3] {
        // Hung under the prompt text, which starts at column 6.
        assert!(row.starts_with("      "), "{row:?}");
        assert!(!row.starts_with("       "), "{row:?}");
    }
    // The header rows only: the turn end below is cut to its width.
    for row in &header[..header.len() - 1] {
        assert!(!row.contains('…'), "{header:#?}");
        assert!(row.chars().count() <= 59, "{row:?}");
    }
    let flat = header
        .iter()
        .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(" ");
    for word in prompt.split_whitespace() {
        assert!(flat.contains(word), "{word}: {flat}");
    }
    assert!(!header[0].contains("steps"), "{header:#?}");
}

#[test]
fn a_settled_batch_is_one_line_naming_the_change_with_its_call_count_flush_right() {
    let _g = lock();
    let stats = landed(
        3,
        0,
        vec![batch(vec![
            ok("Read", json!({"file_path": "src/lex.rs"})),
            ok("Edit", json!({"file_path": "src/lex.rs"})),
            ok("Shell", json!({"command": "cargo test"})),
        ])],
    );
    let rows = rows("fix it", &stats, 80);
    let line = &rows[1];
    assert!(line.starts_with("   ┊ ● "), "{line:?}");
    assert!(line.trim_end().ends_with("3 calls"), "{line:?}");
    assert!(line.contains("cargo test"), "{line:?}");
    assert_eq!(line.chars().count(), 79, "{line:?}");
}

#[test]
fn a_batch_line_never_wraps_however_many_calls_it_holds() {
    let _g = lock();
    let tools = ["Read", "Grep", "Edit", "Shell", "Write", "WebFetch"];
    let blocks = tools
        .iter()
        .map(|tool| {
            running(
                tool,
                json!({"file_path": "a.rs", "pattern": "x", "command": "y"}),
            )
        })
        .collect();
    let stats = FlightStats {
        mark: FlightMark::InFlight,
        steps: 6,
        started: Some(std::time::Instant::now()),
        items: vec![batch(blocks)],
        ..Default::default()
    };
    // Narrow: one row, cut with `…`, the call count kept.
    let narrow = rows("wide", &stats, 40);
    let live: Vec<&String> = narrow.iter().filter(|row| row.contains("◐ …")).collect();
    assert_eq!(live.len(), 1, "{narrow:#?}");
    assert!(live[0].chars().count() <= 39, "{:?}", live[0]);
    assert!(live[0].contains("6 calls"), "{:?}", live[0]);
    // Wide: the last four tools, and not the two before them.
    let wide = rows("wide", &stats, 120);
    let live: Vec<&String> = wide.iter().filter(|row| row.contains("◐ …")).collect();
    assert_eq!(live.len(), 1, "{wide:#?}");
    assert!(
        live[0].contains("Edit › Shell › Write › WebFetch"),
        "{:?}",
        live[0]
    );
    assert!(!live[0].contains("Read"), "{:?}", live[0]);
}

#[test]
fn a_batch_splits_at_a_thinking_block_and_radar_hides_the_thought() {
    let _g = lock();
    let gist = "The helper is 13 lines and the wrap point is at the word boundary every time";
    let stats = landed(
        2,
        0,
        vec![
            batch(vec![ok("Read", json!({"file_path": "a.rs"}))]),
            FlightItem::Thinking(gist.to_string()),
            batch(vec![ok("Edit", json!({"file_path": "a.rs"}))]),
        ],
    );
    let rows = rows("fix it", &stats, 50);
    let batches = rows.iter().filter(|row| row.starts_with("   ┊ ●")).count();
    assert_eq!(batches, 2, "the thought still splits the batch: {rows:#?}");
    assert!(
        !rows
            .iter()
            .any(|row| row.contains('✻') || row.contains("The helper")),
        "radar shows no thinking: {rows:#?}"
    );
}

#[test]
fn a_failed_call_is_a_nested_hit_row_and_the_batch_takes_the_failure_mark() {
    let _g = lock();
    let stats = landed(
        2,
        1,
        vec![batch(vec![
            ok("Read", json!({"file_path": "a.rs"})),
            call(
                "Shell",
                json!({"command": "cargo test -p hoocode-parser"}),
                Some(("1 failed: raw_string_escape", true)),
            ),
        ])],
    );
    let rows = rows("fix it", &stats, 80);
    assert!(rows[1].starts_with("   ┊ ⊗ "), "{rows:#?}");
    let hit = rows
        .iter()
        .find(|row| row.contains("HIT"))
        .expect("a hit row");
    // Nested two columns under the batch mark: the `└` sits at column 7.
    assert!(hit.starts_with("   ┊   └ 02 Shell  "), "{hit:?}");
    assert!(hit.contains("→ 1 failed"), "{hit:?}");
    assert!(hit.trim_end().ends_with("HIT"), "{hit:?}");
}

#[test]
fn a_folded_mission_is_one_summary_line_then_the_answer_then_the_turn_end() {
    let _g = lock();
    let mut stats = landed(
        2,
        0,
        vec![batch(vec![
            ok("Edit", json!({"file_path": "a.rs"})),
            ok("Edit", json!({"file_path": "b.rs"})),
        ])],
    );
    stats.folded = true;
    stats.answer = vec!["The lexer now owns its token text.".to_string()];
    let rows = rows("older mission", &stats, 80);
    assert!(rows[1].starts_with("   ┊ ● Edited"), "{rows:#?}");
    assert!(rows[1].trim_end().ends_with("2 calls"), "{rows:#?}");
    assert_eq!(rows[2], "   ┊", "{rows:#?}");
    assert!(
        rows[3].contains("The lexer now owns its token text."),
        "{rows:#?}"
    );
    assert!(rows.iter().all(|row| !row.contains("HIT")), "{rows:#?}");
    assert!(
        rows.last().unwrap().starts_with("   ╰ ● LANDED"),
        "{rows:#?}"
    );
}

#[test]
fn the_answer_sits_on_the_rail_after_a_blank_rail_line() {
    let _g = lock();
    let mut stats = landed(
        1,
        0,
        vec![batch(vec![ok("Read", json!({"file_path": "a.rs"}))])],
    );
    stats.answer = vec!["It wraps text.".to_string()];
    let rows = rows("fix it", &stats, 80);
    let answer = rows
        .iter()
        .position(|row| row.contains("It wraps text."))
        .expect("the answer row");
    assert_eq!(rows[answer - 1], "   ┊", "{rows:#?}");
    assert!(rows[answer].starts_with("   ┊ It wraps text."), "{rows:#?}");
}

#[test]
fn a_link_in_the_answer_keeps_its_hyperlink() {
    let _g = lock();
    let mut stats = landed(
        1,
        0,
        vec![batch(vec![ok("Read", json!({"file_path": "a.rs"}))])],
    );
    stats.answer = vec!["See [the docs](https://example.com/docs).".to_string()];
    let caps = |hyperlinks| hoocode_tui_images::TerminalCapabilities {
        images: None,
        true_color: true,
        hyperlinks,
    };
    hoocode_tui_images::set_capabilities(caps(true));
    let rows = mission_rows(1, "fix it", &stats, &Palette::default(), 80);
    hoocode_tui_images::set_capabilities(caps(false));
    let row = rows
        .iter()
        .find(|row| row.contains("the docs"))
        .expect("the link row");
    assert!(row.contains("\x1b]8;;https://example.com/docs"), "{row:?}");
    assert!(strip(row).starts_with("   ┊ See "), "{row:?}");
    assert_eq!(hoocode_tui_util::visible_width(row), 80, "{row:?}");
    // The click finds the link under its text.
    let column = strip(row).find("the docs").unwrap() as i64;
    assert_eq!(
        hoocode_tui_util::hyperlink_at(row, column).as_deref(),
        Some("https://example.com/docs")
    );
}

#[test]
fn the_answer_renders_as_markdown_like_peek() {
    let _g = lock();
    let mut stats = landed(
        1,
        0,
        vec![batch(vec![ok("Read", json!({"file_path": "a.rs"}))])],
    );
    stats.answer = vec!["It is **bold**.\n\n- one\n- two".to_string()];
    let rows = rows("fix it", &stats, 80);
    let text = rows.join("\n");
    // The marks are drawn, not shown: no `**`, bullets as list items, and a
    // blank rail row between the paragraph and the list.
    assert!(text.contains("┊ It is bold."), "{rows:#?}");
    assert!(!text.contains("**"), "{rows:#?}");
    assert!(
        text.contains("┊ - one") || text.contains("┊ • one"),
        "{rows:#?}"
    );
}

#[test]
fn a_held_call_shows_its_hold_and_the_turn_end_reads_waiting() {
    let _g = lock();
    let held = running("Shell", json!({"command": "rm -rf target/"}));
    held.borrow_mut().mark_execution_started();
    held.borrow_mut().set_waiting_for_clearance(true);
    let stats = FlightStats {
        mark: FlightMark::InFlight,
        steps: 1,
        started: Some(std::time::Instant::now()),
        items: vec![batch(vec![held])],
        ..Default::default()
    };
    let rows = rows("clean up", &stats, 80);
    let nested = rows
        .iter()
        .find(|row| row.contains("holding for clearance"))
        .expect("a hold row");
    assert!(nested.starts_with("   ┊   └ ◉ Shell"), "{nested:?}");
    let turn_end = rows.last().unwrap();
    assert!(
        turn_end.starts_with("   ╰ ◆ WAITING · approve Shell"),
        "{turn_end:?}"
    );
    assert!(!turn_end.contains("IN FLIGHT"), "{turn_end:?}");
}

#[test]
fn a_flying_turn_end_reads_in_flight_with_its_steps_and_clock() {
    let _g = lock();
    let stats = FlightStats {
        mark: FlightMark::InFlight,
        steps: 25,
        started: Some(std::time::Instant::now() - std::time::Duration::from_secs(72)),
        items: vec![batch(vec![running("Shell", json!({"command": "make"}))])],
        ..Default::default()
    };
    let rows = rows("long run", &stats, 80);
    let turn_end = rows.last().unwrap();
    assert!(
        turn_end.starts_with("   ╰ ◐ IN FLIGHT · 25 steps · 0 hits · 1m12s"),
        "{turn_end:?}"
    );
}

#[test]
fn a_did_not_land_turn_end_names_its_reason_at_the_end() {
    let _g = lock();
    let stats = FlightStats {
        mark: FlightMark::DidNotLand,
        steps: 2,
        hits: 1,
        landed_time: Some("5.0s".to_string()),
        reason: Some("Request failed: connection reset by peer".to_string()),
        items: vec![batch(vec![ok("Read", json!({"file_path": "a.rs"}))])],
        ..Default::default()
    };
    let rows = rows("try it", &stats, 100);
    assert_eq!(
        rows.last().unwrap().trim_end(),
        "   ╰ ⊗ DID NOT LAND · 2 steps · 1 hit · 5.0s · Request failed: connection reset by peer"
    );
}

#[test]
fn an_aborted_turn_end_reads_aborted_with_its_stats() {
    let _g = lock();
    let stats = FlightStats {
        mark: FlightMark::Aborted,
        steps: 12,
        landed_time: Some("30s".to_string()),
        items: Vec::new(),
        ..Default::default()
    };
    let rows = rows("stop", &stats, 80);
    assert_eq!(
        rows.last().unwrap().trim_end(),
        "   ╰ ⊘ ABORTED · 12 steps · 0 hits · 30s"
    );
}

#[test]
fn thinking_wraps_under_its_gist_outside_a_mission() {
    // Thinking outside a mission keeps its own rows (`think_rows`).
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
    let flat = rows
        .iter()
        .map(|row| row.trim_start_matches("        ✻ thought 3s · "))
        .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(flat, gist);
}

#[test]
fn prose_indents_eight_under_a_mission() {
    let palette = Palette::default();
    let ink = palette.ink();
    let plain = |text: &str| vec![text.to_string()];
    let under_mission = strip(&prose_rows(&plain("It wraps text."), STEP_INDENT, ink, 80)[0]);
    assert_eq!(under_mission.trim_end(), "        It wraps text.");
}

#[test]
fn narrow_mission_rows_keep_the_last_column_blank_and_never_overflow() {
    let _g = lock();
    let blocks = vec![
        ok("Read", json!({"file_path": "a.rs"})),
        call(
            "Shell",
            json!({"command": "echo boom; exit 3"}),
            Some(("boom", true)),
        ),
        running(
            "Agent",
            json!({"subagent_type": "general-purpose", "description": "Compute a product"}),
        ),
    ];
    let mut stats = landed(12, 1, vec![batch(blocks)]);
    stats
        .items
        .push(FlightItem::Thinking("The helper is 13 lines".into()));
    stats.answer = vec!["It wraps text and goes on".to_string()];
    let mut folded = stats.clone();
    folded.folded = true;
    for width in [1usize, 10, 12, 20, 24, 40] {
        let mut lines = mission_rows(
            12,
            "a long prompt that goes on",
            &stats,
            &Palette::default(),
            width,
        );
        lines.extend(mission_rows(1, "wrap", &folded, &Palette::default(), width));
        for row in lines {
            let text = strip(&row);
            assert!(
                text.chars().count() < width.max(1),
                "width {width}: {text:?}"
            );
        }
    }
}

#[test]
fn a_mission_block_carries_its_header_body_and_turn_end_in_order() {
    let _g = lock();
    let stats = landed(
        2,
        0,
        vec![
            batch(vec![ok("Read", json!({"file_path": "a.rs"}))]),
            FlightItem::Thinking("check the second file".to_string()),
            batch(vec![ok("Edit", json!({"file_path": "a.rs"}))]),
        ],
    );
    let rows = rows("fix it", &stats, 80);
    assert!(rows[0].starts_with("▲ M1  fix it"), "{rows:#?}");
    assert!(rows.last().unwrap().starts_with("   ╰ "), "{rows:#?}");
    // Every body row sits on the rail (the header and the turn end aside).
    for row in &rows[1..rows.len() - 1] {
        assert!(row.starts_with("   ┊"), "{row:?}");
    }
}

#[test]
fn a_subagent_hold_reads_waiting_on_the_turn_end() {
    let _g = lock();
    live::remember_run("radar-hold-agent-call", "radar-hold-run");
    live::apply("radar-hold-run", &json!({"type": "task_started"}));
    live::apply(
        "radar-hold-run",
        &json!({"type": "task_clearance", "tool": "Edit", "subject": "src/lex.rs", "held": true, "call": "k1"}),
    );
    let agent = call_with_id(
        "Agent",
        "radar-hold-agent-call",
        json!({"subagent_type": "explore", "description": "Survey"}),
        None,
    );
    let stats = FlightStats {
        mark: FlightMark::InFlight,
        steps: 1,
        started: Some(Instant::now()),
        items: vec![batch(vec![agent])],
        ..Default::default()
    };
    let rows = rows("survey", &stats, 80);
    assert!(
        rows.last()
            .unwrap()
            .starts_with("   ╰ ◆ WAITING · approve Edit src/lex.rs"),
        "{rows:#?}"
    );
    live::apply("radar-hold-run", &json!({"type": "task_done"}));
}

#[test]
fn a_failed_subagent_call_turns_its_settled_batch_red_and_nests_its_hit() {
    let _g = lock();
    live::remember_run("radar-fail-agent-call", "radar-fail-run");
    live::apply("radar-fail-run", &json!({"type": "task_started"}));
    live::apply(
        "radar-fail-run",
        &json!({"type": "tool_execution_start", "toolCallId": "s1", "toolName": "Grep", "args": {"pattern": "x"}}),
    );
    live::apply(
        "radar-fail-run",
        &json!({"type": "tool_execution_end", "toolCallId": "s1", "isError": true}),
    );
    live::apply("radar-fail-run", &json!({"type": "task_done"}));
    let agent = call_with_id(
        "Agent",
        "radar-fail-agent-call",
        json!({"subagent_type": "explore", "description": "Survey"}),
        Some(("done", false)),
    );
    let stats = landed(1, 1, vec![batch(vec![agent])]);
    let rows = rows("survey", &stats, 80);
    assert!(rows[1].starts_with("   ┊ ⊗ "), "{rows:#?}");
    assert!(
        rows.iter()
            .any(|row| row.contains("01.1 Grep") && row.contains("HIT")),
        "{rows:#?}"
    );
}

#[test]
fn an_interrupted_batch_reads_dim_cancelled_never_green() {
    let _g = lock();
    let stopped = call(
        "Shell",
        json!({"command": "make"}),
        Some(("cut short", false)),
    );
    stopped.borrow_mut().set_interrupted();
    let stats = FlightStats {
        mark: FlightMark::Aborted,
        landed_time: Some("3s".to_string()),
        items: vec![batch(vec![stopped])],
        ..Default::default()
    };
    let rows = rows("stop", &stats, 80);
    assert!(rows[1].starts_with("   ┊ ⊘ "), "{rows:#?}");
    assert!(
        rows.iter().all(|row| !row.starts_with("   ┊ ●")),
        "{rows:#?}"
    );
}

#[test]
fn a_did_not_land_reason_that_does_not_fit_goes_on_its_own_line() {
    let _g = lock();
    let stats = FlightStats {
        mark: FlightMark::DidNotLand,
        steps: 2,
        hits: 1,
        landed_time: Some("5.0s".to_string()),
        reason: Some("Request failed: connection reset by peer".to_string()),
        items: vec![batch(vec![ok("Read", json!({"file_path": "a.rs"}))])],
        ..Default::default()
    };
    let rows = rows("try it", &stats, 60);
    let turn = rows.len() - 2;
    assert!(
        rows[turn].starts_with("   ╰ ⊗ DID NOT LAND · 2 steps · 1 hit · 5.0s"),
        "{rows:#?}"
    );
    assert_eq!(
        rows.last().unwrap().trim_end(),
        "     Request failed: connection reset by peer",
        "{rows:#?}"
    );
}

#[test]
fn the_render_key_moves_with_folding_and_the_items() {
    let _g = lock();
    let mut stats = landed(
        1,
        0,
        vec![batch(vec![ok("Read", json!({"file_path": "a.rs"}))])],
    );
    let before = stats.render_key().expect("a settled mission has a key");
    stats.folded = true;
    assert_ne!(stats.render_key(), Some(before));
    stats.folded = false;
    assert_eq!(stats.render_key(), Some(before));
    stats.items.push(FlightItem::Thinking("more".to_string()));
    assert_ne!(stats.render_key(), Some(before));
}

#[test]
fn a_held_call_keeps_the_key_off_so_its_row_is_drawn_fresh() {
    let _g = lock();
    let held = running("Shell", json!({"command": "rm -rf target/"}));
    held.borrow_mut().mark_execution_started();
    held.borrow_mut().set_waiting_for_clearance(true);
    let stats = landed(1, 0, vec![batch(vec![held])]);
    assert!(stats.render_key().is_none());
}

#[test]
fn a_live_background_job_keeps_a_landed_missions_key_off() {
    let _g = lock();
    let key = hoocode_code_tool_bash::jobs::row_key("j-radar");
    task_store().upsert_keyed(
        &key,
        "npm run dev",
        CreateTaskOptions {
            source: Some(TaskSource::Shell),
            key: Some(key.clone()),
            ..Default::default()
        },
        TaskPatch {
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        },
    );
    let job = {
        let mut component = ToolExecutionComponent::new(
            "Shell",
            "bg-radar",
            json!({"command": "npm run dev", "run_in_background": true}),
            ToolExecutionOptions {
                view: Radar,
                ..Default::default()
            },
            None,
            &cwd(),
        );
        component.update_result(
            ToolResult {
                content: vec![Content::text("Started background job j-radar")],
                details: json!({"jobId": "j-radar"}),
                is_error: false,
            },
            false,
        );
        Rc::new(RefCell::new(component))
    };
    let stats = landed(1, 0, vec![batch(vec![job])]);
    let first = rows("serve", &stats, 80);
    // The job's row is read from the task store, so the key must not hold.
    assert!(stats.render_key().is_none());
    let second = rows("serve", &stats, 80);
    task_store().remove_keyed(&key);
    assert!(first.iter().any(|row| row.contains("⌁")), "{first:#?}");
    assert!(second.iter().any(|row| row.contains("⌁")), "{second:#?}");
}

#[test]
fn a_flying_mission_without_a_turn_anchor_has_no_key() {
    // A subagent run attached while the parent is idle: no turn anchor, but
    // the mission still flies, so its rows read live state.
    let _g = lock();
    let stats = FlightStats {
        mark: FlightMark::InFlight,
        steps: 1,
        items: vec![batch(vec![ok("Read", json!({"file_path": "a.rs"}))])],
        ..Default::default()
    };
    assert!(stats.started.is_none());
    assert!(stats.render_key().is_none());
}

#[test]
fn a_frozen_failed_call_keeps_its_hit_its_mark_and_its_key() {
    // The call is drawn, then frozen: freezing releases its result, so the
    // failure, its snippet and the batch's mark must come from the summary kept.
    let _g = lock();
    let failed = call(
        "Shell",
        json!({"command": "exit 3"}),
        Some(("boom\nsecond line", true)),
    );
    let stats = landed(1, 1, vec![batch(vec![failed.clone()])]);
    let live = rows("run it", &stats, 80);
    assert!(
        live.iter()
            .any(|row| row.contains("boom") && row.contains("HIT")),
        "{live:#?}"
    );
    failed.borrow_mut().render(80);
    failed.borrow_mut().freeze();
    failed.borrow_mut().render(80);
    let frozen = rows("run it", &stats, 80);
    assert!(
        frozen
            .iter()
            .any(|row| row.contains("boom") && row.contains("HIT")),
        "{frozen:#?}"
    );
    assert!(
        frozen
            .iter()
            .any(|row| row.contains('⊗') && row.contains("1 call")),
        "the batch takes the failure mark: {frozen:#?}"
    );
    assert_eq!(frozen, live);
    assert!(stats.render_key().is_some());
}
