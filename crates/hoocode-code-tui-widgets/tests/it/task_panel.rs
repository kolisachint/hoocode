//! Port of `test/task-panel.test.ts` (the teams lens and its focus tests are
//! gone with decision 9).
//!
//! The store is process-global, so every test holds [`lock`] and starts from
//! a cleared store. Not ported: the two "warns on unknown id" store tests,
//! which assert `console.warn` output (the Rust store ignores unknown ids
//! without printing).

use std::sync::{Mutex, MutexGuard};

use hoocode_code_task_store::{
    task_store, AgentStats, CreateTaskOptions, TaskAgentKind, TaskAgentPatch, TaskAgentState,
    TaskPatch, TaskSource, TaskStatus, TaskUsage,
};
use hoocode_code_tui_theme::init_theme;
use hoocode_code_tui_widgets::task_panel::{TaskPanelComponent, TaskPanelView};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    init_theme(Some("dark"), false);
    task_store().clear();
    guard
}

fn strip(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn render(panel: &mut TaskPanelComponent, width: u16) -> Vec<String> {
    panel.render(width)
}

fn plain(panel: &mut TaskPanelComponent, width: u16) -> Vec<String> {
    panel.render(width).iter().map(|l| strip(l)).collect()
}

fn create(title: &str) -> u64 {
    task_store().create(title, CreateTaskOptions::default()).id
}

fn create_with(title: &str, options: CreateTaskOptions) -> u64 {
    task_store().create(title, options).id
}

fn sub(mode: &str) -> CreateTaskOptions {
    CreateTaskOptions {
        source: Some(TaskSource::Subagent),
        subagent_mode: Some(mode.into()),
        ..Default::default()
    }
}

fn mcp(server: Option<&str>) -> CreateTaskOptions {
    CreateTaskOptions {
        source: Some(TaskSource::Mcp),
        subagent_mode: server.map(String::from),
        ..Default::default()
    }
}

fn mode(mode: &str) -> CreateTaskOptions {
    CreateTaskOptions {
        subagent_mode: Some(mode.into()),
        ..Default::default()
    }
}

fn owned(agent: &str) -> CreateTaskOptions {
    CreateTaskOptions {
        agent: Some(agent.into()),
        ..Default::default()
    }
}

fn set(id: u64, status: TaskStatus) {
    task_store().update(
        id,
        TaskPatch {
            status: Some(status),
            ..Default::default()
        },
    );
}

fn usage(input: f64, output: f64, cost: f64) -> TaskUsage {
    TaskUsage {
        input,
        output,
        cost,
        ..Default::default()
    }
}

/// A console row without its right border and the padding before it.
fn inner(row: &str) -> &str {
    row.trim_end().trim_end_matches('┃').trim_end()
}

fn find<'a>(lines: &'a [String], needle: &str) -> &'a String {
    lines
        .iter()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?} in {lines:#?}"))
}

#[test]
fn collapses_to_empty_when_there_are_no_tasks() {
    let _g = lock();
    assert!(render(&mut TaskPanelComponent::new(), 120).is_empty());
}

#[test]
fn shows_active_subagent_tasks_with_status_icons_ids_and_mode_tags() {
    let _g = lock();
    let explore = create_with("SSE watch endpoint", mode("explore"));
    let edit = create_with("Auth refactor", mode("edit"));
    let plain_task = create("Init project");
    set(explore, TaskStatus::InProgress);
    set(edit, TaskStatus::Pending);
    set(plain_task, TaskStatus::InProgress);

    let lines = render(&mut TaskPanelComponent::new(), 120);
    assert_eq!(lines.len(), 6, "frame, tab strip, three rows, foot");
    let text = lines.join("\n");
    for s in [
        "SSE watch endpoint",
        "Auth refactor",
        "Init project",
        "[explore]",
        "[edit]",
        "◉",
        "○",
    ] {
        assert!(text.contains(s), "{s}");
    }
    for s in ["●", "⠋", "queued", "running…", "◆"] {
        assert!(!text.contains(s), "{s}");
    }
}

#[test]
fn each_lens_carries_the_owner_glyph_and_tag_for_its_own_rows() {
    let _g = lock();
    let s = create_with("find the bug", sub("explore"));
    let m = create_with("fetch", mcp(Some("web")));
    let p = create("init project");
    for id in [s, m, p] {
        set(id, TaskStatus::InProgress);
    }
    let mut panel = TaskPanelComponent::new();

    panel.set_view(TaskPanelView::Plan);
    let flat = render(&mut panel, 120);
    let plain_row = find(&flat, "init project");
    assert!(!plain_row.contains('◈') && !plain_row.contains('⧉'));
    assert!(!flat.iter().any(|l| l.contains("find the bug")));
    assert!(!flat.iter().any(|l| l.contains("fetch")));

    panel.set_view(TaskPanelView::Subagents);
    let sa = render(&mut panel, 120);
    let sub_row = find(&sa, "find the bug");
    assert!(sub_row.contains('◈') && sub_row.contains("[explore]"));
    assert!(
        !sa.iter().any(|l| l.contains("fetch")),
        "MCP calls have their own tab"
    );
    panel.set_view(TaskPanelView::Mcp);
    let mc = render(&mut panel, 120);
    let mcp_row = find(&mc, "fetch");
    assert!(mcp_row.contains('⧉') && mcp_row.contains("[web]"));
    for row in sa.iter().chain(&mc) {
        assert_eq!(visible_width(row), 120);
    }
}

#[test]
fn an_mcp_row_without_a_recorded_server_falls_back_to_the_mcp_tag() {
    let _g = lock();
    let m = create_with("legacy call", mcp(None));
    set(m, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Mcp);
    let text = render(&mut panel, 120).join("\n");
    assert!(text.contains('⧉') && text.contains("[MCP]"));
}

