//! `components/task-panel.ts`: the task ledger above the prompt.
//!
//! A state-coloured left rail (working / reviewed / stopped), a lens tab strip
//! when more than one lens has content, and the rows of the chosen lens:
//! - flat ("tasks"): the main agent's TodoWrite plan, each dispatched run
//!   nested under the plan item it works on;
//! - subagents: the delegated forest (`parentTaskId`), drawn with connectors.
//!
//! The pin memoizes rows per store version; that is only an optimization and
//! is not reproduced. Its 1s run clock is [`TaskPanelComponent::ticking`]: the
//! app re-renders every second while it is true.

use std::collections::{HashMap, HashSet};

use hoocode_code_agent_session::format::{format_duration_secs, format_tokens};
use hoocode_code_task_store::{
    task_owner_id, task_store, Task, TaskAgent, TaskAgentKind, TaskSource, TaskStatus,
};
use hoocode_code_tui_keybindings::{app_key_label, format_key_text, raw_key_hint};
use hoocode_code_tui_theme::{agent_color_for, theme};
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_to_width, visible_width};

fn task_status_icon(status: TaskStatus) -> &'static str {
    // Hollow = not started; ● stays the chat's tool status dot.
    match status {
        TaskStatus::Pending => "○",
        TaskStatus::InProgress => "◐",
        TaskStatus::Done => "✓",
        TaskStatus::Failed => "✗",
        TaskStatus::Cancelled => "⊘",
    }
}

/// U+26A0 with VS15, so terminals draw it one cell wide.
const WARNING_GLYPH: &str = "⚠\u{fe0e}";
/// Marker for MCP-sourced rows, which have no owning agent.
const MCP_SOURCE_GLYPH: &str = "⧉";
/// The thin left rail that groups the pane.
const RAIL: &str = "▎";

/// `TaskPanelView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPanelView {
    Flat,
    Subagents,
}

impl TaskPanelView {
    fn label(self) -> &'static str {
        match self {
            // The flat lens is the task ledger (TodoWrite items plus dispatched
            // runs); `Subagents` below is the delegation forest. Renaming the
            // flat lens to "subagents" collided with the other one.
            Self::Flat => "tasks",
            Self::Subagents => "subagents",
        }
    }
}

/// `TaskPanelDensity`: every row, or the counts alone on one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPanelDensity {
    Full,
    Summary,
}

/// The main agent's own TodoWrite plan item.
fn is_main_task(task: &Task) -> bool {
    task.source.is_none() && task.agent.is_none() && task.parent_task_id.is_none()
}

/// A dispatched subagent run or an MCP call.
fn is_delegated(task: &Task) -> bool {
    task.source.is_some()
}

/// The lenses with content, in cycle order.
fn available_views(tasks: &[Task], agents: &[TaskAgent]) -> Vec<TaskPanelView> {
    let mut views = Vec::new();
    if tasks.iter().any(is_main_task) {
        views.push(TaskPanelView::Flat);
    }
    let has_subagent_work = agents.iter().any(|a| a.kind == TaskAgentKind::Subagent)
        || tasks.iter().any(|t| t.source.is_some());
    if has_subagent_work {
        views.push(TaskPanelView::Subagents);
    }
    views
}

fn agent_glyph(kind: TaskAgentKind) -> &'static str {
    match kind {
        TaskAgentKind::Main => "◆",
        TaskAgentKind::Subagent => "◇",
    }
}

/// The rail colour for the lens's overall state.
fn panel_state_color(tasks: &[Task]) -> &'static str {
    if tasks.iter().any(|t| t.status == TaskStatus::Failed) {
        "error"
    } else if tasks.iter().any(|t| t.status.is_active()) {
        "warning"
    } else {
        "success"
    }
}

fn task_status_color(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::InProgress => "warning",
        TaskStatus::Done => "success",
        TaskStatus::Failed => "error",
        // A user-initiated cancel is not an error.
        TaskStatus::Pending | TaskStatus::Cancelled => "dim",
    }
}

fn now_ms() -> u64 {
    hoocode_code_task_store::now_ms()
}

/// Wall-clock time a delegated task occupied (running ones against now).
fn task_elapsed_secs(task: &Task, now: u64) -> f64 {
    let end = if task.status == TaskStatus::InProgress {
        now
    } else {
        task.updated_at
    };
    end.saturating_sub(task.created_at) as f64 / 1000.0
}

