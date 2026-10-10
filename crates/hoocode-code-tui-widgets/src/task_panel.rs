//! `components/task-panel.ts`: the task ledger above the prompt.
//!
//! A state-coloured left rail (working / reviewed / stopped), a lens tab strip
//! when more than one lens has content, and the rows of the chosen lens:
//! - plan ("tasks"): the main agent's TodoWrite plan, each dispatched run
//!   nested under the plan item it works on;
//! - subagents: the delegated forest (`parentTaskId`), drawn with connectors.
//!
//! The rows come from a [`RowModel`], rebuilt only when the store's
//! `version()` moves, so a frame copies nothing from the store. Its 1s run
//! clock is [`TaskPanelComponent::ticking`]: the app re-renders every second
//! while it is true.

use std::collections::{HashMap, HashSet};

use hoocode_code_agent_session::format::{format_duration_secs, format_tokens};
use hoocode_code_task_store::{
    task_store, Task, TaskAgent, TaskAgentKind, TaskKind, TaskStatus, TaskStore,
};
use hoocode_code_tui_keybindings::{app_key_label, format_key_text, raw_key_hint};
use hoocode_code_tui_theme::tui::WARNING_GLYPH;
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

/// Marker for MCP-sourced rows, which have no owning agent.
const MCP_SOURCE_GLYPH: &str = "⧉";
/// The thin left rail that groups the pane.
const RAIL: &str = "▎";

/// `TaskPanelView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPanelView {
    Plan,
    Subagents,
}

impl TaskPanelView {
    fn label(self) -> &'static str {
        match self {
            // The plan lens is the task ledger (TodoWrite items plus dispatched
            // runs); `Subagents` below is the delegation forest. Renaming the
            // plan lens to "subagents" collided with the other one.
            Self::Plan => "tasks",
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

/// A dispatched subagent run or an MCP call.
fn is_delegated(task: &Task) -> bool {
    task.kind() != TaskKind::Plan
}

/// The lenses with content, in cycle order.
fn available_views(tasks: &[Task], agents: &[TaskAgent]) -> Vec<TaskPanelView> {
    let mut views = Vec::new();
    if tasks.iter().any(Task::is_plan_item) {
        views.push(TaskPanelView::Plan);
    }
    let has_subagent_work =
        agents.iter().any(|a| a.kind == TaskAgentKind::Subagent) || tasks.iter().any(is_delegated);
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

/// The rail colour for a lens's overall state: any failure, else any live
/// work, else done.
fn panel_state_color<'a>(tasks: impl Iterator<Item = &'a Task>) -> &'static str {
    let (mut failed, mut active) = (false, false);
    for task in tasks {
        failed |= task.status == TaskStatus::Failed;
        active |= task.status.is_active();
    }
    if failed {
        "error"
    } else if active {
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

/// One row of a lens. Its identity is the task's store id, which survives a
/// rebuild; the tree connector comes from the lens shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelRow {
    pub id: u64,
    pub tree_prefix: String,
}

/// Done, total and live counts of one lens (its tab).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LensStats {
    done: usize,
    total: usize,
    live: bool,
}

/// The panel's rows for one store version: the store's tasks and agents, the
/// lenses with content, and each lens's rows. Rebuilt in [`RowModel::sync`]
/// only when the store's `version()` has moved.
#[derive(Debug, Default)]
pub struct RowModel {
    version: Option<u64>,
    tasks: Vec<Task>,
    agents: Vec<TaskAgent>,
    task_index: HashMap<u64, usize>,
    agent_index: HashMap<String, usize>,
    views: Vec<TaskPanelView>,
    plan: Vec<PanelRow>,
    subagents: Vec<PanelRow>,
    builds: u64,
}

impl RowModel {
    /// Rebuild from `store` if it has changed since the last sync. The version
    /// is read before the copy, so a mutation that races the copy costs one
    /// extra rebuild and never a stale model.
    pub fn sync(&mut self, store: &TaskStore) {
        let version = store.version();
        if self.version == Some(version) {
            return;
        }
        let tasks = store.list();
        let agents = store.agents();
        let builds = self.builds + 1;
        *self = Self::build(version, tasks, agents, builds);
    }