#[test]
fn the_agents_and_mcp_tabs_keep_each_rows_mode_and_server_tag() {
    let _g = lock();
    let s = create_with("find the bug", sub("explore"));
    let m = create_with("fetch", mcp(Some("web")));
    set(s, TaskStatus::InProgress);
    set(m, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    let lines = plain(&mut panel, 120);
    assert!(find(&lines, "find the bug").contains("[explore]"));
    panel.set_view(TaskPanelView::Mcp);
    let lines = plain(&mut panel, 120);
    assert!(find(&lines, "fetch").contains("[web]"));
}

fn explore_agent(activity: &str) {
    task_store().upsert_agent(
        "explore",
        "explore",
        TaskAgentKind::Subagent,
        TaskAgentPatch {
            state: Some(TaskAgentState::Running),
            activity: Some(activity.into()),
            ..Default::default()
        },
    );
}

fn explore_run() -> u64 {
    let id = create_with(
        "trace the auth flow",
        CreateTaskOptions {
            agent: Some("explore".into()),
            ..sub("explore")
        },
    );
    set(id, TaskStatus::InProgress);
    id
}

#[test]
fn an_in_progress_subagent_row_shows_the_owners_live_tool_activity() {
    let _g = lock();
    explore_agent("CodeSearch");
    explore_run();
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    let lines = plain(&mut panel, 120);
    let row = find(&lines, "trace the auth flow");
    assert!(row.contains("⋯ CodeSearch · "), "{row}");
    let after = row.split("⋯ CodeSearch · ").nth(1).unwrap();
    assert!(after.starts_with(|c: char| c.is_ascii_digit()), "{row}");
}

#[test]
fn an_idle_in_progress_subagent_row_keeps_its_run_clock() {
    let _g = lock();
    explore_agent("");
    explore_run();
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    let lines = plain(&mut panel, 120);
    let row = inner(find(&lines, "trace the auth flow")).to_string();
    assert!(row.ends_with('s'), "{row}");
    let clock = row.rsplit(' ').next().unwrap();
    assert!(clock.trim_end_matches('s').parse::<f64>().is_ok(), "{row}");
    assert!(!row.contains('⋯') && !row.contains("running…"));
}

#[test]
fn an_in_progress_main_plan_row_carries_its_elapsed_time() {
    let _g = lock();
    let t = create("Patch the warning threshold");
    set(t, TaskStatus::InProgress);
    let lines = plain(&mut TaskPanelComponent::new(), 120);
    let row = find(&lines, "Patch the warning threshold");
    // Waypoints name their time (spec §4 c6); the free-running clock is
    // for delegated rows.
    let trimmed = inner(row);
    assert!(
        trimmed.ends_with('s') && !trimmed.ends_with("threshold"),
        "{row}"
    );
}

#[test]
fn title_column_stays_aligned_across_single_and_double_digit_ids() {
    let _g = lock();
    for i in 1..=10 {
        let t = create(&format!("task-{i}"));
        set(t, TaskStatus::InProgress);
    }
    // The console shows six rows at a time: the pick on the last one windows
    // task-5 to task-10 into view.
    let mut panel = focused();
    render(&mut panel, 120);
    for _ in 0..9 {
        press(&mut panel, DOWN);
    }
    let lines = plain(&mut panel, 120);
    // Columns, not bytes: the pick's `›` is wider in bytes than the gutter.
    let column = |row: &str, title: &str| visible_width(&row[..row.find(title).unwrap()]);
    let row5 = find(&lines, "task-5");
    let row10 = find(&lines, "task-10");
    assert_eq!(column(row5, "task-5"), column(row10, "task-10"));
}

#[test]
fn completed_and_failed_tasks_stay_visible_with_their_status() {
    let _g = lock();
    create_with("Still running", mode("explore"));
    let d = create("Finished work");
    set(d, TaskStatus::Done);
    let f = create("Broken build");
    set(f, TaskStatus::Failed);
    let lines = render(&mut TaskPanelComponent::new(), 120);
    assert_eq!(lines.len(), 6, "frame, tab strip, three rows, foot");
    let text = lines.join("\n");
    for s in ["Still running", "Finished work", "Broken build", "●", "⊗"] {
        assert!(text.contains(s), "{s}");
    }
}

#[test]
fn a_task_note_renders_as_a_warning_cue_replacing_the_usage_stamp() {
    let _g = lock();
    let d = create("Audit reconnect path");
    task_store().update(
        d,
        TaskPatch {
            status: Some(TaskStatus::Done),
            usage: Some(usage(4000.0, 500.0, 0.0)),
            note: Some(Some("ran on inherited model".into())),
            ..Default::default()
        },
    );
    let s = create("Run suite");
    task_store().update(
        s,
        TaskPatch {
            status: Some(TaskStatus::Failed),
            note: Some(Some("anthropic exhausted".into())),
            ..Default::default()
        },
    );
    let lines = render(&mut TaskPanelComponent::new(), 120);
    let note_row = find(&lines, "Audit reconnect path");
    assert!(note_row.contains("\u{26a0}\u{fe0e} ran on inherited model"));
    assert!(!note_row.contains("4.5k"));
    assert!(find(&lines, "Run suite").contains("\u{26a0}\u{fe0e} anthropic exhausted"));
}

#[test]
fn long_titles_use_the_full_left_width() {
    let _g = lock();
    let a =
        create("A deliberately long task title that should fill the row up to the right column");
    set(a, TaskStatus::InProgress);
    let lines = render(&mut TaskPanelComponent::new(), 80);
    let row = lines
        .iter()
        .find(|l| strip(l).contains("deliberately long"))
        .unwrap();
    assert_eq!(visible_width(row), 80);
    assert!(strip(row).contains("should fill the row"), "{}", strip(row));
}

#[test]
fn finished_tasks_show_combined_token_usage_per_row() {
    let _g = lock();
    let d = create_with("Investigate flaky test", sub("explore"));
    task_store().update(
        d,
        TaskPatch {
            status: Some(TaskStatus::Done),
            usage: Some(usage(9000.0, 1100.0, 0.02)),
            ..Default::default()
        },
    );
    let lines = render(&mut TaskPanelComponent::new(), 120);
    let row = find(&lines, "Investigate flaky test");
    assert!(row.contains("10k"));
    assert!(!row.contains('↑') && !row.contains('↓'));
}

#[test]
fn a_finished_task_says_its_count_is_tokens() {
    let _g = lock();
    let d = create_with("Tiny lookup", sub("explore"));
    task_store().update(
        d,
        TaskPatch {
            status: Some(TaskStatus::Done),
            usage: Some(usage(100.0, 20.0, 0.0)),
            ..Default::default()
        },
    );
    let lines = render(&mut TaskPanelComponent::new(), 120);
    let row = find(&lines, "Tiny lookup");
    assert!(row.contains("120 tokens"), "{row}");
}

#[test]
fn the_pane_carries_no_turn_token_or_cost_delta() {
    let _g = lock();
    let a = create_with("Explore module", sub("explore"));
    task_store().update(
        a,
        TaskPatch {
            status: Some(TaskStatus::Done),
            usage: Some(usage(3000.0, 500.0, 0.01)),
            ..Default::default()
        },
    );
    let b = create("Run tests");
    set(b, TaskStatus::Done);
    let header = strip(&render(&mut TaskPanelComponent::new(), 120).join("\n"));
    for s in ["turn", "$", "WORKING", "REVIEWED"] {
        assert!(!header.contains(s), "{s}");
    }
}

fn plan_and_done_run() -> u64 {
    let plan = create("Plain work");
    set(plan, TaskStatus::InProgress);
    let s = create_with("trace the auth flow", sub("explore"));
    set(s, TaskStatus::Done);
    plan
}

#[test]
fn each_tab_carries_its_own_lens_count() {
    let _g = lock();
    plan_and_done_run();
    let header = strip(&render(&mut TaskPanelComponent::new(), 120).join("\n"));
    assert!(header.contains("Plan 0/1"), "{header}");
    assert!(header.contains("Agents 1/1"), "{header}");
}

#[test]
fn the_shown_tab_is_bracketed_so_the_strip_reads_without_colour() {
    let _g = lock();
    plan_and_done_run();
    let mut panel = TaskPanelComponent::new();
    // The strip is the console's second row: the shown tab in brackets.
    let strip_row = strip(&render(&mut panel, 120)[1]);
    assert!(strip_row.contains("[Plan 0/1]"), "{strip_row}");
    assert!(!strip_row.contains("[Agents"), "{strip_row}");
    panel.set_view(TaskPanelView::Subagents);
    let strip_row = strip(&render(&mut panel, 120)[1]);
    assert!(strip_row.contains("[Agents 1/1]"), "{strip_row}");
}

#[test]
fn the_console_keeps_the_tab_strip_and_drops_the_cycle_hint_when_narrow() {
    let _g = lock();
    let plan = create("Explore module");
    set(plan, TaskStatus::Done);
    let s = create_with("trace the auth flow", sub("explore"));
    set(s, TaskStatus::Done);
    let lines = render(&mut TaskPanelComponent::new(), 20);
    assert!(lines.iter().all(|l| visible_width(l) <= 20), "{lines:#?}");
    assert!(!strip(&lines.join("\n")).contains("cycle"));
    assert!(strip(&lines[1]).contains("Plan"));
}

#[test]
fn reset_clears_finished_tasks_and_restarts_numbering() {
    let _g = lock();
    let d = create("Finished work");
    set(d, TaskStatus::Done);
    let f = create("Broken build");
    set(f, TaskStatus::Failed);
    task_store().reset();
    let mut panel = TaskPanelComponent::new();
    assert!(render(&mut panel, 120).is_empty());
    let next = create("Fresh task");
    assert_eq!(next, 1);
    assert!(plain(&mut panel, 120).join("\n").contains("Fresh task"));
}

#[test]
fn reset_keeps_active_tasks_and_their_numbering() {
    let _g = lock();
    let running = create_with("Still running", mode("explore"));
    set(running, TaskStatus::InProgress);
    let d = create("Finished work");
    set(d, TaskStatus::Done);
    task_store().reset();
    let mut panel = TaskPanelComponent::new();
    let text = render(&mut panel, 120).join("\n");
    assert!(text.contains("Still running") && !text.contains("Finished work"));
    set(running, TaskStatus::Done);
    assert!(render(&mut panel, 120).join("\n").contains("Still running"));
    let next = create("Next task");
    assert!(next != 1 && next > running);
}

#[test]
fn shows_all_tasks_without_a_line_limit() {
    let _g = lock();
    for i in 0..6 {
        let t = create(&format!("Task {i}"));
        set(t, TaskStatus::InProgress);
    }
    let lines = render(&mut TaskPanelComponent::new(), 120);
    assert_eq!(lines.len(), 9, "frame, tab strip, six rows, foot");
    let text = lines.join("\n");
    assert!(text.contains("Task 0") && text.contains("Task 5"));
}

#[test]
fn the_strip_lists_the_tabs_that_have_rows() {
    let _g = lock();
    let t = create("Plain work");
    set(t, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    let one = strip(&render(&mut panel, 120)[1]);
    assert!(one.contains("Plan 0/1") && !one.contains("Agents"), "{one}");
    assert!(strip(&render(&mut panel, 120)[0]).contains("/tasks · ctrl+g"));

    create_with("find the bug", sub("explore"));
    let two = strip(&render(&mut panel, 120)[1]);
    assert!(
        two.contains("Plan 0/1") && two.contains("Agents 0/1"),
        "{two}"
    );
    assert!(!two.contains("teams"));
}

#[test]
fn cycle_view_advances_through_the_lenses_that_have_content() {
    let _g = lock();
    create("plan the work");
    create_with("find the bug", sub("explore"));
    let mut panel = TaskPanelComponent::new();
    assert_eq!(panel.get_view(), TaskPanelView::Plan);
    assert_eq!(panel.cycle_view(true), TaskPanelView::Subagents);
    assert_eq!(panel.cycle_view(true), TaskPanelView::Plan);
}

#[test]
fn cycle_view_skips_empty_lenses() {
    let _g = lock();
    let t = create("Plain work");
    set(t, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    assert_eq!(panel.cycle_view(true), TaskPanelView::Plan);
    create_with("find the bug", sub("explore"));
    assert_eq!(panel.cycle_view(true), TaskPanelView::Subagents);
    assert_eq!(panel.cycle_view(true), TaskPanelView::Plan);
}

#[test]
fn a_selected_lens_that_empties_falls_back_to_flat_rendering() {
    let _g = lock();
    let t = create("Plain work");
    set(t, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    let lines = plain(&mut panel, 120);
    assert!(lines.join("\n").contains("Plain work"));
    assert_eq!(lines.len(), 4, "frame, tab strip, one row, foot");
    assert_eq!(panel.cycle_view(true), TaskPanelView::Plan);
}

#[test]
fn subagents_view_renders_a_recursive_task_tree() {
    let _g = lock();
    let explore = create_with("Explore module", sub("explore"));
    set(explore, TaskStatus::Done);
    let review = create_with(
        "Review findings",
        CreateTaskOptions {
            parent_task_id: Some(explore),
            ..sub("review")
        },
    );
    set(review, TaskStatus::Done);
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    let lines = plain(&mut panel, 120);
    assert_eq!(lines.len(), 5, "frame, tab strip, two rows, foot");
    let explore_row = find(&lines, "Explore module");
    let review_row = find(&lines, "Review findings");
    assert!(!explore_row.contains("└─") && !explore_row.contains("├─"));
    assert!(explore_row.contains("[explore]"));
    assert!(review_row.contains("└─") && review_row.contains("[review]"));
    assert!(review_row.find("Review findings") > explore_row.find("Explore module"));
}

#[test]
fn subagents_view_draws_branch_connectors_for_siblings() {
    let _g = lock();
    let explore = create_with("Explore module", sub("explore"));
    set(explore, TaskStatus::Done);
    let child = |title: &str, m: &str| {
        let id = create_with(
            title,
            CreateTaskOptions {
                parent_task_id: Some(explore),
                ..sub(m)
            },
        );
        set(id, TaskStatus::Done);
    };
    child("First child", "edit");
    child("Second child", "review");
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    let lines = plain(&mut panel, 120);
    assert!(find(&lines, "First child").contains("├─"));
    assert!(find(&lines, "Second child").contains("└─"));
}

#[test]
fn subagents_tree_with_only_top_level_tasks_renders_flat() {
    let _g = lock();
    let a = create_with("Explore A", sub("explore"));
    let b = create_with("Explore B", sub("explore"));
    set(a, TaskStatus::InProgress);
    set(b, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    let lines = plain(&mut panel, 120);
    assert_eq!(lines.len(), 5, "frame, tab strip, two rows, foot");
    let (ra, rb) = (find(&lines, "Explore A"), find(&lines, "Explore B"));
    for row in [ra, rb] {
        assert!(!row.contains("└─") && !row.contains("├─"));
    }
    assert_eq!(ra.find("Explore A"), rb.find("Explore B"));
}

fn plan_sub_mcp(sub_status: TaskStatus, mcp_status: TaskStatus) {
    let plan = create("Write the plan");
    set(plan, TaskStatus::InProgress);
    let s = create_with("Explore the API", sub("explore"));
    set(s, sub_status);
    let m = create_with("Fetch the spec", mcp(Some("web")));
    set(m, mcp_status);
}

#[test]
fn the_plan_agents_and_mcp_tabs_split_work_by_kind() {
    let _g = lock();
    plan_sub_mcp(TaskStatus::InProgress, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Plan);
    let flat = plain(&mut panel, 120).join("\n");
    assert!(flat.contains("Write the plan"));
    assert!(!flat.contains("Explore the API") && !flat.contains("Fetch the spec"));
    panel.set_view(TaskPanelView::Subagents);
    let sa = plain(&mut panel, 120).join("\n");
    assert!(!sa.contains("Write the plan"));
    assert!(sa.contains("Explore the API") && !sa.contains("Fetch the spec"));
    panel.set_view(TaskPanelView::Mcp);
    let mc = plain(&mut panel, 120).join("\n");
    assert!(!mc.contains("Explore the API") && mc.contains("Fetch the spec"));
}

#[test]
fn each_tabs_count_is_scoped_to_its_own_kind() {
    let _g = lock();
    plan_sub_mcp(TaskStatus::InProgress, TaskStatus::Done);
    let mut panel = TaskPanelComponent::new();
    for view in TaskPanelView::ALL {
        panel.set_view(view);
        let header = strip(&render(&mut panel, 120).join("\n"));
        assert!(header.contains("Plan 0/1"), "{header}");
        // A running run reads as a count of what runs, not done/total.
        assert!(header.contains("Agents 1 running"), "{header}");
        assert!(header.contains("MCP 1/1"), "{header}");
    }
}

/// A panel whose tabs carry double-digit counts: Plan 3/12, twelve running
/// agents, two running shell jobs, one running schedule, three running calls.
fn crowded_tabs() {
    for i in 0..12 {
        let item = create(&format!("Step {i:02}"));
        if i < 3 {
            set(item, TaskStatus::Done);
        }
    }
    for i in 0..12 {
        let run = create_with(&format!("scan {i:02}"), sub("explore"));
        set(run, TaskStatus::InProgress);
    }
    for i in 0..2 {
        let job = create_with(&format!("sleep {i}"), shell());
        set(job, TaskStatus::InProgress);
    }
    let schedule = create_with(
        "nightly sweep",
        CreateTaskOptions {
            source: Some(TaskSource::Schedule),
            ..Default::default()
        },
    );
    set(schedule, TaskStatus::InProgress);
    for i in 0..3 {
        let call = create_with(&format!("fetch {i}"), mcp(Some("web")));
        set(call, TaskStatus::InProgress);
    }
}

#[test]
fn the_shown_tab_keeps_its_brackets_at_80_columns_with_double_digit_counts() {
    let _g = lock();
    crowded_tabs();
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Mcp);
    let lines = plain(&mut panel, 80);
    assert!(lines.iter().all(|l| visible_width(l) <= 80), "{lines:#?}");
    // The full strip is 80 cells, past the body row's 78. The counts of the
    // tabs not shown go first; the shown tab keeps its count and brackets.
    let strip = &lines[1];
    assert!(strip.contains("[MCP 3 running]"), "{strip}");
    assert!(
        !strip.contains("12") && !strip.contains("Agents 12"),
        "{strip}"
    );
    for label in ["Plan", "Agents", "Shell", "Schedule"] {
        assert!(strip.contains(label), "{strip}");
    }
    // Focused, the strip and the rows still fit and the bracket stays whole.
    let mut keyboard = focused();
    keyboard.set_view(TaskPanelView::Mcp);
    let lines = plain(&mut keyboard, 80);
    assert!(lines[1].contains("[MCP 3 running]"), "{lines:#?}");
    assert!(lines.iter().all(|l| visible_width(l) <= 80), "{lines:#?}");
}

#[test]
fn the_tab_strip_drops_counts_then_labels_before_it_cuts_the_shown_tab() {
    let _g = lock();
    crowded_tabs();
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Mcp);
    // At 40 columns the body row is 38 cells: the counts (44 cells) do not fit,
    // the labels (34 cells) do.
    let strip = plain(&mut panel, 40)[1].clone();
    assert!(strip.contains("Schedule [MCP]"), "{strip}");
    assert!(!strip.contains("running"), "{strip}");
    // Only the shown tab fits 12 columns: its brackets stay whole.
    let strip = plain(&mut panel, 12)[1].clone();
    assert!(strip.contains("[MCP]"), "{strip}");
    assert!(!strip.contains("Plan"), "{strip}");
    assert!(visible_width(&strip) <= 12, "{strip}");
}

#[test]
fn an_armed_shell_cancel_says_stop() {
    let _g = lock();
    let job = create_with("sleep 60", shell());
    task_store().update(
        job,
        TaskPatch {
            status: Some(TaskStatus::InProgress),
            action: Some(Some(RowAction::Abort)),
            ..Default::default()
        },
    );
    let mut panel = focused();
    panel.set_view(TaskPanelView::Shell);
    render(&mut panel, 120);
    press(&mut panel, "x");
    let lines = plain(&mut panel, 120);
    assert!(
        lines.iter().any(|l| l.contains("press x again to stop")),
        "{lines:#?}"
    );
    assert!(!lines.iter().any(|l| l.contains("to cancel")), "{lines:#?}");
}

#[test]
fn an_armed_cancel_clears_when_its_run_finishes() {
    let _g = lock();
    let run = run_with_id("trace the auth flow", "run-1");
    let mut panel = focused();
    panel.set_view(TaskPanelView::Subagents);
    render(&mut panel, 120);
    press(&mut panel, "x");
    assert!(plain(&mut panel, 120)
        .join("\n")
        .contains("press x again to cancel"));
    set(run, TaskStatus::Done);
    let text = plain(&mut panel, 120).join("\n");
    assert!(!text.contains("press x again"), "{text}");
    assert!(!text.contains("· x "), "{text}");
}

#[test]
fn a_stale_arm_does_not_swallow_escape() {
    let _g = lock();
    let run = run_with_id("trace the auth flow", "run-1");
    let mut panel = focused();
    panel.set_view(TaskPanelView::Subagents);
    render(&mut panel, 120);
    press(&mut panel, "x");
    // The run ends while armed: the arm is stale, so the first Esc leaves.
    set(run, TaskStatus::Done);
    press(&mut panel, ESC);
    assert_eq!(panel.take_events(), vec![PanelEvent::Exit]);
}

#[test]
fn a_narrow_strip_keeps_a_lone_shown_tab_whole_without_its_indent() {
    let _g = lock();
    create("a plan item");
    let job = create_with(
        "nightly sweep",
        CreateTaskOptions {
            source: Some(TaskSource::Schedule),
            ..Default::default()
        },
    );
    set(job, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Schedule);
    // The body row is 10 cells at width 12: `  [Schedule]` (12) does not fit,
    // `[Schedule]` (10) does.
    let strip = plain(&mut panel, 12)[1].clone();
    assert!(strip.contains("[Schedule]"), "{strip}");
    assert!(!strip.contains("Plan"), "{strip}");
    assert!(visible_width(&strip) <= 12, "{strip}");
}

#[test]
fn the_plan_tab_counts_its_items_not_the_runs_nested_under_them() {
    let _g = lock();
    let a = create("Read the spec");
    let b = create("Write the port");
    set(a, TaskStatus::Done);
    // Two runs under the first item, one under the second: still two items.
    for (title, item) in [("scan A", a), ("scan B", a), ("scan C", b)] {
        let run = create_with(
            title,
            CreateTaskOptions {
                linked_task_id: Some(item),
                ..sub("explore")
            },
        );
        set(run, TaskStatus::Done);
    }
    let mut panel = TaskPanelComponent::new();
    let lines = plain(&mut panel, 120);
    assert!(lines[0].contains("FLIGHT PLAN  1/2"), "{lines:#?}");
    assert!(lines[1].contains("[Plan 1/2]"), "{lines:#?}");
    assert!(lines[1].contains("Agents 3/3"), "{lines:#?}");
}

#[test]
fn live_tabs_count_what_runs_and_the_plan_stays_done_of_total() {
    let _g = lock();
    let plan = create("Write the port");
    set(plan, TaskStatus::InProgress);
    let first = create_with("explore a", sub("explore"));
    let second = create_with("explore b", sub("explore"));
    set(first, TaskStatus::InProgress);
    set(second, TaskStatus::InProgress);
    let job = create_with("sleep 60", shell());
    set(job, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    let strip_row = strip(&render(&mut panel, 120)[1]);
    assert!(strip_row.contains("[Plan 0/1]"), "{strip_row}");
    assert!(strip_row.contains("Agents 2 running"), "{strip_row}");
    assert!(strip_row.contains("Shell 1 running"), "{strip_row}");
    // The shown tab is bracketed, running count and all.
    panel.set_view(TaskPanelView::Shell);
    let strip_row = strip(&render(&mut panel, 120)[1]);
    assert!(strip_row.contains("[Shell 1 running]"), "{strip_row}");
    // Once nothing runs, the tab reads done/total again.
    set(first, TaskStatus::Done);
    set(second, TaskStatus::Done);
    set(job, TaskStatus::Done);
    let strip_row = strip(&render(&mut panel, 120)[1]);
    assert!(strip_row.contains("[Shell 1/1]"), "{strip_row}");
    assert!(strip_row.contains("Agents 2/2"), "{strip_row}");
}

#[test]
fn an_empty_flat_lens_falls_through_to_the_subagents_tree() {
    let _g = lock();
    let s = create_with("Explore the API", sub("explore"));
    set(s, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    assert_eq!(panel.get_view(), TaskPanelView::Plan);
    assert!(plain(&mut panel, 120)
        .join("\n")
        .contains("Explore the API"));
}

#[test]
fn subagents_tree_shows_delegated_work_and_excludes_plan_owned_tasks() {
    let _g = lock();
    let w = create_with(
        "worker task",
        CreateTaskOptions {
            agent: Some("worker".into()),
            ..sub("build")
        },
    );
    set(w, TaskStatus::InProgress);
    let p = create_with("planner task", owned("planner"));
    set(p, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    let text = plain(&mut panel, 120).join("\n");
    assert!(!text.contains("planner task"));
    assert!(text.contains("worker task"));
}

#[test]
fn subagents_tree_renders_delegated_roots_without_a_roster() {
    let _g = lock();
    let s = create_with("find the bug", sub("explore"));
    set(s, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    panel.set_view(TaskPanelView::Subagents);
    let lines = plain(&mut panel, 120);
    assert_eq!(lines.len(), 4, "frame, tab strip, one row, foot");
    for s in ["find the bug", "[explore]", "◈"] {
        assert!(lines[2].contains(s), "{s}");
    }
}

#[test]
fn reset_drops_the_roster_with_the_tasks_but_keeps_owners_of_live_tasks() {
    let _g = lock();
    let store = task_store();
    for (id, state) in [
        ("explore", TaskAgentState::Running),
        ("review", TaskAgentState::Done),
    ] {
        store.upsert_agent(
            id,
            id,
            TaskAgentKind::Subagent,
            TaskAgentPatch {
                role: Some("subagent".into()),
                state: Some(state),
                ..Default::default()
            },
        );
    }
    let live = create_with(
        "Still running",
        CreateTaskOptions {
            source: Some(TaskSource::Subagent),
            agent: Some("explore".into()),
            ..Default::default()
        },
    );
    set(live, TaskStatus::InProgress);
    let done = create_with(
        "Finished work",
        CreateTaskOptions {
            source: Some(TaskSource::Subagent),
            agent: Some("review".into()),
            ..Default::default()
        },
    );
    set(done, TaskStatus::Done);
    store.reset();
    let ids: Vec<String> = store.agents().into_iter().map(|a| a.id).collect();
    assert_eq!(ids, vec!["explore"]);
    set(live, TaskStatus::Done);
    store.reset();
    assert!(store.agents().is_empty());
}

#[test]
fn add_agent_stats_accumulates_usage_across_dispatches() {
    let _g = lock();
    let store = task_store();
    store.upsert_agent(
        "explore",
        "explore",
        TaskAgentKind::Subagent,
        TaskAgentPatch::default(),
    );
    store.add_agent_stats(
        "explore",
        AgentStats {
            input: 1000.0,
            output: 200.0,
            cost: 0.01,
        },
    );
    store.add_agent_stats(
        "explore",
        AgentStats {
            input: 500.0,
            output: 100.0,
            cost: 0.005,
        },
    );
    let stats = store.agents()[0].stats.unwrap();
    assert_eq!((stats.input, stats.output), (1500.0, 300.0));
    assert!((stats.cost - 0.015).abs() < 1e-12);
    store.upsert_agent(
        "explore",
        "explore",
        TaskAgentKind::Subagent,
        TaskAgentPatch {
            state: Some(TaskAgentState::Running),
            ..Default::default()
        },
    );
    assert_eq!(store.agents()[0].stats.unwrap().input, 1500.0);
}

#[test]
fn reset_preserves_main_agents_with_stats_but_drops_settled_subagent_runs() {
    let _g = lock();
    let store = task_store();
    let dummy = create("dummy");
    set(dummy, TaskStatus::Done);
    store.upsert_agent(
        "main-ag",
        "planner",
        TaskAgentKind::Main,
        TaskAgentPatch::default(),
    );
    store.add_agent_stats(
        "main-ag",
        AgentStats {
            input: 100.0,
            output: 50.0,
            cost: 0.001,
        },
    );
    store.upsert_agent(
        "run-1",
        "explore#1",
        TaskAgentKind::Subagent,
        TaskAgentPatch {
            state: Some(TaskAgentState::Done),
            ..Default::default()
        },
    );
    store.add_agent_stats(
        "run-1",
        AgentStats {
            input: 200.0,
            output: 20.0,
            cost: 0.002,
        },
    );
    store.reset();
    assert!(store.agents().iter().any(|a| a.id == "main-ag"));
    assert!(!store.agents().iter().any(|a| a.id == "run-1"));

    store.upsert_agent(
        "zero-ag",
        "zero",
        TaskAgentKind::Main,
        TaskAgentPatch::default(),
    );
    let dummy2 = create("dummy2");
    set(dummy2, TaskStatus::Done);
    store.reset();
    assert!(!store.agents().iter().any(|a| a.id == "zero-ag"));
    assert!(store.agents().iter().any(|a| a.id == "main-ag"));
}

/// The live row explains the run: which attempt, which model, how long left.
/// Before this, a slow run said `⋯ read · 4:12` and nothing about why it was
/// slow or which model it had actually landed on.
#[test]
fn a_running_row_shows_attempt_model_and_the_deadline_counting_down() {
    let _g = lock();
    let now = hoocode_code_task_store::now_ms();
    let run = create_with("map the repo", sub("explore"));
    set(run, TaskStatus::InProgress);
    task_store().upsert_agent(
        "subagent",
        "subagent",
        TaskAgentKind::Subagent,
        TaskAgentPatch {
            activity: Some("Read".into()),
            attempt: Some(2),
            model: Some("mock/pinned".into()),
            deadline_at: Some(now + 192_000),
            ..Default::default()
        },
    );
    let lines = render(&mut TaskPanelComponent::new(), 160);
    let row = find(&lines, "map the repo");
    assert!(row.contains("attempt 2"), "{row:?}");
    assert!(row.contains("mock/pinned"), "{row:?}");
    assert!(
        row.contains("left"),
        "the deadline should count down: {row:?}"
    );
}

/// A finished row says how it ended and why, which a stopwatch cannot: a run
/// cut short and a run that failed used to look the same.
#[test]
fn a_finished_row_shows_the_outcome_and_its_cause() {
    let _g = lock();
    let run = create_with("review the diff", sub("code-review"));
    set(run, TaskStatus::InProgress);
    task_store().upsert_agent(
        "subagent",
        "subagent",
        TaskAgentKind::Subagent,
        TaskAgentPatch {
            activity: Some(String::new()),
            outcome: Some("partial".into()),
            confidence: Some(0.6),
            cause: Some("Ran out of time before completing.".into()),
            ..Default::default()
        },
    );
    let lines = render(&mut TaskPanelComponent::new(), 160);
    let row = find(&lines, "review the diff");
    assert!(row.contains("partial"), "{row:?}");
    assert!(row.contains("0.6"), "the child's confidence: {row:?}");
    assert!(row.contains("Ran out of time"), "{row:?}");
}

// ---- focus, tabs, the pick, and the keys ----

use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_widgets::task_panel::PanelEvent;

const TAB: &str = "\t";
const SHIFT_TAB: &str = "\x1b[Z";
const UP: &str = "\x1b[A";
const DOWN: &str = "\x1b[B";
const ENTER: &str = "\r";
const ESC: &str = "\x1b";
const CTRL_G: &str = "\x07";
const RIGHT: &str = "\x1b[C";
const LEFT: &str = "\x1b[D";

/// Installs the app's bindings for this thread, then focuses a panel.
fn focused() -> TaskPanelComponent {
    AppKeybindingsManager::default().install();
    let mut panel = TaskPanelComponent::new();
    panel.set_focused(true);
    panel
}

fn press(panel: &mut TaskPanelComponent, data: &str) {
    panel.handle_input(data);
}

/// A running subagent run whose pool id is `run_id`.
fn run_with_id(title: &str, run_id: &str) -> u64 {
    let id = create_with(
        title,
        CreateTaskOptions {
            agent: Some(run_id.into()),
            ..sub("explore")
        },
    );
    set(id, TaskStatus::InProgress);
    id
}

#[test]
fn the_panel_is_focusable_only_when_it_has_rows() {
    let _g = lock();
    AppKeybindingsManager::default().install();
    let mut panel = TaskPanelComponent::new();
    assert!(panel.is_focusable());
    assert!(!panel.can_focus());
    create("one row");
    assert!(panel.can_focus());
}

#[test]
fn left_right_and_tab_switch_the_tab_while_the_panel_has_the_keyboard() {
    let _g = lock();
    plan_sub_mcp(TaskStatus::InProgress, TaskStatus::InProgress);
    let mut panel = focused();
    render(&mut panel, 120);
    assert_eq!(panel.get_view(), TaskPanelView::Plan);
    press(&mut panel, RIGHT);
    assert_eq!(panel.get_view(), TaskPanelView::Subagents);
    press(&mut panel, TAB);
    assert_eq!(panel.get_view(), TaskPanelView::Mcp);
    press(&mut panel, LEFT);
    assert_eq!(panel.get_view(), TaskPanelView::Subagents);
    press(&mut panel, SHIFT_TAB);
    assert_eq!(panel.get_view(), TaskPanelView::Plan);
    assert!(panel.take_events().is_empty());
}

#[test]
fn up_and_down_pick_rows_and_clamp_at_the_top() {
    let _g = lock();
    let a = create("row a");
    let b = create("row b");
    let c = create("row c");
    let mut panel = focused();
    render(&mut panel, 120);
    assert_eq!(panel.picked(), Some((TaskPanelView::Plan, a)));
    press(&mut panel, DOWN);
    assert_eq!(panel.picked(), Some((TaskPanelView::Plan, b)));
    press(&mut panel, DOWN);
    assert_eq!(panel.picked(), Some((TaskPanelView::Plan, c)));
    press(&mut panel, UP);
    assert_eq!(panel.picked(), Some((TaskPanelView::Plan, b)));
    press(&mut panel, UP);
    press(&mut panel, UP);
    assert_eq!(
        panel.picked(),
        Some((TaskPanelView::Plan, a)),
        "clamps at the top"
    );
    assert!(panel.take_events().is_empty());
}

#[test]
fn down_past_the_last_row_returns_to_the_prompt() {
    let _g = lock();
    create("row a");
    create("row b");
    let c = create("row c");
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, DOWN);
    press(&mut panel, DOWN);
    assert_eq!(panel.picked(), Some((TaskPanelView::Plan, c)));
    assert!(
        panel.take_events().is_empty(),
        "the last row is not past it"
    );
    press(&mut panel, DOWN);
    assert_eq!(panel.take_events(), vec![PanelEvent::Exit]);
}

#[test]
fn up_from_the_prompt_enters_the_panel_on_its_bottom_row() {
    let _g = lock();
    create("row a");
    create("row b");
    let c = create("row c");
    let mut panel = TaskPanelComponent::new();
    assert!(panel.enter_from_below());
    panel.set_focused(true);
    render(&mut panel, 120);
    assert_eq!(panel.picked(), Some((TaskPanelView::Plan, c)));
}

#[test]
fn up_from_the_prompt_does_not_take_a_hidden_or_empty_panel() {
    let _g = lock();
    let mut panel = TaskPanelComponent::new();
    assert!(!panel.enter_from_below(), "no rows");
    create("row a");
    panel.set_radar(true);
    assert!(!panel.enter_from_below(), "hidden in radar");
    panel.set_radar(false);
    assert!(panel.enter_from_below());
}

#[test]
fn the_pick_follows_its_row_across_a_rebuild() {
    let _g = lock();
    let a = create("row a");
    let b = create("row b");
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, DOWN);
    set(a, TaskStatus::Done);
    render(&mut panel, 120);
    assert_eq!(panel.picked(), Some((TaskPanelView::Plan, b)));
}

#[test]
fn enter_queues_an_open_for_the_picked_row() {
    let _g = lock();
    create("row a");
    let b = create("row b");
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, DOWN);
    press(&mut panel, ENTER);
    assert_eq!(
        panel.take_events(),
        vec![PanelEvent::Open {
            view: TaskPanelView::Plan,
            task_id: b,
        }]
    );
}

#[test]
fn x_arms_then_a_second_x_cancels_a_running_subagent() {
    let _g = lock();
    run_with_id("trace the auth flow", "run-1");
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, "x");
    assert!(panel.take_events().is_empty(), "the first x only arms");
    let armed = plain(&mut panel, 120).join("\n");
    assert!(armed.contains("press x again to cancel"), "{armed}");
    press(&mut panel, "x");
    assert_eq!(
        panel.take_events(),
        vec![PanelEvent::Cancel {
            run_id: "run-1".into()
        }]
    );
}

#[test]
fn any_other_key_disarms_a_pending_cancel() {
    let _g = lock();
    run_with_id("first run", "run-1");
    run_with_id("second run", "run-2");
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, "x");
    press(&mut panel, DOWN);
    assert!(!plain(&mut panel, 120).join("\n").contains("press x again"));
    press(&mut panel, "x");
    assert!(
        panel.take_events().is_empty(),
        "the x on the new row only arms it"
    );
    press(&mut panel, "x");
    assert_eq!(
        panel.take_events(),
        vec![PanelEvent::Cancel {
            run_id: "run-2".into()
        }]
    );
}

#[test]
fn x_does_nothing_on_a_plan_item_or_a_finished_run() {
    let _g = lock();
    let plan = create("a plan item");
    set(plan, TaskStatus::InProgress);
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, "x");
    press(&mut panel, "x");
    assert!(panel.take_events().is_empty());
    assert!(!plain(&mut panel, 120).join("\n").contains("press x again"));

    let finished = run_with_id("done run", "run-9");
    set(finished, TaskStatus::Done);
    let mut panel = focused();
    panel.set_view(TaskPanelView::Subagents);
    render(&mut panel, 120);
    press(&mut panel, "x");
    press(&mut panel, "x");
    assert!(
        panel.take_events().is_empty(),
        "a finished run has nothing to cancel"
    );
}

/// The hint foot of a focused panel showing `view`, with its first row picked.
fn foot_on(view: TaskPanelView) -> String {
    let mut panel = focused();
    panel.set_view(view);
    plain(&mut panel, 120).last().unwrap().clone()
}

#[test]
fn the_agents_hint_names_enter_and_x_only_while_the_run_is_active() {
    let _g = lock();
    let run = run_with_id("Map the auth module", "run-1");
    let foot = foot_on(TaskPanelView::Subagents);
    assert!(foot.contains("· enter open its log"), "{foot}");
    assert!(foot.contains("· x cancel"), "{foot}");
    assert!(!foot.contains("jump") && !foot.contains("drop"), "{foot}");
    set(run, TaskStatus::Done);
    let foot = foot_on(TaskPanelView::Subagents);
    assert!(foot.contains("· enter open its log"), "{foot}");
    assert!(
        !foot.contains("· x "),
        "a finished run has no cancel: {foot}"
    );
}

#[test]
fn the_shell_hint_names_open_output_and_stop_only_while_the_job_runs() {
    let _g = lock();
    let job = task_store().create(
        "sleep 60",
        CreateTaskOptions {
            source: Some(TaskSource::Shell),
            key: Some("shell:job-1".into()),
            ..Default::default()
        },
    );
    task_store().update(
        job.id,
        TaskPatch {
            status: Some(TaskStatus::InProgress),
            action: Some(Some(RowAction::Abort)),
            ..Default::default()
        },
    );
    let foot = foot_on(TaskPanelView::Shell);
    assert!(foot.contains("· enter open output"), "{foot}");
    assert!(foot.contains("· x stop"), "{foot}");
    // A landed job keeps its row (its output still opens) but cannot be stopped.
    task_store().update(
        job.id,
        TaskPatch {
            status: Some(TaskStatus::Done),
            action: Some(None),
            ..Default::default()
        },
    );
    let foot = foot_on(TaskPanelView::Shell);
    assert!(foot.contains("· enter open output"), "{foot}");
    assert!(!foot.contains("· x "), "{foot}");
}

#[test]
fn the_hint_names_no_enter_or_x_where_they_do_nothing() {
    let _g = lock();
    create("a plan item");
    let foot = foot_on(TaskPanelView::Plan);
    assert!(
        !foot.contains("· enter") && !foot.contains("· x "),
        "{foot}"
    );
    assert!(foot.contains("↑↓ move · ←→ tab · esc back"), "{foot}");

    create_with(
        "a job",
        CreateTaskOptions {
            source: Some(TaskSource::Schedule),
            ..Default::default()
        },
    );
    let foot = foot_on(TaskPanelView::Schedule);
    assert!(
        !foot.contains("· enter") && !foot.contains("· x "),
        "{foot}"
    );

    // A finished MCP call has no Abort, so `x` does nothing on it either.
    let call = create_with("fetch the spec", mcp(Some("web")));
    set(call, TaskStatus::Done);
    let foot = foot_on(TaskPanelView::Mcp);
    assert!(
        !foot.contains("· enter") && !foot.contains("· x "),
        "{foot}"
    );
}

#[test]
fn the_hint_names_the_login_row_action() {
    let _g = lock();
    with_action("web", TaskSource::Mcp, RowAction::Login);
    let foot = foot_on(TaskPanelView::Mcp);
    assert!(foot.contains("· enter log in"), "{foot}");
    assert!(!foot.contains("· x "), "{foot}");
}

#[test]
fn the_hint_names_the_running_call_cancel_and_no_enter() {
    let _g = lock();
    let call = with_action("fetch the spec", TaskSource::Mcp, RowAction::Abort);
    set(call, TaskStatus::InProgress);
    let foot = foot_on(TaskPanelView::Mcp);
    assert!(foot.contains("· x cancel"), "{foot}");
    assert!(
        !foot.contains("· enter"),
        "Enter opens nothing on a call: {foot}"
    );
}

#[test]
fn escape_first_disarms_a_pending_cancel_and_then_leaves() {
    let _g = lock();
    run_with_id("trace the auth flow", "run-1");
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, "x");
    press(&mut panel, ESC);
    assert!(panel.take_events().is_empty(), "escape only disarms");
    press(&mut panel, ESC);
    assert_eq!(panel.take_events(), vec![PanelEvent::Exit]);
}

