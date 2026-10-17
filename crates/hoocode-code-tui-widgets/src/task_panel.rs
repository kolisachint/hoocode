//! `components/task-panel.ts`: the task ledger above the prompt.
//!
//! A state-coloured left rail (working / reviewed / stopped), a tab strip
//! when more than one tab has rows, and the rows of the current tab. One tab
//! per kind of task, in tab order (see [`TaskPanelView`]); a tab with no rows
//! is hidden.
//! - Plan: the main agent's TodoWrite plan, each dispatched run nested under
//!   the plan item it works on;
//! - Agents: the subagent forest (`parentTaskId`), drawn with connectors;
//! - MCP: the MCP calls, flat.
//!
//! Unfocused, the panel shows the current tab and a count per tab. Focused
//! (Ctrl+G, `app.panel.focus`), it takes the keyboard: tabs switch, a row is
//! picked, Enter opens it and `x` (twice) cancels a subagent run. What the app
//! must do about those is queued as [`PanelEvent`]s, read with
//! [`TaskPanelComponent::take_events`].
//!
//! The rows come from a [`RowModel`], rebuilt only when the store's
//! `version()` moves, so a frame copies nothing from the store. Its 1s run
//! clock is [`TaskPanelComponent::ticking`]: the app re-renders every second
//! while it is true.

use std::collections::{HashMap, HashSet};

use hoocode_code_agent_session::format::{format_duration_secs, format_tokens};
use hoocode_code_task_store::{
    task_store, RowAction, Task, TaskAgent, TaskAgentKind, TaskKind, TaskStatus, TaskStore,
};
use hoocode_code_tui_keybindings::{app_key_label, format_key_text, key_text, raw_key_hint};
use hoocode_code_tui_theme::tui::WARNING_GLYPH;
use hoocode_code_tui_theme::{agent_color_for, theme};
use hoocode_tui_keys::{get_keybindings, is_key_release};
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

/// One tab of the panel: the rows of one [`TaskKind`]. Declared in tab order
/// (the derived `Ord`), and [`TaskPanelView::ALL`] lists them, so [`RowModel`]
/// derives the tabs from the rows present. A new kind gets a variant here, a
/// label, and a rule in [`lens_rows`]; the compiler points at each of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPanelView {
    /// The main agent's TodoWrite plan, with each linked run nested under its item.
    Plan,
    /// Dispatched subagent runs: the delegation forest.
    Subagents,
    /// Background shell jobs, one row each.
    Shell,
    /// Scheduled jobs (`/loop`, the Cron tools), one row each.
    Schedule,
    /// MCP calls and servers.
    Mcp,
}

impl TaskPanelView {
    /// Every tab, in tab order (Plan · Agents · Shell · Schedule · MCP).
    pub const ALL: [TaskPanelView; 5] = [
        Self::Plan,
        Self::Subagents,
        Self::Shell,
        Self::Schedule,
        Self::Mcp,
    ];

    /// The kind of task this tab lists.
    pub fn kind(self) -> TaskKind {
        match self {
            Self::Plan => TaskKind::Plan,
            Self::Subagents => TaskKind::Subagent,
            Self::Shell => TaskKind::Shell,
            Self::Schedule => TaskKind::Schedule,
            Self::Mcp => TaskKind::Mcp,
        }
    }

    /// The tab's name in the strip.
    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "Plan",
            Self::Subagents => "Agents",
            Self::Shell => "Shell",
            Self::Schedule => "Schedule",
            Self::Mcp => "MCP",
        }
    }
}

/// What the panel asks the app to do. Queued by key handling and read with
/// [`TaskPanelComponent::take_events`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelEvent {
    /// Enter on a row. The attach view (T3.3) takes it; until then the app
    /// does nothing.
    Open { view: TaskPanelView, task_id: u64 },
    /// Enter on a row that offers an action (Login), or `x` twice on a row
    /// that offers Abort. The store performs it.
    Perform { task_id: u64, action: RowAction },
    /// `x` pressed twice on a running subagent. `run_id` is the pool's id for
    /// the run (the store task's `agent`).
    Cancel { run_id: String },
    /// Back to the prompt: Esc, or the focus key again.
    Exit,
}

/// `TaskPanelDensity`: every row, or the counts alone on one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPanelDensity {
    Full,
    Summary,
}