    fn build(version: u64, tasks: Vec<Task>, agents: Vec<TaskAgent>, builds: u64) -> Self {
        let task_index = tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        let agent_index = agents
            .iter()
            .enumerate()
            .map(|(i, a)| (a.id.clone(), i))
            .collect();
        let views = available_views(&tasks, &agents);
        let plan = plan_lens_rows(&tasks);
        let subagents = subagent_lens_rows(&tasks);
        Self {
            version: Some(version),
            tasks,
            agents,
            task_index,
            agent_index,
            views,
            plan,
            subagents,
            builds,
        }
    }

    /// How many times the model has been rebuilt from the store.
    pub fn builds(&self) -> u64 {
        self.builds
    }

    /// The lenses with content, in cycle order.
    pub fn views(&self) -> &[TaskPanelView] {
        &self.views
    }

    /// The store had no tasks at the last sync.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// The task behind a row id, if the model has it.
    pub fn task(&self, id: u64) -> Option<&Task> {
        self.task_index.get(&id).map(|&i| &self.tasks[i])
    }

    /// The agent that owns a task, if the store has one with that id.
    pub fn owner_of(&self, task: &Task) -> Option<&TaskAgent> {
        self.agent_index
            .get(&task.owner_id())
            .map(|&i| &self.agents[i])
    }

    /// The rows of a lens, each with its task, in display order.
    pub fn lens(&self, view: TaskPanelView) -> impl Iterator<Item = (&PanelRow, &Task)> + '_ {
        let rows = match view {
            TaskPanelView::Plan => &self.plan,
            TaskPanelView::Subagents => &self.subagents,
        };
        rows.iter()
            .filter_map(move |row| self.task(row.id).map(|task| (row, task)))
    }

    fn lens_stats(&self, view: TaskPanelView) -> LensStats {
        self.lens(view)
            .fold(LensStats::default(), |mut stats, (_, task)| {
                stats.total += 1;
                stats.done += usize::from(task.status == TaskStatus::Done);
                stats.live |= task.status == TaskStatus::InProgress;
                stats
            })
    }
}