#[test]
fn ctrl_g_hides_the_panel_and_returns_the_keyboard_to_the_prompt() {
    let _g = lock();
    create("row a");
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, CTRL_G);
    assert_eq!(panel.take_events(), vec![PanelEvent::Exit]);
    assert!(render(&mut panel, 120).is_empty(), "the panel is hidden");
}
#[test]
fn losing_focus_clears_a_pending_cancel() {
    let _g = lock();
    run_with_id("trace the auth flow", "run-1");
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, "x");
    panel.set_focused(false);
    panel.set_focused(true);
    render(&mut panel, 120);
    assert!(!plain(&mut panel, 120).join("\n").contains("press x again"));
}

#[test]
fn focus_opens_the_flight_plan_console() {
    let _g = lock();
    create("row a");
    let mut panel = focused();
    let lines = plain(&mut panel, 120);
    // Spec §4 c6: `FLIGHT PLAN  2/5  agents 0 · sats 0` with `/tasks · ctrl+g`
    // on the right edge.
    assert!(
        lines[0].contains("FLIGHT PLAN  0/1  agents 0 · sats 0"),
        "{lines:#?}"
    );
    assert!(lines[0].contains("/tasks · ctrl+g"), "{lines:#?}");
    assert!(
        lines.iter().any(|l| l.contains('›') && l.contains("row a")),
        "{lines:#?}"
    );
    // The tab strip is the second row, the shown tab bracketed.
    assert!(lines[1].contains("[Plan 0/1]"), "{lines:#?}");
    assert!(
        !lines.iter().any(|l| l.contains("agents in flight")),
        "{lines:#?}"
    );
    // A plan item has no Enter and no `x`: the foot names only the moves.
    assert!(
        lines
            .last()
            .unwrap()
            .contains("↑↓ move · ←→ tab · esc back"),
        "{lines:#?}"
    );
}

