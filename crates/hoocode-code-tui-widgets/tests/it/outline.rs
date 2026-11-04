//! Direct outline checks (declutter spec "Outline view"): the user message,
//! one recap row per batch of calls, a failed call on its own red row, the
//! answer, and the failed or stopped end line. Steps, bodies and thinking are
//! hidden; a finished turn has no end line.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::{lock, strip};
use hoocode_ai_types::Content;
use hoocode_code_subagents::live;
use hoocode_code_task_store::{task_store, CreateTaskOptions, TaskPatch, TaskSource, TaskStatus};
use hoocode_code_tui_widgets::outline::{
    prose_rows, turn_end, turn_rows, FlightItem, FlightMark, FlightStats, TurnEnd, STEP_INDENT,
};
use hoocode_code_tui_widgets::tool_chain::ToolBlock;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_output_view::ToolOutputView::Outline;
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_tui_ratatui::Palette;
use hoocode_tui_render::Component;
use serde_json::{json, Value};

fn cwd() -> String {
    std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// A call as the transcript holds it: finished with an ok result, failed
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
            view: Outline,
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

/// The turn's rows at `width`, plain, trailing blanks trimmed.
fn rows(prompt: &str, stats: &FlightStats, width: usize) -> Vec<String> {
    turn_rows(prompt, stats, &Palette::default(), width)
        .iter()
        .map(|row| strip(row).trim_end().to_string())
        .collect()
}

fn batch(blocks: Vec<ToolBlock>) -> FlightItem {
    FlightItem::Batch(blocks)
}

#[test]
fn a_finished_turn_is_the_user_message_its_recap_and_its_answer_with_no_end_line() {
    let _g = lock();
    let mut stats = landed(
        2,
        0,
        vec![batch(vec![
            ok("Read", json!({"file_path": "src/config.rs"})),
            ok("Edit", json!({"file_path": "src/config.rs"})),
        ])],
    );
    stats.answer = vec!["Done.".to_string()];
    let rows = rows("fix the loader", &stats, 80);
    assert_eq!(
        rows,
        vec![
            " › fix the loader",
            "",
            " ● Read src/config.rs · Edit src/config.rs",
            "",
            " Done.",
        ],
        "{rows:#?}"
    );
    assert_eq!(turn_end(&stats), None);
}

#[test]
fn the_user_message_wraps_under_its_text_and_every_row_is_full_width() {
    let _g = lock();
    let prompt = "Make the config loader apply environment overrides before the file is parsed";
    let stats = landed(0, 0, Vec::new());
    let styled = turn_rows(prompt, &stats, &Palette::default(), 40);
    let plain = rows(prompt, &stats, 40);
    assert!(plain[0].starts_with(" › Make"), "{plain:#?}");
    assert!(plain[1].starts_with("   ") && !plain[1].starts_with("    "));
    assert!(!plain.join(" ").contains('…'), "{plain:#?}");
    for row in styled.iter().take(3) {
        assert_eq!(hoocode_tui_util::visible_width(row), 40, "{row:?}");
    }
}

#[test]
fn a_command_is_named_by_its_program_and_subcommand() {
    let _g = lock();
    let stats = landed(
        1,
        0,
        vec![batch(vec![ok(
            "Shell",
            json!({"command": "cargo test -p hoocode-config 2>&1 | tail -40"}),
        )])],
    );
    let rows = rows("test it", &stats, 80);
    assert!(
        rows.contains(&" ● Shell cargo test".to_string()),
        "{rows:#?}"
    );
}

#[test]
fn a_recap_row_never_wraps_however_many_calls_it_holds() {
    let _g = lock();
    let blocks: Vec<ToolBlock> = (0..12)
        .map(|i| ok("Read", json!({"file_path": format!("src/file_{i}.rs")})))
        .collect();
    let stats = landed(12, 0, vec![batch(blocks)]);
    let rows = rows("read them", &stats, 60);
    let recap: Vec<&String> = rows.iter().filter(|r| r.starts_with(" ● ")).collect();
    assert_eq!(recap.len(), 1, "{rows:#?}");
    assert!(recap[0].ends_with('…'), "{rows:#?}");
    assert!(hoocode_tui_util::visible_width(recap[0]) < 60);
}

#[test]
fn a_thinking_block_splits_the_recap_and_outline_hides_the_thought() {
    let _g = lock();
    let stats = landed(
        2,
        0,
        vec![
            batch(vec![ok("Read", json!({"file_path": "a.rs"}))]),
            FlightItem::Thinking("Now the edit.".to_string()),
            batch(vec![ok("Edit", json!({"file_path": "a.rs"}))]),
        ],
    );
    let rows = rows("fix it", &stats, 80);
    assert!(rows.contains(&" ● Read a.rs".to_string()), "{rows:#?}");
    assert!(rows.contains(&" ● Edit a.rs".to_string()), "{rows:#?}");
    assert!(!rows.join("\n").contains("Now the edit"), "{rows:#?}");
}

#[test]
fn a_failed_call_gets_its_own_red_row_and_leaves_the_recap() {
    let _g = lock();
    let stats = landed(
        2,
        1,
        vec![batch(vec![
            call(
                "Grep",
                json!({"pattern": "\\\\\""}),
                Some(("regex parse error: unclosed group\nmore", true)),
            ),
            ok("Read", json!({"file_path": "a.rs"})),
        ])],
    );
    let rows = rows("find it", &stats, 80);
    assert!(rows.contains(&" ● Read a.rs".to_string()), "{rows:#?}");
    let failed = rows
        .iter()
        .find(|r| r.starts_with(" ⊗ Grep"))
        .expect("the failed row");
    assert!(
        failed.ends_with("  regex parse error: unclosed group"),
        "{rows:#?}"
    );
    assert!(!rows.join("\n").contains("more"), "{rows:#?}");
    assert!(
        !rows
            .iter()
            .any(|r| r.starts_with(" ● ") && r.contains("Grep")),
        "{rows:#?}"
    );
}

#[test]
fn a_folded_turn_is_one_recap_row_then_the_answer() {
    let _g = lock();
    let mut stats = landed(
        3,
        0,
        vec![
            batch(vec![ok("Read", json!({"file_path": "a.rs"}))]),
            FlightItem::Thinking("next".to_string()),
            batch(vec![
                ok("Edit", json!({"file_path": "a.rs"})),
                ok("Shell", json!({"command": "cargo test"})),
            ]),
        ],
    );
    stats.folded = true;
    stats.answer = vec!["Fixed.".to_string()];
    let rows = rows("fix it", &stats, 100);
    assert_eq!(
        rows,
        vec![
            " › fix it",
            "",
            " ● Read a.rs · Edit a.rs · Shell cargo test",
            "",
            " Fixed.",
        ],
        "{rows:#?}"
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
    assert!(text.contains("\n It is bold."), "{rows:#?}");
    assert!(!text.contains("**"), "{rows:#?}");
    assert!(
        text.contains(" - one") || text.contains(" • one"),
        "{rows:#?}"
    );
}

#[test]
fn a_link_in_the_answer_keeps_its_hyperlink() {
    let _g = lock();
    let mut stats = landed(0, 0, Vec::new());
    stats.answer = vec!["See [the docs](https://example.com/docs).".to_string()];
    let caps = |hyperlinks| hoocode_tui_images::TerminalCapabilities {
        images: None,
        true_color: true,
        hyperlinks,
    };
    hoocode_tui_images::set_capabilities(caps(true));
    let rows = turn_rows("fix it", &stats, &Palette::default(), 80);
    hoocode_tui_images::set_capabilities(caps(false));
    let row = rows
        .iter()
        .find(|row| row.contains("the docs"))
        .expect("the link row");
    assert!(row.contains("\x1b]8;;https://example.com/docs"), "{row:?}");
    assert!(strip(row).starts_with(" See "), "{row:?}");
}

#[test]
fn a_failed_turn_names_its_error_then_ends_with_failed() {
    let _g = lock();
    let mut stats = landed(
        1,
        0,
        vec![batch(vec![ok("Read", json!({"file_path": "a.rs"}))])],
    );
    stats.mark = FlightMark::DidNotLand;
    stats.landed_time = Some("5.0s".to_string());
    stats.reason = Some("connection reset by peer".to_string());
    let rows = rows("try it", &stats, 80);
    let n = rows.len();
    assert_eq!(
        rows[n - 4..],
        ["", " ⊗ connection reset by peer", "", " ⊗ failed · 5.0s"]
    );
    assert_eq!(turn_end(&stats), Some(TurnEnd::Failed));
}

#[test]
fn a_turn_with_only_failed_calls_finished_and_has_no_end_line() {
    let _g = lock();
    let mut stats = landed(
        1,
        1,
        vec![batch(vec![call(
            "Shell",
            json!({"command": "exit 3"}),
            Some(("boom", true)),
        )])],
    );
    stats.mark = FlightMark::DidNotLand;
    let rows = rows("run it", &stats, 80);
    assert_eq!(turn_end(&stats), None);
    assert!(!rows.iter().any(|r| r.contains("failed ·")), "{rows:#?}");
}

#[test]
fn a_stopped_turn_ends_with_stopped_and_its_batch_reads_dim() {
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
    assert_eq!(
        rows,
        vec![" › stop", "", " ⊗ Shell make", "", " ⊗ stopped · 3s"],
        "{rows:#?}"
    );
}

#[test]
fn a_running_turn_reads_its_calls_live_and_has_no_end_line() {
    let _g = lock();
    let stats = FlightStats {
        mark: FlightMark::InFlight,
        steps: 2,
        items: vec![batch(vec![
            ok("Read", json!({"file_path": "a.rs"})),
            running("Shell", json!({"command": "cargo build"})),
        ])],
        ..Default::default()
    };
    let rows = rows("build it", &stats, 80);
    assert_eq!(
        rows,
        vec![" › build it", "", " ● Read a.rs · Shell cargo build"],
        "{rows:#?}"
    );
}

#[test]
fn prose_outside_a_turn_sits_at_the_prose_indent() {
    let _g = lock();
    let ink = Palette::default().ink();
    let row = strip(&prose_rows(&["It wraps text.".to_string()], STEP_INDENT, ink, 80)[0]);
    assert!(row.starts_with(" It wraps text."), "{row:?}");
}

#[test]
fn narrow_rows_never_overflow() {
    let _g = lock();
    let mut stats = landed(
        2,
        1,
        vec![batch(vec![
            ok(
                "Edit",
                json!({"file_path": "src/a/very/long/path/to/a/file.rs"}),
            ),
            call(
                "Shell",
                json!({"command": "cargo test --workspace --all-targets"}),
                Some(("error: a long failure message that will not fit", true)),
            ),
        ])],
    );
    stats.answer = vec!["Done with a long answer that wraps.".to_string()];
    for width in [12usize, 20, 40] {
        for row in turn_rows(
            "wrap this prompt please",
            &stats,
            &Palette::default(),
            width,
        ) {
            assert!(
                hoocode_tui_util::visible_width(&row) <= width,
                "@{width}: {row:?}"
            );
        }
    }
}

#[test]
fn a_subagents_failed_call_is_nested_under_the_recap() {
    let _g = lock();
    live::remember_run("outline-fail-agent-call", "outline-fail-run");
    live::apply("outline-fail-run", &json!({"type": "task_started"}));
    live::apply(
        "outline-fail-run",
        &json!({"type": "tool_execution_start", "toolCallId": "s1", "toolName": "Grep", "args": {"pattern": "x"}}),
    );
    live::apply(
        "outline-fail-run",
        &json!({"type": "tool_execution_end", "toolCallId": "s1", "isError": true,
                "result": {"content": [{"type": "text", "text": "regex parse error"}]}}),
    );
    live::apply("outline-fail-run", &json!({"type": "task_done"}));
    let agent = call_with_id(
        "Agent",
        "outline-fail-agent-call",
        json!({"subagent_type": "explore", "description": "Survey"}),
        Some(("done", false)),
    );
    let stats = landed(1, 1, vec![batch(vec![agent])]);
    let rows = rows("survey", &stats, 80);
    assert_eq!(rows[2], " ● Agent explore · Survey", "{rows:#?}");
    assert!(rows[3].starts_with("   ⊗ Grep x"), "{rows:#?}");
}

#[test]
fn no_space_words_are_left_in_the_outline() {
    let _g = lock();
    let mut stats = landed(
        1,
        1,
        vec![batch(vec![call(
            "Shell",
            json!({"command": "exit 3"}),
            Some(("boom", true)),
        )])],
    );
    stats.reason = Some("boom".to_string());
    for mark in [
        FlightMark::Landed,
        FlightMark::InFlight,
        FlightMark::DidNotLand,
        FlightMark::Aborted,
    ] {
        stats.mark = mark;
        let text = rows("go", &stats, 80).join("\n");
        for word in [
            "LAND", "HIT", "FLIGHT", "WAITING", "ABORTED", "┊", "╰", "▲", "M1",
        ] {
            assert!(!text.contains(word), "{mark:?} {word}: {text}");
        }
    }
}

#[test]
fn the_render_key_moves_with_folding_and_the_items() {
    let _g = lock();
    let mut stats = landed(
        1,
        0,
        vec![batch(vec![ok("Read", json!({"file_path": "a.rs"}))])],
    );
    let before = stats.render_key().expect("a settled turn has a key");
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
fn a_live_background_job_keeps_a_finished_turns_key_off() {
    let _g = lock();
    let key = hoocode_code_tool_bash::jobs::row_key("j-outline");
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
            "bg-outline",
            json!({"command": "npm run dev", "run_in_background": true}),
            ToolExecutionOptions {
                view: Outline,
                ..Default::default()
            },
            None,
            &cwd(),
        );
        component.update_result(
            ToolResult {
                content: vec![Content::text("Started background job j-outline")],
                details: json!({"jobId": "j-outline"}),
                is_error: false,
            },
            false,
        );
        Rc::new(RefCell::new(component))
    };
    let stats = landed(1, 0, vec![batch(vec![job])]);
    // The job's state is read from the task store, so the key must not hold.
    assert!(stats.render_key().is_none());
    let rows = rows("serve", &stats, 80);
    task_store().remove_keyed(&key);
    assert!(rows.contains(&" ● Shell npm run".to_string()), "{rows:#?}");
}

#[test]
fn a_running_turn_without_a_turn_anchor_has_no_key() {
    // A subagent run attached while the parent is idle: no turn anchor, but
    // the turn still runs, so its rows read live state.
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
fn a_frozen_failed_call_keeps_its_row_and_its_key() {
    // The call is drawn, then frozen: freezing releases its result, so the
    // failure and its reason must come from the summary kept.
    let _g = lock();
    let failed = call(
        "Shell",
        json!({"command": "exit 3"}),
        Some(("boom\nsecond line", true)),
    );
    let stats = landed(1, 1, vec![batch(vec![failed.clone()])]);
    let live = rows("run it", &stats, 80);
    assert!(
        live.iter().any(|row| row == " ⊗ Shell exit 3  boom"),
        "{live:#?}"
    );
    failed.borrow_mut().render(80);
    failed.borrow_mut().freeze();
    failed.borrow_mut().render(80);
    let frozen = rows("run it", &stats, 80);
    assert_eq!(frozen, live);
    assert!(stats.render_key().is_some());
}