fn pad(n: usize) -> String {
    " ".repeat(n)
}

/// `formatLensTabs`: one tab per lens with its done/total, plus key hints
/// when they fit.
fn format_lens_tabs(
    tasks: &[Task],
    width: usize,
    view: TaskPanelView,
    tab_views: &[TaskPanelView],
    show_cycle_hint: bool,
) -> String {
    let t = theme();
    let tabs: Vec<(String, String)> = tab_views
        .iter()
        .map(|&v| {
            let lens = filter_tasks_for_lens(tasks, v);
            let done = lens.iter().filter(|t| t.status == TaskStatus::Done).count();
            let count = format!("{done}/{}", lens.len());
            let plain = format!(" {} {count} ", v.label());
            if v != view {
                return (plain.clone(), t.fg("dim", &plain));
            }
            let body = format!(
                " {} {} ",
                t.bold(&t.fg("accent", v.label())),
                t.fg("muted", &count)
            );
            let live = lens.iter().any(|t| t.status == TaskStatus::InProgress);
            (
                plain,
                if live {
                    t.bg("selectedBg", &body)
                } else {
                    body
                },
            )
        })
        .collect();
    let selected = tab_views
        .iter()
        .position(|&v| v == view)
        .and_then(|i| tabs.get(i))
        .or(tabs.first())
        .cloned();
    let strip = (
        tabs.iter().map(|t| t.0.as_str()).collect::<String>(),
        tabs.iter().map(|t| t.1.as_str()).collect::<String>(),
    );

    let mut parts: Vec<(String, String)> = Vec::new();
    if show_cycle_hint {
        let key = app_key_label("app.tasks.cycleForward");
        parts.push((
            format!("{} cycle", format_key_text(&key, false)),
            raw_key_hint(&key, "cycle"),
        ));
    }
    let sep = "  ";
    let hint_plain = parts
        .iter()
        .map(|p| p.0.as_str())
        .collect::<Vec<_>>()
        .join(sep);
    let hint_styled = parts
        .iter()
        .map(|p| p.1.as_str())
        .collect::<Vec<_>>()
        .join(&t.fg("muted", sep));

    // Narrowing order: drop the hints, then the unselected tabs.
    if !hint_plain.is_empty() && visible_width(&strip.0) + 2 + visible_width(&hint_plain) <= width {
        let gap = width - visible_width(&strip.0) - visible_width(&hint_plain);
        return format!("{}{}{hint_styled}", strip.1, pad(gap));
    }
    for (plain, styled) in std::iter::once(strip).chain(selected.clone()) {
        if visible_width(&plain) <= width {
            return format!("{styled}{}", pad(width - visible_width(&plain)));
        }
    }
    truncate_to_width(
        &selected.map(|s| s.0).unwrap_or_default(),
        width,
        "…",
        false,
    )
}

/// Row options (`formatTaskLine`'s third argument).
#[derive(Default, Clone, Copy)]
struct LineOptions<'a> {
    owner: Option<&'a TaskAgent>,
    tree_prefix: &'a str,
}