#[test]
fn unfocused_the_console_draws_the_same_rows_without_the_pick_or_the_hint() {
    let _g = lock();
    create("row a");
    let mut panel = TaskPanelComponent::new();
    let lines = plain(&mut panel, 120);
    assert_eq!(lines.len(), 4, "frame, tab strip, one row, foot");
    assert!(lines[0].contains("FLIGHT PLAN"), "{lines:#?}");
    assert!(lines[2].contains("row a"), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains('›')), "{lines:#?}");
    assert!(!lines.last().unwrap().contains("move"), "{lines:#?}");
}

#[test]
fn the_panel_is_shown_in_peek_and_full_and_hidden_in_radar_by_default() {
    let _g = lock();
    create("row a");
    let mut panel = TaskPanelComponent::new();
    assert!(panel.is_shown());
    assert!(!render(&mut panel, 120).is_empty());
    panel.set_radar(true);
    assert!(!panel.is_shown());
    assert!(render(&mut panel, 120).is_empty());
    panel.set_radar(false);
    assert!(!render(&mut panel, 120).is_empty());
}

#[test]
fn ctrl_g_toggles_the_panel_and_a_view_switch_resets_the_toggle() {
    let _g = lock();
    create("row a");
    let mut panel = TaskPanelComponent::new();
    assert!(!panel.toggle_shown(), "a shown panel is hidden");
    assert!(render(&mut panel, 120).is_empty());
    assert!(panel.toggle_shown(), "and shown again");
    assert!(!render(&mut panel, 120).is_empty());

    // Hide in peek, then switch to radar: radar's default (hidden) wins.
    panel.toggle_shown();
    panel.set_radar(true);
    assert!(!panel.is_shown());
    // Show in radar, then switch back to peek: peek's default (shown) wins.
    assert!(panel.toggle_shown());
    panel.set_radar(false);
    assert!(panel.is_shown());
}