/// `formatLensTabs`: one tab per lens with its done/total, plus key hints
/// when they fit.
fn format_lens_tabs(
    model: &RowModel,
    width: usize,
    view: TaskPanelView,
    tab_views: &[TaskPanelView],
    show_cycle_hint: bool,
) -> String {
    let t = theme();
    let tabs: Vec<(String, String)> = tab_views
        .iter()
        .map(|&v| {
            let stats = model.lens_stats(v);
            let count = format!("{}/{}", stats.done, stats.total);
            let plain = format!(" {} {count} ", v.label());
            if v != view {
                return (plain.clone(), t.fg("dim", &plain));
            }
            let body = format!(
                " {} {} ",
                t.bold(&t.fg("accent", v.label())),
                t.fg("muted", &count)
            );
            (
                plain,
                if stats.live {
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

    let is_mcp = task.kind() == TaskKind::Mcp;
    let owner_kind =
        options
            .owner
            .map(|o| o.kind)
            .unwrap_or(if task.kind() == TaskKind::Subagent {
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

/// `flatLensRows`: the plan, each linked subagent run nested under its item.
fn plan_lens_rows(tasks: &[Task]) -> Vec<PanelRow> {
    let mut linked: HashMap<u64, Vec<&Task>> = HashMap::new();
    for task in tasks {
        if let (Some(link), TaskKind::Subagent) = (task.linked_task_id, task.kind()) {
            linked.entry(link).or_default().push(task);
        }
    }
    let mut rows = Vec::new();
    for task in tasks.iter().filter(|t| t.is_plan_item()) {
        rows.push(PanelRow {
            id: task.id,
            tree_prefix: String::new(),
        });
        let runs = linked.get(&task.id).map(Vec::as_slice).unwrap_or_default();
        for (i, run) in runs.iter().enumerate() {
            rows.push(PanelRow {
                id: run.id,
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

/// `subagentLensRows`: a depth-first walk of the delegated forest (orphans
/// become roots; a visited set guards against cycles).
fn subagent_lens_rows(tasks: &[Task]) -> Vec<PanelRow> {
    let ids: HashSet<u64> = tasks.iter().map(|t| t.id).collect();
    let mut children: HashMap<u64, Vec<&Task>> = HashMap::new();
    for task in tasks {
        if let Some(parent) = task.parent_task_id {
            children.entry(parent).or_default().push(task);
        }
    }
    let roots = tasks.iter().filter(|t| match t.parent_task_id {
        None => is_delegated(t),
        Some(parent) => !ids.contains(&parent),
    });
    let mut rows = Vec::new();
    let mut visited = HashSet::new();
    fn walk(
        task: &Task,
        prefix: &str,
        is_last: bool,
        is_root: bool,
        children: &HashMap<u64, Vec<&Task>>,
        visited: &mut HashSet<u64>,
        rows: &mut Vec<PanelRow>,
    ) {
        if !visited.insert(task.id) {
            return;
        }
        let branch = if is_last { "└─ " } else { "├─ " };
        rows.push(PanelRow {
            id: task.id,
            tree_prefix: if is_root {
                String::new()
            } else {
                format!("{prefix}{branch}")
            },
        });
        let kids = children
            .get(&task.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
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

/// `TaskPanelComponent`.
pub struct TaskPanelComponent {
    view: TaskPanelView,
    density: TaskPanelDensity,
    disposed: bool,
    ticking: bool,
    model: RowModel,
}

impl Default for TaskPanelComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskPanelComponent {
    pub fn new() -> Self {
        Self {
            view: TaskPanelView::Plan,
            density: TaskPanelDensity::Full,
            disposed: false,
            ticking: false,
            model: RowModel::default(),
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
    /// snaps back to the plan.
    pub fn cycle_view(&mut self, forward: bool) -> TaskPanelView {
        self.model.sync(task_store());
        let available = self.model.views();
        let len = available.len() as isize;
        let idx = available
            .iter()
            .position(|v| *v == self.view)
            .map_or(-1, |i| i as isize);
        self.view = if len == 0 {
            TaskPanelView::Plan
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
        self.model.sync(task_store());
        let model = &self.model;
        let width = width as usize;
        let now = now_ms();

        // Keep the stored view while it has content, else the first lens that
        // does.
        let available = model.views();
        let view = if available.contains(&self.view) {
            self.view
        } else {
            available.first().copied().unwrap_or(TaskPanelView::Plan)
        };
        if model.is_empty() {
            self.ticking = false;
            return Vec::new();
        }

        self.ticking = model
            .lens(view)
            .any(|(_, task)| task.status == TaskStatus::InProgress && is_delegated(task));

        let t = theme();
        let gutter = format!(
            "{} ",
            t.fg(
                panel_state_color(model.lens(view).map(|(_, task)| task)),
                RAIL
            )
        );
        let inner = width.saturating_sub(visible_width(RAIL) + 1);
        let tab_views: Vec<TaskPanelView> = if available.contains(&view) {
            available.to_vec()
        } else {
            available
                .iter()
                .copied()
                .chain(std::iter::once(view))
                .collect()
        };

        if self.density == TaskPanelDensity::Summary {
            return vec![gutter.clone() + &format_lens_tabs(model, inner, view, &tab_views, false)];
        }

        let mut lines = Vec::new();
        if available.len() >= 2 {
            lines.push(gutter.clone() + &format_lens_tabs(model, inner, view, &tab_views, true));
        }
        for (row, task) in model.lens(view) {
            let options = LineOptions {
                owner: model.owner_of(task),
                tree_prefix: &row.tree_prefix,
            };
            lines.push(gutter.clone() + &format_task_line(task, inner, options, now));
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

#[cfg(test)]
mod row_model_tests {
    use super::*;
    use hoocode_code_task_store::{CreateTaskOptions, TaskAgentPatch, TaskPatch, TaskSource};

    fn plan(store: &TaskStore, title: &str) -> u64 {
        store.create(title, CreateTaskOptions::default()).id
    }

    fn run(store: &TaskStore, title: &str, options: CreateTaskOptions) -> u64 {
        store
            .create(
                title,
                CreateTaskOptions {
                    source: Some(TaskSource::Subagent),
                    ..options
                },
            )
            .id
    }

    /// `RowModel::lens` borrows the model, so the tests collect plain values.
    fn lens_ids(model: &RowModel, view: TaskPanelView) -> Vec<(u64, String)> {
        model
            .lens(view)
            .map(|(row, _)| (row.id, row.tree_prefix.clone()))
            .collect()
    }

    #[test]
    fn sync_rebuilds_only_when_the_store_version_moves() {
        let store = TaskStore::new();
        let p = plan(&store, "first");
        let mut model = RowModel::default();
        model.sync(&store);
        assert_eq!(model.builds(), 1);
        model.sync(&store);
        assert_eq!(model.builds(), 1, "an unchanged store is not copied again");

        store.update(
            p,
            TaskPatch {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        );
        model.sync(&store);
        assert_eq!(model.builds(), 2);
        assert_eq!(
            model.task(p).map(|t| t.status),
            Some(TaskStatus::Done),
            "the rebuilt model shows the new status"
        );
    }

    #[test]
    fn plan_lens_nests_a_linked_run_and_keeps_an_unlinked_one_out() {
        let store = TaskStore::new();
        let p1 = plan(&store, "first item");
        let p2 = plan(&store, "second item");
        let linked = store.create(
            "linked run",
            CreateTaskOptions {
                source: Some(TaskSource::Subagent),
                linked_task_id: Some(p1),
                ..Default::default()
            },
        );
        let unlinked = run(&store, "unlinked run", CreateTaskOptions::default());
        let mut model = RowModel::default();
        model.sync(&store);

        assert_eq!(
            lens_ids(&model, TaskPanelView::Plan),
            vec![
                (p1, String::new()),
                (linked.id, "└─ ".into()),
                (p2, String::new()),
            ]
        );
        // Both runs are delegated roots, so both show in the subagents lens.
        assert_eq!(
            lens_ids(&model, TaskPanelView::Subagents),
            vec![(linked.id, String::new()), (unlinked, String::new())]
        );
    }

    #[test]
    fn subagents_lens_draws_a_child_under_its_parent_run() {
        let store = TaskStore::new();
        let parent = run(&store, "parent", CreateTaskOptions::default());
        let child = run(
            &store,
            "child",
            CreateTaskOptions {
                parent_task_id: Some(parent),
                ..Default::default()
            },
        );
        let mut model = RowModel::default();
        model.sync(&store);
        assert_eq!(
            lens_ids(&model, TaskPanelView::Subagents),
            vec![(parent, String::new()), (child, "└─ ".into())]
        );
    }

    #[test]
    fn a_row_keeps_its_id_across_a_rebuild() {
        let store = TaskStore::new();
        let p1 = plan(&store, "first");
        let p2 = plan(&store, "second");
        let mut model = RowModel::default();
        model.sync(&store);
        let before = lens_ids(&model, TaskPanelView::Plan);

        store.update(
            p2,
            TaskPatch {
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        );
        model.sync(&store);
        assert_eq!(lens_ids(&model, TaskPanelView::Plan), before);
        assert_eq!(model.lens(TaskPanelView::Plan).next().unwrap().0.id, p1);
        assert_eq!(
            model.task(p2).map(|t| t.status),
            Some(TaskStatus::InProgress)
        );
    }

    #[test]
    fn a_task_owned_by_an_agent_is_in_no_lens_but_its_owner_resolves() {
        let store = TaskStore::new();
        store.upsert_agent(
            "explore",
            "explore",
            TaskAgentKind::Subagent,
            TaskAgentPatch::default(),
        );
        let owned = store.create(
            "owned by explore",
            CreateTaskOptions {
                agent: Some("explore".into()),
                ..Default::default()
            },
        );
        let run_id = run(
            &store,
            "a run",
            CreateTaskOptions {
                agent: Some("explore".into()),
                ..Default::default()
            },
        );
        let mut model = RowModel::default();
        model.sync(&store);

        assert!(lens_ids(&model, TaskPanelView::Plan).is_empty());
        assert_eq!(
            lens_ids(&model, TaskPanelView::Subagents),
            vec![(run_id, String::new())]
        );
        let owner = model.owner_of(model.task(run_id).unwrap());
        assert_eq!(owner.map(|a| a.id.as_str()), Some("explore"));
        assert!(model.owner_of(model.task(owned.id).unwrap()).is_some());
    }

    #[test]
    fn views_list_only_the_lenses_with_content() {
        let store = TaskStore::new();
        let mut model = RowModel::default();
        model.sync(&store);
        assert!(model.views().is_empty());

        plan(&store, "only a plan item");
        model.sync(&store);
        assert_eq!(model.views(), [TaskPanelView::Plan]);

        run(&store, "a run", CreateTaskOptions::default());
        model.sync(&store);
        assert_eq!(
            model.views(),
            [TaskPanelView::Plan, TaskPanelView::Subagents]
        );
    }

    #[test]
    fn a_subagent_agent_without_tasks_still_opens_the_subagents_lens() {
        let store = TaskStore::new();
        store.upsert_agent(
            "explore",
            "explore",
            TaskAgentKind::Subagent,
            TaskAgentPatch::default(),
        );
        let mut model = RowModel::default();
        model.sync(&store);
        assert_eq!(model.views(), [TaskPanelView::Subagents]);
    }

    #[test]
    fn lens_stats_count_done_total_and_live_for_each_lens() {
        let store = TaskStore::new();
        let p1 = plan(&store, "done item");
        plan(&store, "live item");
        store.update(
            p1,
            TaskPatch {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        );
        let r = run(&store, "finished run", CreateTaskOptions::default());
        store.update(
            r,
            TaskPatch {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        );
        let mut model = RowModel::default();
        model.sync(&store);

        let plan_stats = model.lens_stats(TaskPanelView::Plan);
        assert_eq!(
            (plan_stats.done, plan_stats.total, plan_stats.live),
            (1, 2, false)
        );
        let sub_stats = model.lens_stats(TaskPanelView::Subagents);
        assert_eq!(
            (sub_stats.done, sub_stats.total, sub_stats.live),
            (1, 1, false)
        );
    }

    #[test]
    fn an_unknown_row_id_has_no_task() {
        let store = TaskStore::new();
        plan(&store, "one");
        let mut model = RowModel::default();
        model.sync(&store);
        assert!(model.task(999).is_none());
    }

    #[test]
    fn panel_state_color_reports_failure_then_live_work_then_done() {
        let store = TaskStore::new();
        let a = plan(&store, "a");
        let b = plan(&store, "b");
        store.update(
            a,
            TaskPatch {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        );
        let mut model = RowModel::default();
        model.sync(&store);
        let color =
            |model: &RowModel| panel_state_color(model.lens(TaskPanelView::Plan).map(|(_, t)| t));
        assert_eq!(color(&model), "warning", "a pending item is live work");
        store.update(
            b,
            TaskPatch {
                status: Some(TaskStatus::Failed),
                ..Default::default()
            },
        );
        model.sync(&store);
        assert_eq!(color(&model), "error");
    }
}