/// `formatTaskLine`: status icon, owner glyph where it disambiguates, origin
/// tag, title, and a right column (usage, live activity and run clock, or a
/// warning note), padded to the pane width.
fn format_task_line(task: &Task, width: usize, options: LineOptions<'_>, now: u64) -> String {
    let t = theme();
    let icon = t.fg(
        task_status_color(task.status),
        task_status_icon(task.status),
    );

    let is_mcp = task.source == Some(TaskSource::Mcp);
    let owner_kind =
        options
            .owner
            .map(|o| o.kind)
            .unwrap_or(if task.source == Some(TaskSource::Subagent) {
                TaskAgentKind::Subagent
            } else {
                TaskAgentKind::Main
            });
    let show_owner_glyph = is_mcp || owner_kind != TaskAgentKind::Main;
    let source_glyph = if is_mcp {
        MCP_SOURCE_GLYPH
    } else {
        agent_glyph(owner_kind)
    };
    let agent_type_name = (!is_mcp && owner_kind == TaskAgentKind::Subagent)
        .then(|| task.subagent_mode.as_deref().or(task.agent.as_deref()))
        .flatten();
    let styled_source = if !show_owner_glyph {
        String::new()
    } else if let Some(name) = agent_type_name {
        t.fg(agent_color_for(name), source_glyph)
    } else {
        t.fg("dim", source_glyph)
    };

    let (tag, tag_color) = if is_mcp {
        (
            format!("[{}]", task.subagent_mode.as_deref().unwrap_or("MCP")),
            "mcp",
        )
    } else if let Some(mode) = &task.subagent_mode {
        (format!("[{mode}]"), agent_color_for(mode))
    } else {
        (String::new(), "accent")
    };
    let styled_tag = if tag.is_empty() {
        String::new()
    } else {
        format!("{} ", t.fg(tag_color, &tag))
    };
    let title = task.title.as_str();
    let styled_title = match task.status {
        TaskStatus::Done => t.fg("muted", title),
        TaskStatus::Pending => t.fg("dim", title),
        TaskStatus::Failed => t.fg("error", title),
        TaskStatus::Cancelled => t.fg("dim", &t.strikethrough(title)),
        TaskStatus::InProgress => t.bold(title),
    };

    let mut right_plain = String::new();
    let mut right_styled = String::new();
    match task.status {
        TaskStatus::Done | TaskStatus::Failed => {
            if let Some(usage) = &task.usage {
                let total = usage.input + usage.output;
                if total > 0.0 {
                    right_plain = format_tokens(total as u64);
                    right_styled = t.fg("muted", &right_plain);
                }
            }
        }
        TaskStatus::InProgress => {
            let activity = options
                .owner
                .and_then(|o| o.activity.as_deref())
                .filter(|a| !a.is_empty())
                .map(|a| format!("⋯ {a}"))
                .unwrap_or_default();
            // Attempt, model, deadline: the three things that explain a slow or
            // surprising run. Rendered at draw time, so the countdown ticks.
            let run_detail = options
                .owner
                .map(|owner| describe_run(owner, now))
                .unwrap_or_default();
            let run_for = if is_delegated(task) {
                format_duration_secs(task_elapsed_secs(task, now))
            } else {
                String::new()
            };
            let join = |plain: String, styled: String| -> (String, String) {
                if run_detail.is_empty() {
                    return (plain, styled);
                }
                (
                    format!("{plain} · {run_detail}"),
                    format!("{styled}{}", t.fg("dim", &format!(" · {run_detail}"))),
                )
            };
            match (activity.is_empty(), run_for.is_empty()) {
                (false, false) => {
                    let (plain, styled) = join(
                        format!("{activity} · {run_for}"),
                        t.fg("warning", &activity) + &t.fg("dim", &format!(" · {run_for}")),
                    );
                    right_plain = plain;
                    right_styled = styled;
                }
                (false, true) => {
                    let (plain, styled) = join(activity.clone(), t.fg("warning", &activity));
                    right_plain = plain;
                    right_styled = styled;
                }
                (true, false) => {
                    let (plain, styled) = join(run_for.clone(), t.fg("dim", &run_for));
                    right_plain = plain;
                    right_styled = styled;
                }
                (true, true) => {}
            }
        }
        _ => {}
    }
    if let Some(note) = &task.note {
        right_plain = format!("{WARNING_GLYPH} {note}");
        right_styled = t.fg("warning", &right_plain);
    }

    let right_width = if right_plain.is_empty() {
        0
    } else {
        visible_width(&right_plain) + 1
    };
    let left_width = width.saturating_sub(right_width);
    let tree_prefix = if options.tree_prefix.is_empty() {
        String::new()
    } else {
        t.fg("borderMuted", options.tree_prefix)
    };
    let source = if styled_source.is_empty() {
        String::new()
    } else {
        format!("{styled_source} ")
    };
    let left_body = format!("{tree_prefix}{icon} {source}{styled_tag}{styled_title}");
    let left = truncate_to_width(&left_body, left_width, "…", false);
    if right_plain.is_empty() {
        let gap = width.saturating_sub(visible_width(&left));
        return format!("{left}{}", pad(gap));
    }
    let gap = width
        .saturating_sub(visible_width(&left) + visible_width(&right_plain))
        .max(1);
    format!("{left}{}{right_styled}", pad(gap))
}

/// One row of the flat or subagents lens: a task and its tree connector.
struct TreeRow<'a> {
    task: &'a Task,
    tree_prefix: String,
}