#[test]
fn a_background_agent_run_is_an_agents_row_like_a_shell_job() {
    let _g = lock();
    run_with_id("Map the auth module", "bg-run-1");
    let job = create_with("sleep 60", shell());
    set(job, TaskStatus::InProgress);
    let mut panel = TaskPanelComponent::new();
    let lines = plain(&mut panel, 120);
    assert!(lines[0].contains("agents 1 · sats 1"), "{lines:#?}");
    assert!(
        lines[1].contains("Agents 1 running") && lines[1].contains("Shell 1 running"),
        "{lines:#?}"
    );
    panel.set_view(TaskPanelView::Subagents);
    let agents = plain(&mut panel, 120);
    let row = find(&agents, "Map the auth module");
    assert!(row.contains('◈') && row.contains("[explore]"), "{row}");
}
// ---- Schedule tab, row actions, next_at ----

use hoocode_code_task_store::RowAction;

fn with_action(title: &str, source: TaskSource, action: RowAction) -> u64 {
    let id = create_with(
        title,
        CreateTaskOptions {
            source: Some(source),
            ..Default::default()
        },
    );
    task_store().update(
        id,
        TaskPatch {
            action: Some(Some(action)),
            ..Default::default()
        },
    );
    id
}

#[test]
fn a_schedule_row_gets_its_own_tab_and_shows_when_it_next_fires() {
    let _g = lock();
    let job = create_with(
        "nightly sweep",
        CreateTaskOptions {
            source: Some(TaskSource::Schedule),
            ..Default::default()
        },
    );
    task_store().update(
        job,
        TaskPatch {
            next_at: Some(Some(hoocode_code_task_store::now_ms() + 5 * 60 * 1000)),
            ..Default::default()
        },
    );
    let mut panel = TaskPanelComponent::new();
    let lines = plain(&mut panel, 120);
    assert_eq!(lines.len(), 4, "frame, tab strip, one row, foot");
    assert!(
        lines[2].contains("nightly sweep") && lines[2].contains("next in"),
        "{lines:#?}"
    );
    panel.set_view(TaskPanelView::Schedule);
    assert!(plain(&mut panel, 120).join("\n").contains("nightly sweep"));
    create_with("a call", mcp(Some("web")));
    panel.set_view(TaskPanelView::Mcp);
    assert!(!plain(&mut panel, 120).join("\n").contains("nightly sweep"));
}