/// A row with a run clock: every row that is not a plan item (a subagent run, an
/// MCP call, a shell job or a scheduled job). A running one shows how long it has run.
fn is_delegated(task: &Task) -> bool {
    task.kind() != TaskKind::Plan
}

/// The owner glyph of a subagent's row. The main agent's rows carry none
/// (`show_owner_glyph` is false for them), so it has no glyph.
const SUBAGENT_GLYPH: &str = "◇";

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
/// tabs with rows, and each tab's rows. Rebuilt in [`RowModel::sync`] only
/// when the store's `version()` has moved.
#[derive(Debug, Default)]
pub struct RowModel {
    version: Option<u64>,
    tasks: Vec<Task>,
    agents: Vec<TaskAgent>,
    task_index: HashMap<u64, usize>,
    agent_index: HashMap<String, usize>,
    /// The tabs that have rows, in tab order.
    views: Vec<TaskPanelView>,
    /// Each tab's rows, for the tabs in `views`.
    lenses: Vec<(TaskPanelView, Vec<PanelRow>)>,
    /// Some row in any tab is running (drives the tick).
    running: bool,
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
        let task_index: HashMap<u64, usize> =
            tasks.iter().enumerate().map(|(i, t)| (t.id, i)).collect();
        let agent_index = agents
            .iter()
            .enumerate()
            .map(|(i, a)| (a.id.clone(), i))
            .collect();
        let lenses: Vec<(TaskPanelView, Vec<PanelRow>)> = TaskPanelView::ALL
            .into_iter()
            .map(|view| (view, lens_rows(view, &tasks)))
            .filter(|(_, rows)| !rows.is_empty())
            .collect();
        let views = lenses.iter().map(|(view, _)| *view).collect();
        let running = lenses.iter().any(|(_, rows)| {
            rows.iter().any(|row| {
                task_index
                    .get(&row.id)
                    .is_some_and(|&i| tasks[i].status == TaskStatus::InProgress)
            })
        });
        Self {
            version: Some(version),
            tasks,
            agents,
            task_index,
            agent_index,
            views,
            lenses,
            running,
            builds,
        }
    }

    /// How many times the model has been rebuilt from the store.
    pub fn builds(&self) -> u64 {
        self.builds
    }

    /// The tabs with rows, in tab order. A tab with no rows is not listed.
    pub fn views(&self) -> &[TaskPanelView] {
        &self.views
    }

    /// The store had no tasks at the last sync.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Some row in any tab is running at the last sync.
    pub fn any_running(&self) -> bool {
        self.running
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

    /// The row ids of a tab, in display order (empty for a tab with no rows).
    pub fn row_ids(&self, view: TaskPanelView) -> Vec<u64> {
        self.rows_of(view).iter().map(|row| row.id).collect()
    }

    fn rows_of(&self, view: TaskPanelView) -> &[PanelRow] {
        self.lenses
            .iter()
            .find(|(v, _)| *v == view)
            .map(|(_, rows)| rows.as_slice())
            .unwrap_or_default()
    }

    /// The rows of a tab, each with its task, in display order.
    pub fn lens(&self, view: TaskPanelView) -> impl Iterator<Item = (&PanelRow, &Task)> + '_ {
        self.rows_of(view)
            .iter()
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

/// The rows of one tab for a set of tasks.
fn lens_rows(view: TaskPanelView, tasks: &[Task]) -> Vec<PanelRow> {
    match view {
        TaskPanelView::Plan => plan_lens_rows(tasks),
        TaskPanelView::Subagents => subagent_lens_rows(tasks),
        TaskPanelView::Shell | TaskPanelView::Schedule | TaskPanelView::Mcp => {
            flat_lens_rows(tasks, view.kind())
        }
    }
}

/// Every task of one kind, in store order, with no tree.
fn flat_lens_rows(tasks: &[Task], kind: TaskKind) -> Vec<PanelRow> {
    tasks
        .iter()
        .filter(|task| task.kind() == kind)
        .map(|task| PanelRow {
            id: task.id,
            tree_prefix: String::new(),
        })
        .collect()
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
        let key = app_key_label("app.panel.focus");
        parts.push((
            format!("{} focus", format_key_text(&key, false)),
            raw_key_hint(&key, "focus"),
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
    /// The row's cancel is armed: the right column asks for the second `x`.
    confirm: bool,
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
    // Not mcp and shown means a subagent: the main agent's rows show no glyph.
    let source_glyph = if is_mcp {
        MCP_SOURCE_GLYPH
    } else {
        SUBAGENT_GLYPH
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
        TaskStatus::Pending if task.kind() == TaskKind::Schedule => {
            // A waiting job shows when it next fires (the store's `next_at`).
            if let Some(next) = task.next_at.filter(|next| *next > now) {
                right_plain = format!(
                    "next in {}",
                    format_duration_secs(((next - now) / 1000) as f64)
                );
                right_styled = t.fg("dim", &right_plain);
            }
        }
        _ => {}
    }
    if let Some(note) = &task.note {
        right_plain = format!("{WARNING_GLYPH} {note}");
        right_styled = t.fg("warning", &right_plain);
    }
    if options.confirm {
        right_plain = "press x again to cancel".to_string();
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

/// `subagentLensRows`: a depth-first walk of the subagent runs (orphans become
/// roots; a visited set guards against cycles). MCP calls are not in it.
fn subagent_lens_rows(tasks: &[Task]) -> Vec<PanelRow> {
    let runs: Vec<&Task> = tasks
        .iter()
        .filter(|t| t.kind() == TaskKind::Subagent)
        .collect();
    let ids: HashSet<u64> = runs.iter().map(|t| t.id).collect();
    let mut children: HashMap<u64, Vec<&Task>> = HashMap::new();
    for task in &runs {
        if let Some(parent) = task.parent_task_id {
            children.entry(parent).or_default().push(task);
        }
    }
    let roots = runs.iter().copied().filter(|t| match t.parent_task_id {
        None => true,
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
    /// The panel has the keyboard (Ctrl+G).
    focused: bool,
    /// The picked row's id. Kept by id, so a rebuild keeps the pick.
    selected: Option<u64>,
    /// The row whose cancel is armed (one `x` pressed).
    confirming: Option<u64>,
    /// Queued for the app; see [`PanelEvent`].
    events: Vec<PanelEvent>,
}

impl Default for TaskPanelComponent {
    fn default() -> Self {
        Self::new()
    }
}

/// The id of the row to pick: the stored one while it is in `view`, else the
/// view's first row.
fn resolve_selection(model: &RowModel, view: TaskPanelView, selected: Option<u64>) -> Option<u64> {
    let ids = model.row_ids(view);
    match selected {
        Some(id) if ids.contains(&id) => Some(id),
        _ => ids.first().copied(),
    }
}

/// The keys the panel answers to, in the order they are tried.
const PANEL_KEYS: [&str; 8] = [
    "app.panel.focus",
    "app.panel.nextTab",
    "app.panel.prevTab",
    "app.panel.up",
    "app.panel.down",
    "app.panel.open",
    "app.panel.cancel",
    "app.panel.exit",
];

/// The focused panel's key hint line.
fn focus_hint(width: usize) -> String {
    let t = theme();
    let up_down = format!(
        "{}/{}",
        key_text("app.panel.up"),
        key_text("app.panel.down")
    );
    let parts = [
        raw_key_hint(&key_text("app.panel.nextTab"), "switch"),
        raw_key_hint(&up_down, "pick"),
        raw_key_hint(&key_text("app.panel.open"), "open"),
        raw_key_hint(&key_text("app.panel.cancel"), "cancel"),
        raw_key_hint(&key_text("app.panel.exit"), "back"),
    ];
    let line = parts.join(&t.fg("muted", "  "));
    truncate_to_width(&line, width, "…", false)
}

impl TaskPanelComponent {
    pub fn new() -> Self {
        Self {
            view: TaskPanelView::Plan,
            density: TaskPanelDensity::Full,
            disposed: false,
            ticking: false,
            model: RowModel::default(),
            focused: false,
            selected: None,
            confirming: None,
            events: Vec::new(),
        }
    }

    pub fn get_view(&self) -> TaskPanelView {
        self.view
    }

    pub fn set_view(&mut self, view: TaskPanelView) {
        self.view = view;
        self.selected = None;
    }

    /// `setDensity`: driven by the chrome, not by the pane.
    pub fn set_density(&mut self, density: TaskPanelDensity) {
        self.density = density;
    }

    /// `cycleView`: the next tab with rows, wrapping; an emptied view snaps
    /// back to the plan. The pick goes to the new tab's first row.
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
        self.selected = None;
        self.view
    }

    /// Whether the panel has any rows to focus. Syncs the model first.
    pub fn can_focus(&mut self) -> bool {
        self.model.sync(task_store());
        !self.model.views().is_empty()
    }

    /// Whether the panel has the keyboard.
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The row the pick is on, if any: its tab and task id.
    pub fn picked(&mut self) -> Option<(TaskPanelView, u64)> {
        self.model.sync(task_store());
        let view = self.shown_view();
        let id = resolve_selection(&self.model, view, self.selected)?;
        Some((view, id))
    }

    /// The events queued by key handling since the last call, in order.
    pub fn take_events(&mut self) -> Vec<PanelEvent> {
        std::mem::take(&mut self.events)
    }

    /// The tab a render shows: the stored one while it has rows, else the
    /// first tab that does (the model must be synced).
    fn shown_view(&self) -> TaskPanelView {
        let available = self.model.views();
        if available.contains(&self.view) {
            self.view
        } else {
            available.first().copied().unwrap_or(TaskPanelView::Plan)
        }
    }

    /// A running row is live: re-render every second for its run clock.
    pub fn ticking(&self) -> bool {
        self.ticking
    }

    /// Stop the run clock. Call on teardown.
    pub fn dispose(&mut self) {
        self.disposed = true;
        self.ticking = false;
    }

    /// Move the pick by `delta` rows within the shown tab. Ends clamp; nothing wraps.
    fn move_pick(&mut self, delta: isize) {
        let view = self.shown_view();
        let ids = self.model.row_ids(view);
        if ids.is_empty() {
            return;
        }
        // The pick as shown: a pick that is not set shows the first row.
        let current = resolve_selection(&self.model, view, self.selected)
            .and_then(|id| ids.iter().position(|row| *row == id))
            .unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, ids.len() as isize - 1) as usize;
        self.selected = Some(ids[next]);
    }

    /// `x`: the first press arms the cancel of a running subagent row, the
    /// second cancels it. Other rows have no cancel yet and ignore `x`.
    fn cancel_pick(&mut self, armed: Option<u64>) {
        let Some((_, id)) = self.picked() else {
            return;
        };
        let Some(task) = self.model.task(id) else {
            return;
        };
        // The event the second `x` queues: an Abort row performs its action;
        // a running subagent run goes through the pool. Anything else has no cancel.
        let confirmed = if task.action == Some(RowAction::Abort) {
            Some(PanelEvent::Perform {
                task_id: id,
                action: RowAction::Abort,
            })
        } else if task.kind() == TaskKind::Subagent && task.status.is_active() {
            task.agent
                .clone()
                .map(|run_id| PanelEvent::Cancel { run_id })
        } else {
            None
        };
        let Some(event) = confirmed else {
            return;
        };
        if armed == Some(id) {
            self.events.push(event);
        } else {
            self.confirming = Some(id);
        }
    }

    /// Enter: perform the picked row's action (Login), or open the row (the
    /// attach view's entry point). An Abort row is cancelled with `x`, not Enter.
    fn open_pick(&mut self) {
        let Some((view, task_id)) = self.picked() else {
            return;
        };
        let event = match self.model.task(task_id).and_then(|t| t.action) {
            None | Some(RowAction::Abort) => PanelEvent::Open { view, task_id },
            Some(action) => PanelEvent::Perform { task_id, action },
        };
        self.events.push(event);
    }

    /// Key handling while the panel has the keyboard.
    fn handle_key(&mut self, data: &str) {
        self.model.sync(task_store());
        let kb = get_keybindings();
        let Some(id) = PANEL_KEYS.iter().copied().find(|id| kb.matches(data, id)) else {
            return;
        };
        // Any key but `x` disarms a pending cancel. Esc only disarms when one is pending.
        let armed = self.confirming.take();
        match id {
            "app.panel.exit" if armed.is_some() => {}
            "app.panel.focus" | "app.panel.exit" => self.events.push(PanelEvent::Exit),
            "app.panel.nextTab" => {
                self.cycle_view(true);
            }
            "app.panel.prevTab" => {
                self.cycle_view(false);
            }
            "app.panel.up" => self.move_pick(-1),
            "app.panel.down" => self.move_pick(1),
            "app.panel.open" => self.open_pick(),
            "app.panel.cancel" => self.cancel_pick(armed),
            _ => {}
        }
    }
}

impl Component for TaskPanelComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.disposed {
            return Vec::new();
        }
        self.model.sync(task_store());
        let width = width as usize;
        let now = now_ms();
        if self.model.is_empty() {
            self.ticking = false;
            return Vec::new();
        }

        // Keep the stored view while it has rows, else the first tab that does.
        let view = self.shown_view();
        let selected = resolve_selection(&self.model, view, self.selected);
        self.selected = selected;
        self.ticking = self.model.any_running();

        let model = &self.model;
        let available = model.views();
        let t = theme();
        let gutter = format!(
            "{} ",
            t.fg(
                panel_state_color(model.lens(view).map(|(_, task)| task)),
                RAIL
            )
        );
        // Focus shows the strip even with one tab, the pick marker, and a hint
        // line. Focus also shows the rows in full, whatever the density.
        let focused = self.focused;
        let marker_width = if focused { 2 } else { 0 };
        let inner = width.saturating_sub(visible_width(RAIL) + 1 + marker_width);
        let tab_views: Vec<TaskPanelView> = if available.contains(&view) {
            available.to_vec()
        } else {
            available
                .iter()
                .copied()
                .chain(std::iter::once(view))
                .collect()
        };

        if self.density == TaskPanelDensity::Summary && !focused {
            return vec![gutter.clone() + &format_lens_tabs(model, inner, view, &tab_views, false)];
        }

        let mut lines = Vec::new();
        if available.len() >= 2 || focused {
            let hint = !focused;
            lines.push(gutter.clone() + &format_lens_tabs(model, inner, view, &tab_views, hint));
        }
        for (row, task) in model.lens(view) {
            let options = LineOptions {
                owner: model.owner_of(task),
                tree_prefix: &row.tree_prefix,
                confirm: self.confirming == Some(row.id),
            };
            let marker = match (focused, selected == Some(row.id)) {
                (false, _) => String::new(),
                (true, true) => t.fg("accent", "› "),
                (true, false) => "  ".to_string(),
            };
            lines.push(gutter.clone() + &marker + &format_task_line(task, inner, options, now));
        }
        if focused {
            lines.push(gutter.clone() + &focus_hint(inner));
        }
        lines
    }

    fn handle_input(&mut self, data: &str) {
        if is_key_release(data) {
            return;
        }
        self.handle_key(data);
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.confirming = None;
        }
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
    fn an_agent_without_rows_opens_no_tab() {
        let store = TaskStore::new();
        store.upsert_agent(
            "explore",
            "explore",
            TaskAgentKind::Subagent,
            TaskAgentPatch::default(),
        );
        let mut model = RowModel::default();
        model.sync(&store);
        assert!(model.views().is_empty(), "empty tabs are hidden");
    }

    #[test]
    fn a_shell_job_opens_only_the_shell_tab_between_agents_and_schedule() {
        let store = TaskStore::new();
        store.create(
            "sleep 9",
            CreateTaskOptions {
                source: Some(TaskSource::Shell),
                key: Some("shell:job-1".into()),
                ..Default::default()
            },
        );
        let mut model = RowModel::default();
        model.sync(&store);
        assert_eq!(model.views(), [TaskPanelView::Shell]);
        assert_eq!(
            TaskPanelView::ALL,
            [
                TaskPanelView::Plan,
                TaskPanelView::Subagents,
                TaskPanelView::Shell,
                TaskPanelView::Schedule,
                TaskPanelView::Mcp,
            ]
        );
        assert_eq!(TaskPanelView::Shell.label(), "Shell");
        assert_eq!(TaskPanelView::Shell.kind(), TaskKind::Shell);
    }

    #[test]
    fn an_mcp_call_opens_only_the_mcp_tab() {
        let store = TaskStore::new();
        store.create("fetch", mcp_options(Some("web")));
        let mut model = RowModel::default();
        model.sync(&store);
        assert_eq!(model.views(), [TaskPanelView::Mcp]);
    }

    fn mcp_options(server: Option<&str>) -> CreateTaskOptions {
        CreateTaskOptions {
            source: Some(TaskSource::Mcp),
            subagent_mode: server.map(String::from),
            ..Default::default()
        }
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