/// `subagentLensRows`: a depth-first walk of the delegated forest (orphans
/// become roots; a visited set guards against cycles).
fn subagent_lens_rows(tasks: &[Task]) -> Vec<TreeRow<'_>> {
    let ids: HashSet<u64> = tasks.iter().map(|t| t.id).collect();
    let mut children: HashMap<u64, Vec<&Task>> = HashMap::new();
    for task in tasks {
        if let Some(parent) = task.parent_task_id {
            children.entry(parent).or_default().push(task);
        }
    }
    let roots = tasks.iter().filter(|t| match t.parent_task_id {
        None => t.source.is_some(),
        Some(parent) => !ids.contains(&parent),
    });
    let mut rows = Vec::new();
    let mut visited = HashSet::new();
    fn walk<'a>(
        task: &'a Task,
        prefix: &str,
        is_last: bool,
        is_root: bool,
        children: &HashMap<u64, Vec<&'a Task>>,
        visited: &mut HashSet<u64>,
        rows: &mut Vec<TreeRow<'a>>,
    ) {
        if !visited.insert(task.id) {
            return;
        }
        let branch = if is_last { "└─ " } else { "├─ " };
        rows.push(TreeRow {
            task,
            tree_prefix: if is_root {
                String::new()
            } else {
                format!("{prefix}{branch}")
            },
        });
        let kids = children.get(&task.id).cloned().unwrap_or_default();
        let child_prefix = if is_root {
            String::new()
        } else {
            format!("{prefix}{}", if is_last { "   " } else { "│  " })
        };
        for (i, kid) in kids.iter().enumerate() {
            walk(
                kid,
                &child_prefix,
                i == kids.len() - 1,
                false,
                children,
                visited,
                rows,
            );
        }
    }
    for root in roots {
        walk(root, "", true, true, &children, &mut visited, &mut rows);
    }
    rows
}

/// `flatLensRows`: the plan, each linked subagent run nested under its item.
fn flat_lens_rows(tasks: &[Task]) -> Vec<TreeRow<'_>> {
    let mut linked: HashMap<u64, Vec<&Task>> = HashMap::new();
    for task in tasks {
        if let (Some(link), Some(TaskSource::Subagent)) = (task.linked_task_id, task.source) {
            linked.entry(link).or_default().push(task);
        }
    }
    let mut rows = Vec::new();
    for task in tasks.iter().filter(|t| is_main_task(t)) {
        rows.push(TreeRow {
            task,
            tree_prefix: String::new(),
        });
        let runs = linked.get(&task.id).cloned().unwrap_or_default();
        for (i, run) in runs.iter().enumerate() {
            rows.push(TreeRow {
                task: run,
                tree_prefix: if i == runs.len() - 1 {
                    "└─ "
                } else {
                    "├─ "
                }
                .to_string(),
            });
        }
    }
    rows
}

/// `filterTasksForLens`: exactly the tasks the lens shows.
fn filter_tasks_for_lens(tasks: &[Task], view: TaskPanelView) -> Vec<&Task> {
    match view {
        TaskPanelView::Flat => flat_lens_rows(tasks).into_iter().map(|r| r.task).collect(),
        TaskPanelView::Subagents => subagent_lens_rows(tasks)
            .into_iter()
            .map(|r| r.task)
            .collect(),
    }
}

/// `TaskPanelComponent`.
pub struct TaskPanelComponent {
    view: TaskPanelView,
    density: TaskPanelDensity,
    disposed: bool,
    ticking: bool,
}

impl Default for TaskPanelComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskPanelComponent {
    pub fn new() -> Self {
        Self {
            view: TaskPanelView::Flat,
            density: TaskPanelDensity::Full,
            disposed: false,
            ticking: false,
        }
    }

    pub fn get_view(&self) -> TaskPanelView {
        self.view
    }

    pub fn set_view(&mut self, view: TaskPanelView) {
        self.view = view;
    }

    /// `setDensity`: driven by the chrome, not by the pane.
    pub fn set_density(&mut self, density: TaskPanelDensity) {
        self.density = density;
    }