#[test]
fn schedule_tab_sits_between_agents_and_mcp() {
    let _g = lock();
    create_with(
        "a job",
        CreateTaskOptions {
            source: Some(TaskSource::Schedule),
            ..Default::default()
        },
    );
    create_with("a call", mcp(Some("web")));
    create_with("a run", sub("explore"));
    let mut panel = focused();
    render(&mut panel, 120);
    assert_eq!(panel.cycle_view(true), TaskPanelView::Subagents);
    assert_eq!(panel.cycle_view(true), TaskPanelView::Schedule);
    assert_eq!(panel.cycle_view(true), TaskPanelView::Mcp);
}

#[test]
fn enter_on_a_login_row_performs_the_login_instead_of_opening_it() {
    let _g = lock();
    let server = with_action("web", TaskSource::Mcp, RowAction::Login);
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, ENTER);
    assert_eq!(
        panel.take_events(),
        vec![PanelEvent::Perform {
            task_id: server,
            action: RowAction::Login,
        }]
    );
}

#[test]
fn x_arms_then_performs_abort_on_a_call_row() {
    let _g = lock();
    let call = with_action("fetch the spec", TaskSource::Mcp, RowAction::Abort);
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, "x");
    assert!(panel.take_events().is_empty(), "the first x only arms");
    press(&mut panel, "x");
    assert_eq!(
        panel.take_events(),
        vec![PanelEvent::Perform {
            task_id: call,
            action: RowAction::Abort,
        }]
    );
}

#[test]
fn enter_on_an_abort_row_opens_it_and_leaves_the_abort_to_x() {
    let _g = lock();
    let call = with_action("fetch the spec", TaskSource::Mcp, RowAction::Abort);
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, ENTER);
    assert_eq!(
        panel.take_events(),
        vec![PanelEvent::Open {
            view: TaskPanelView::Mcp,
            task_id: call,
        }]
    );
}

#[test]
fn the_tick_runs_while_any_row_in_any_tab_is_running() {
    let _g = lock();
    let plan = create("a plan item");
    let call = create_with("a call", mcp(Some("web")));
    let mut panel = TaskPanelComponent::new();
    render(&mut panel, 120);
    assert!(!panel.ticking());
    set(call, TaskStatus::InProgress);
    render(&mut panel, 120);
    assert!(panel.ticking(), "a running MCP call on another tab ticks");
    set(call, TaskStatus::Done);
    set(plan, TaskStatus::InProgress);
    render(&mut panel, 120);
    assert!(panel.ticking());
    set(plan, TaskStatus::Done);
    render(&mut panel, 120);
    assert!(!panel.ticking());
}

// ---- Mission Control chrome (spec §4 c6/r6) ----

fn shell() -> CreateTaskOptions {
    CreateTaskOptions {
        source: Some(TaskSource::Shell),
        ..Default::default()
    }
}