    /// `cycleView`: the next lens with content, wrapping; an emptied view
    /// snaps back to flat.
    pub fn cycle_view(&mut self, forward: bool) -> TaskPanelView {
        let store = task_store();
        let available = available_views(&store.list(), &store.agents());
        let len = available.len() as isize;
        let idx = available
            .iter()
            .position(|v| *v == self.view)
            .map_or(-1, |i| i as isize);
        self.view = if len == 0 {
            TaskPanelView::Flat
        } else {
            let step = if forward { 1 } else { -1 };
            available[((idx + step + len) % len) as usize]
        };
        self.view
    }

    /// A delegated row is live: re-render every second for its run clock.
    pub fn ticking(&self) -> bool {
        self.ticking
    }

    /// Stop the run clock. Call on teardown.
    pub fn dispose(&mut self) {
        self.disposed = true;
        self.ticking = false;
    }
}

impl Component for TaskPanelComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.disposed {
            return Vec::new();
        }
        let width = width as usize;
        let store = task_store();
        let tasks = store.list();
        let agents = store.agents();
        let now = now_ms();

        // Keep the stored view while it has content, else the first lens that
        // does.
        let available = available_views(&tasks, &agents);
        let view = if available.contains(&self.view) {
            self.view
        } else {
            available.first().copied().unwrap_or(TaskPanelView::Flat)
        };
        if tasks.is_empty() {
            self.ticking = false;
            return Vec::new();
        }

        let lens: Vec<Task> = filter_tasks_for_lens(&tasks, view)
            .into_iter()
            .cloned()
            .collect();
        self.ticking = lens
            .iter()
            .any(|t| t.status == TaskStatus::InProgress && is_delegated(t));

        let t = theme();
        let gutter = format!("{} ", t.fg(panel_state_color(&lens), RAIL));
        let inner = width.saturating_sub(visible_width(RAIL) + 1);
        let tab_views: Vec<TaskPanelView> = if available.contains(&view) {
            available.clone()
        } else {
            available
                .iter()
                .copied()
                .chain(std::iter::once(view))
                .collect()
        };

        if self.density == TaskPanelDensity::Summary {
            return vec![
                gutter.clone() + &format_lens_tabs(&tasks, inner, view, &tab_views, false),
            ];
        }

        let mut lines = Vec::new();
        if available.len() >= 2 {
            lines.push(gutter.clone() + &format_lens_tabs(&tasks, inner, view, &tab_views, true));
        }

        let agent_by_id: HashMap<&str, &TaskAgent> =
            agents.iter().map(|a| (a.id.as_str(), a)).collect();
        let rows = match view {
            TaskPanelView::Flat => flat_lens_rows(&tasks),
            TaskPanelView::Subagents => subagent_lens_rows(&tasks),
        };
        for row in rows {
            let owner_id = task_owner_id(row.task.agent.as_deref(), row.task.source);
            let options = LineOptions {
                owner: agent_by_id.get(owner_id.as_str()).copied(),
                tree_prefix: &row.tree_prefix,
            };
            lines.push(gutter.clone() + &format_task_line(row.task, inner, options, now));
        }
        lines
    }
}

/// `attempt 2 · anthropic/claude-opus-5 · 3:12 left` — the three facts that
/// explain a run, and the outcome once it has one.
fn describe_run(owner: &TaskAgent, now: u64) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(attempt) = owner.attempt.filter(|n| *n > 1) {
        parts.push(format!("attempt {attempt}"));
    }
    if let Some(model) = owner.model.as_deref().filter(|m| !m.is_empty()) {
        parts.push(model.to_string());
    }
    if let Some(deadline) = owner.deadline_at.filter(|d| *d > now) {
        parts.push(format!(
            "{} left",
            format_duration_secs(((deadline - now) / 1000) as f64)
        ));
    }
    if let Some(outcome) = owner.outcome.as_deref().filter(|o| !o.is_empty()) {
        parts.clear();
        let mut line = outcome.to_string();
        if let Some(confidence) = owner.confidence {
            line.push_str(&format!(" · {confidence:.1}"));
        }
        if let Some(cause) = owner.cause.as_deref().filter(|c| !c.is_empty()) {
            let one_line: String = cause.split_whitespace().collect::<Vec<_>>().join(" ");
            line.push_str(&format!(" · {}", truncate_to(&one_line, 48)));
        }
        parts.push(line);
    }
    parts.join(" · ")
}

fn truncate_to(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}