#[test]
fn focused_title_counts_live_agents_and_satellites() {
    let _g = lock();
    let a = create("row a");
    let run = create_with("scan the repo", sub("explore"));
    set(run, TaskStatus::InProgress);
    let job = create_with("sleep 60", shell());
    set(job, TaskStatus::InProgress);
    let mut panel = focused();
    let lines = plain(&mut panel, 120);
    assert!(
        lines[0].contains("FLIGHT PLAN  0/1  agents 1 · sats 1"),
        "{lines:#?}"
    );
    // Landed work drops out of the counts.
    set(a, TaskStatus::Done);
    set(run, TaskStatus::Done);
    set(job, TaskStatus::Done);
    let lines = plain(&mut panel, 120);
    assert!(
        lines[0].contains("FLIGHT PLAN  1/1  agents 0 · sats 0"),
        "{lines:#?}"
    );
}

#[test]
fn focused_rows_use_mission_circles_and_one_status_glyph_per_shell_job() {
    let _g = lock();
    let done = create("read it");
    set(done, TaskStatus::Done);
    let failed = create("build it");
    set(failed, TaskStatus::Failed);
    create_with("sleep 60", shell());
    let mut panel = focused();
    panel.set_view(TaskPanelView::Shell);
    let shell_lines = plain(&mut panel, 120);
    // One status glyph, then the title; the satellite mark sits in the right detail.
    let job_row = find(&shell_lines, "sleep 60");
    assert!(job_row.contains("○ sleep 60"), "{job_row}");
    assert!(!job_row.contains('⌁'), "{job_row}");
    panel.set_view(TaskPanelView::Plan);
    let lines = plain(&mut panel, 120);
    assert!(
        lines
            .iter()
            .any(|l| l.contains('●') && l.contains("read it")),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains('⊗') && l.contains("build it")),
        "{lines:#?}"
    );
}

#[test]
fn running_shell_rows_carry_the_satellite_detail() {
    // Spec §4 r9/c7: a running job reads `⌁ sat job-1 · in orbit 4s` on
    // the right, like the radar row.
    let _g = lock();
    let job = task_store().create(
        "sleep 60",
        CreateTaskOptions {
            source: Some(TaskSource::Shell),
            key: Some("shell:job-1".into()),
            ..Default::default()
        },
    );
    task_store().update(
        job.id,
        TaskPatch {
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        },
    );
    let mut panel = focused();
    panel.set_view(TaskPanelView::Shell);
    let lines = plain(&mut panel, 120);
    assert!(
        lines.iter().any(|l| l.contains("⌁ sat job-1 · in orbit")),
        "{lines:#?}"
    );
}

#[test]
fn the_console_shows_at_most_six_rows_and_says_how_many_are_hidden() {
    let _g = lock();
    for i in 0..12 {
        create(&format!("Step {i:02}"));
    }
    let mut panel = TaskPanelComponent::new();
    let lines = plain(&mut panel, 120);
    // Frame, tab strip, six rows, the `N more` row, foot.
    assert_eq!(lines.len(), 10, "{lines:#?}");
    assert!(lines[2].contains("Step 00") && lines[7].contains("Step 05"));
    assert!(lines[8].contains("6 more"), "{lines:#?}");
}

#[test]
fn a_short_terminal_shows_a_quarter_of_its_rows() {
    let _g = lock();
    for i in 0..12 {
        create(&format!("Step {i:02}"));
    }
    let mut panel = TaskPanelComponent::new();
    panel.set_screen_rows(12);
    let lines = plain(&mut panel, 120);
    // Frame, tab strip, three rows, the `N more` row, foot.
    assert_eq!(lines.len(), 7, "{lines:#?}");
    assert!(lines[5].contains("9 more"), "{lines:#?}");
}

#[test]
fn the_window_follows_the_pick_and_says_what_is_hidden() {
    let _g = lock();
    for i in 0..12 {
        create(&format!("Step {i:02}"));
    }
    let mut panel = focused();
    render(&mut panel, 120);
    for _ in 0..11 {
        press(&mut panel, DOWN);
    }
    let lines = plain(&mut panel, 120);
    assert!(
        lines
            .iter()
            .any(|l| l.contains('›') && l.contains("Step 11")),
        "{lines:#?}"
    );
    assert!(lines.iter().any(|l| l.contains("Step 06")), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("Step 05")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("6 more")), "{lines:#?}");
}

#[test]
fn left_and_right_land_the_pick_on_the_new_tabs_last_row() {
    let _g = lock();
    create("a plan item");
    create_with("run a", sub("explore"));
    create_with("run b", sub("explore"));
    let last = create_with("run c", sub("explore"));
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, RIGHT);
    assert_eq!(panel.picked(), Some((TaskPanelView::Subagents, last)));
    assert!(
        panel.take_events().is_empty(),
        "the last row is not past it"
    );
    press(&mut panel, DOWN);
    assert_eq!(panel.take_events(), vec![PanelEvent::Exit]);
}

#[test]
fn a_queued_exit_can_be_taken_back_before_the_next_key() {
    let _g = lock();
    create("row a");
    create("row b");
    let mut panel = focused();
    render(&mut panel, 120);
    press(&mut panel, DOWN);
    press(&mut panel, DOWN);
    assert!(panel.take_exit());
    assert!(
        panel.take_events().is_empty(),
        "the exit is taken, not kept"
    );
    assert!(!panel.take_exit());
}

#[test]
fn up_is_handed_over_only_while_the_panel_is_shown_with_rows() {
    let _g = lock();
    let mut panel = TaskPanelComponent::new();
    assert!(!panel.can_take_up(), "no rows");
    create("row a");
    assert!(panel.can_take_up());
    panel.set_radar(true);
    assert!(!panel.can_take_up(), "hidden in radar");
    panel.set_radar(false);
    panel.toggle_shown();
    assert!(!panel.can_take_up(), "hidden by Ctrl+G");
}
