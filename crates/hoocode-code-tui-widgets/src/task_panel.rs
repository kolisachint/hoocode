//! `components/task-panel.ts`: the task ledger above the prompt.
//!
//! Declutter spec (Dialogs, Tasks): the panel is a pane above the prompt.
//! Unfocused it has no title: rows only, with one dim strip row first when
//! more than one tab has rows. The strip names the shown tab whole, folds
//! the rest into `+N tabs`, and pins the hint `ctrl+x l next tab · ctrl+x g
//! hide` right (the hint drops `hide`, then `next tab`, before any tab label
//! is cut). Focused it gets the `Tasks` chip with `/tasks · ctrl+x g` and the
//! strip after it, the pick marker and the hint row. Row glyphs: `◉` working,
//! `○` queued, `●` done, `⊗` failed or cancelled. It draws the same at every
//! density.
//!
//! The panel is shown by default. The leader `ctrl+x g` toggles it. With no
//! rows it draws nothing.
//!
//! One tab per kind of task, in tab order (see [`TaskPanelView`]); a tab
//! with no rows is hidden.
//! - Plan: the main agent's TodoWrite plan, each dispatched run nested under
//!   the plan item it works on;
//! - Agents: the subagent forest (`parentTaskId`), drawn with connectors;
//! - Shell: background shell jobs, one row each;
//! - Schedule: scheduled jobs, one row each;
//! - MCP: the MCP calls, flat.
//!
//! The prompt keeps the keyboard. Up on the prompt's first line hands it to the
//! panel on the bottom row ([`TaskPanelComponent::enter_from_below`]). Then the
//! pick moves with Up and Down, Left and Right (or Tab) switch tabs, and Down
//! past the last row, Esc or `ctrl+x g` returns to the prompt. A printable key
//! the panel does not use returns to the prompt too, and is typed there.
//! Enter opens the pick, and `x` (twice) cancels a subagent run. What the app
//! must do about those is queued as [`PanelEvent`]s, read with
//! [`TaskPanelComponent::take_events`].
//!
//! The rows come from a [`RowModel`], rebuilt only when the store's
//! `version()` moves, so a frame copies nothing from the store. Its 1s run
//! clock is [`TaskPanelComponent::ticking`]: the app re-renders every second
//! while it is true.

use hoocode_code_tui_keybindings::{key_hint_text, key_pair_hint};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use hoocode_code_agent_session::format::{format_duration_secs, format_tokens};
use hoocode_code_settings::SettingsManager;
use hoocode_code_task_store::{
    task_store, RowAction, Task, TaskAgent, TaskKind, TaskStatus, TaskStore,
};
use hoocode_code_tui_theme::agent_color_for;
use hoocode_tui_glyphs::status;
use hoocode_tui_keys::{get_keybindings, is_key_release};
use hoocode_tui_ratatui::{truncate_line, Line, Palette, Span, Style};
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_at_word, truncate_to_width, visible_width};

use crate::pane::{fit_hint_line, Pane, PaneStyle};

fn task_status_icon(status: TaskStatus) -> &'static str {
    // Mission Control circles (spec §2): hollow = not started, fisheye =
    // running, solid = done, crossed = failed, barred = cancelled. The
    // half circle is missing from the font and falls back off-baseline.
    match status {
        TaskStatus::Pending => status::TODO_STR,
        TaskStatus::InProgress => status::RUNNING_STR,
        TaskStatus::Done => status::DONE_STR,
        TaskStatus::Failed => "⊗",
        TaskStatus::Cancelled => "⊗",
    }
}

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
    /// Back to the prompt: Esc, Down past the last row, or ctrl+x g (which also
    /// hides the panel).
    Exit,
    /// A printable key the panel does not use: the keyboard goes back to the
    /// prompt, and the key is typed into it.
    Type(String),
}

/// A single printable character (no control character, no escape sequence).
fn is_printable(data: &str) -> bool {
    let mut chars = data.chars();
    matches!((chars.next(), chars.next()), (Some(ch), None) if !ch.is_control())
}

/// A row with a run clock: every row that is not a plan item (a subagent run, an
/// MCP call, a shell job or a scheduled job). A running one shows how long it has run.
fn is_delegated(task: &Task) -> bool {
    task.kind() != TaskKind::Plan
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

/// One row of a lens. Its identity is the task's store id, which survives a
/// rebuild; the tree connector comes from the lens shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelRow {
    pub id: u64,
    pub tree_prefix: String,
}

/// Done and total counts of one lens (its tab), and how many of its rows run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LensStats {
    done: usize,
    total: usize,
    running: usize,
}

impl LensStats {
    /// A tab's count after its name. The plan shows done/total. A live tab shows
    /// how many of its rows run while any does, else done/total.
    fn label(self, view: TaskPanelView) -> String {
        if view != TaskPanelView::Plan && self.running > 0 {
            format!("{} {} running", view.label(), self.running)
        } else {
            format!("{} {}/{}", view.label(), self.done, self.total)
        }
    }
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
            // The plan counts its items only; the runs nested under them are not.
            .filter(|(_, task)| view != TaskPanelView::Plan || task.is_plan_item())
            .fold(LensStats::default(), |mut stats, (_, task)| {
                stats.total += 1;
                stats.done += usize::from(task.status == TaskStatus::Done);
                stats.running += usize::from(task.status == TaskStatus::InProgress);
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

/// The unfocused strip's hint, by how much of it is left. Level 0 names the
/// cycle key and the hide key; level 1 drops `hide`; level 2 drops both. The
/// strip drops hints a level at a time before it cuts a tab label.
fn strip_hint_items(level: usize) -> Vec<(String, &'static str)> {
    let cycle = (key_hint_text("app.tasks.cycleForward"), "next tab");
    let hide = (key_hint_text("app.panel.focus"), "hide");
    match level {
        0 => vec![cycle, hide],
        1 => vec![cycle],
        _ => Vec::new(),
    }
}

/// The hint's spans: keys in normal ink, action words dim, ` · ` between items.
fn strip_hint_spans(items: &[(String, &'static str)]) -> Vec<Span<'static>> {
    let st = PaneStyle::current();
    let mut spans = Vec::new();
    for (i, (keys, action)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", st.dim_style()));
        }
        spans.push(Span::styled(keys.clone(), st.text_style()));
        spans.push(Span::styled(format!(" {action}"), st.dim_style()));
    }
    spans
}

fn strip_hint_width(items: &[(String, &'static str)]) -> usize {
    strip_hint_spans(items).iter().map(|s| s.width()).sum()
}

/// The tab strip: the shown tab in normal text, the other tabs dim, and the
/// hint pinned right when `hinted`. At narrow widths the hint drops a level
/// at a time first, then the trailing tabs collapse into `+N tabs`
/// (`plan 1/3 · +3 tabs`), always keeping the shown tab visible; what still
/// overflows the pane cuts.
fn tab_strip(model: &RowModel, view: TaskPanelView, hinted: bool, width: usize) -> Line<'static> {
    let st = PaneStyle::current();
    let tabs = model.views();
    let labels: Vec<String> = tabs
        .iter()
        .map(|tab| model.lens_stats(*tab).label(*tab).to_lowercase())
        .collect();
    let shown = tabs.iter().position(|tab| *tab == view).unwrap_or(0);
    // Cells for the strip content: the pane keeps a 2-column pad and the
    // unfocused strip adds its own 2-column indent.
    let budget = width.saturating_sub(4);
    // The fullest hint level that fits with every tab whole. When none does,
    // no hint, and the tabs collapse.
    let hint_items = if hinted {
        (0..=2)
            .map(strip_hint_items)
            .find(|items| strip_fits(&labels, labels.len(), strip_hint_width(items), budget))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let hint_w = strip_hint_width(&hint_items);
    // The leading tabs shown whole; the rest collapse into `+N tabs`.
    let mut shown_tabs = labels.len();
    while shown_tabs > shown + 1 && !strip_fits(&labels, shown_tabs, hint_w, budget) {
        shown_tabs -= 1;
    }
    let mut spans = Vec::new();
    for (i, label) in labels.iter().enumerate().take(shown_tabs) {
        if i > 0 {
            spans.push(Span::styled(" · ", st.dim_style()));
        }
        spans.push(Span::styled(
            label.clone(),
            if tabs[i] == view {
                st.text_style()
            } else {
                st.dim_style()
            },
        ));
    }
    let hidden = labels.len().saturating_sub(shown_tabs);
    if hidden > 0 {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", st.dim_style()));
        }
        spans.push(Span::styled(format!("+{hidden} tabs"), st.dim_style()));
    }
    if hint_w > 0 {
        let used: usize = spans.iter().map(|s| s.width()).sum();
        let gap = budget.checked_sub(used + hint_w).unwrap_or(1);
        spans.push(Span::raw(" ".repeat(gap)));
        spans.extend(strip_hint_spans(&hint_items));
    }
    Line::from(spans)
}

/// Whether `shown_tabs` leading tabs (plus `+N tabs` for the rest, and a hint
/// `hint_w` cells wide when there is one) fit in `budget` cells with at least
/// one space of pin gap.
fn strip_fits(labels: &[String], shown_tabs: usize, hint_w: usize, budget: usize) -> bool {
    let mut used = 0;
    for (i, label) in labels.iter().enumerate().take(shown_tabs) {
        if i > 0 {
            used += visible_width(" · ");
        }
        used += visible_width(label);
    }
    let hidden = labels.len().saturating_sub(shown_tabs);
    if hidden > 0 {
        if shown_tabs > 0 {
            used += visible_width(" · ");
        }
        used += visible_width(&format!("+{hidden} tabs"));
    }
    if hint_w > 0 {
        used += 1 + hint_w;
    }
    used <= budget
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

/// The status circle's token style in the flight plan. Only the glyph is
/// coloured: a working row reads accent, a done row sits back dim (yellow
/// is for blocking only).
fn flight_status_style(status: TaskStatus) -> Style {
    let pal = Palette::current();
    match status {
        TaskStatus::InProgress => pal.acc(),
        TaskStatus::Done => pal.dim(),
        TaskStatus::Failed => pal.err(),
        // Not started reads faint (spec §3.1: `○`); a user-initiated
        // cancel is dim, not an error.
        TaskStatus::Pending => pal.faint(),
        TaskStatus::Cancelled => pal.dim(),
    }
}

/// A row's facts plus their style: the waypoint's elapsed time and run
/// facts, the usage count, the next firing, a warning note, or the
/// armed-cancel ask. Plain strings, laid out left to right by [`flight_row`];
/// only the armed-cancel ask still pins right.
fn flight_detail(
    task: &Task,
    owner: Option<&TaskAgent>,
    confirm: bool,
    now: u64,
    scoped_aliases: &[(String, Option<String>)],
) -> Option<(Vec<String>, Style)> {
    let pal = Palette::current();
    if confirm {
        let word = if task.kind() == TaskKind::Shell {
            "stop"
        } else {
            "cancel"
        };
        return Some((vec![format!("press x again to {word}")], pal.warn()));
    }
    if let Some(note) = &task.note {
        // A schedule row's failed run still says when the job fires next.
        let next = schedule_next_in(task, now)
            .map(|next| format!(" · {next}"))
            .unwrap_or_default();
        let style = if task.status == TaskStatus::Failed {
            pal.err()
        } else {
            pal.warn()
        };
        return Some((vec![format!("{note}{next}")], style));
    }
    // A running shell job is a satellite (spec §4 r9/c7). It is spanned in
    // `flight_row`, not painted here: painting over `paint` output would
    // mangle the inner escapes.
    match task.status {
        // A schedule row keeps showing when its job fires next, after a run too.
        TaskStatus::Done | TaskStatus::Failed if schedule_next_in(task, now).is_some() => {
            let word = if task.status == TaskStatus::Done {
                "done"
            } else {
                "failed"
            };
            let next = schedule_next_in(task, now).unwrap_or_default();
            return Some((vec![word.to_string(), next], pal.dim()));
        }
        TaskStatus::Done | TaskStatus::Failed => {
            let total: f64 = task
                .usage
                .as_ref()
                .map(|u| u.input + u.output)
                .unwrap_or(0.0);
            if total > 0.0 {
                return Some((
                    vec![format!("{} tokens", format_tokens(total as u64))],
                    pal.faint(),
                ));
            }
        }
        TaskStatus::InProgress => {
            // No live activity: the row reads type, title, then facts
            // (`explore  Trace the auth flow  12s · muse · 3:12 left`).
            let mut facts: Vec<String> = Vec::new();
            if is_delegated(task) {
                facts.push(format_duration_secs(task_elapsed_secs(task, now)));
            }
            facts.extend(describe_run(owner, now, scoped_aliases));
            if !facts.is_empty() {
                return Some((facts, pal.faint()));
            }
        }
        TaskStatus::Pending => {
            if let Some(next) = schedule_next_in(task, now) {
                return Some((vec![next], pal.dim()));
            }
        }
        TaskStatus::Cancelled => return Some((vec!["cancelled".to_string()], pal.dim())),
    }
    None
}

/// `next in 4m` for a schedule row that is not running and whose next fire is
/// ahead; `None` for any other row.
fn schedule_next_in(task: &Task, now: u64) -> Option<String> {
    if task.kind() != TaskKind::Schedule || task.status == TaskStatus::InProgress {
        return None;
    }
    let next = task.next_at.filter(|next| *next > now)?;
    Some(format!(
        "next in {}",
        format_duration_secs(((next - now) / 1000) as f64)
    ))
}

/// A flight row's facts after two spaces: drop facts last-first (deadline,
/// then model, then clock) while the row is too wide, then cut the title at
/// a word boundary. Facts never wrap; the row never exceeds `inner`.
fn append_flight_facts(
    row: &mut Line<'static>,
    title: String,
    title_style: Style,
    facts: &[String],
    fact_style: Style,
    inner: usize,
) {
    let left_width: usize = row.spans.iter().map(|s| s.width()).sum();
    let mut kept = facts.len();
    while kept > 0 {
        let width =
            left_width + visible_width(&title) + 2 + visible_width(&facts[..kept].join(" · "));
        if width <= inner {
            break;
        }
        kept -= 1;
    }
    let mut budget = inner.saturating_sub(left_width);
    if kept > 0 {
        budget = budget.saturating_sub(2 + visible_width(&facts[..kept].join(" · ")));
    }
    row.spans
        .push(Span::styled(truncate_at_word(&title, budget), title_style));
    if kept > 0 {
        row.spans.push(Span::raw("  "));
        row.spans
            .push(Span::styled(facts[..kept].join(" · "), fact_style));
    }
    // A pathological left side (a tag wider than `inner`) is cut hard.
    let width: usize = row.spans.iter().map(|s| s.width()).sum();
    if width > inner {
        *row = truncate_line(row, inner);
    }
}

/// The right detail of a row, laid out like every other one: the row keeps
/// the room the detail needs, then a gap, then the detail spans.
fn append_detail(row: &mut Line<'static>, detail: Vec<Span<'static>>, inner: usize) {
    use hoocode_tui_ratatui::truncate_spans;
    let detail = truncate_spans(detail, inner, "…");
    let detail_width: usize = detail.iter().map(|s| s.width()).sum();
    let max_left = inner.saturating_sub(detail_width + 1).max(1);
    *row = truncate_line(row, max_left);
    let used: usize = row.spans.iter().map(|s| s.width()).sum();
    let gap = inner.saturating_sub(used + detail_width).max(1);
    row.spans.push(Span::raw(" ".repeat(gap)));
    row.spans.extend(detail);
}
/// One flight-plan row (content only; the console adds the `›` marker or
/// gutter and the selection band): the status circle, the lowercase type tag
/// where it disambiguates, the title, then two spaces and the facts, for
/// example `◉ explore  Trace the auth flow  12s · muse · 3:12 left`. Narrow
/// rows drop facts last-first (deadline, model, clock); the title cuts last.
/// Only the armed-cancel ask still pins right.
fn flight_row(
    task: &Task,
    owner: Option<&TaskAgent>,
    tree_prefix: &str,
    confirm: bool,
    inner: usize,
    now: u64,
    scoped_aliases: &[(String, Option<String>)],
) -> Line<'static> {
    let pal = Palette::current();
    let mut spans = vec![
        Span::styled(
            task_status_icon(task.status),
            flight_status_style(task.status),
        ),
        Span::raw(" "),
    ];
    if !tree_prefix.is_empty() {
        spans.push(Span::styled(tree_prefix.to_string(), pal.faint()));
    }
    let is_mcp = task.kind() == TaskKind::Mcp;
    // An agent or MCP row names its type first, lowercased, then two spaces:
    // `explore  Trace the auth flow`.
    let tag = if is_mcp {
        Some((
            task.subagent_mode.as_deref().unwrap_or("mcp").to_string(),
            pal.style("mcp"),
        ))
    } else {
        task.subagent_mode
            .as_ref()
            .map(|mode| (mode.to_lowercase(), pal.style(agent_color_for(mode))))
    };
    if let Some((tag, style)) = tag {
        spans.push(Span::styled(tag, style));
        spans.push(Span::raw("  "));
    }
    // Normal text, except finished and cancelled rows, which sit back (dim).
    let title_style = match task.status {
        TaskStatus::Done | TaskStatus::Cancelled => pal.dim(),
        TaskStatus::Failed | TaskStatus::InProgress | TaskStatus::Pending => Style::default(),
    };
    let title = task.title.clone();
    // The armed-cancel ask keeps its right-pinned read.
    if confirm {
        let mut row = Line::from(spans);
        row.spans.push(Span::styled(title, title_style));
        if let Some((facts, fact_style)) = flight_detail(task, owner, confirm, now, scoped_aliases)
        {
            let detail = truncate_to_width(&facts.join(" · "), inner, "…", false);
            append_detail(&mut row, vec![Span::styled(detail, fact_style)], inner);
        }
        return row;
    }
    let mut row = Line::from(spans);
    let Some((facts, fact_style)) = flight_detail(task, owner, false, now, scoped_aliases) else {
        row.spans.push(Span::styled(title, title_style));
        return row;
    };
    append_flight_facts(&mut row, title, title_style, &facts, fact_style, inner);
    row
}

/// The most rows the console shows at once. A short terminal shows fewer:
/// a quarter of its height, at least one row.
pub const PANEL_MAX_ROWS: usize = 6;

/// The row cap for a terminal `screen_rows` tall: [`PANEL_MAX_ROWS`], or a
/// quarter of the height when that is less.
fn row_cap(screen_rows: u16) -> usize {
    (usize::from(screen_rows) / 4).clamp(1, PANEL_MAX_ROWS)
}

/// The task pane: the tab strip (a dim first row unfocused, after the
/// `Tasks` chip focused), one row per row of the shown tab, and the hint row
/// while the keyboard is in the panel. Rows beyond `max_rows` are windowed
/// around the pick, and a `N more` row says how many are hidden.
#[allow(clippy::too_many_arguments)]
fn render_flight_plan(
    model: &RowModel,
    view: TaskPanelView,
    selected: Option<u64>,
    confirming: Option<u64>,
    focused: bool,
    width: usize,
    now: u64,
    max_rows: usize,
    scoped_aliases: &[(String, Option<String>)],
) -> Vec<String> {
    let st = PaneStyle::current();
    let ids = model.row_ids(view);
    // Rows sit behind the pane's 3-column gutter and a 1-column margin.
    let row_inner = width.saturating_sub(4);
    let mut pane = if focused {
        let mut pane = Pane::new("Tasks");
        pane.command = format!(
            "/tasks · {}",
            hoocode_code_tui_keybindings::key_hint_text("app.panel.focus")
        );
        pane.facts = tab_strip(model, view, false, width);
        pane.selected = selected.and_then(|id| ids.iter().position(|row| *row == id));
        let owned = pick_hints(model, selected);
        let refs: Vec<(&str, &str)> = owned.iter().map(|(k, a)| (k.as_str(), *a)).collect();
        pane.hint = fit_hint_line(&refs, false, width);
        pane
    } else {
        let mut pane = Pane::new("");
        // One tab needs no strip; several get one dim row first.
        if model.views().len() > 1 {
            let mut spans = vec![Span::raw("  ")];
            spans.extend(tab_strip(model, view, true, width).spans);
            pane.top.push(Line::from(spans));
        }
        pane
    };
    pane.rows = model
        .lens(view)
        .map(|(row, task)| {
            flight_row(
                task,
                model.owner_of(task),
                &row.tree_prefix,
                confirming == Some(row.id),
                row_inner,
                now,
                scoped_aliases,
            )
        })
        .collect();
    pane.max_visible = Some(max_rows);
    let hidden = pane.rows.len().saturating_sub(max_rows);
    if hidden > 0 {
        pane.bottom.push(Line::from(Span::styled(
            format!("  {hidden} more"),
            st.dim_style(),
        )));
    }
    pane.style = Some(st);
    pane.render(width)
}

/// Whether `x` can cancel a row: an Abort row (a running shell job, an MCP call
/// in flight), or an active subagent run. The panel's `cancel_pick` queues the
/// cancel for the same rows.
fn is_cancellable(task: &Task) -> bool {
    task.action == Some(RowAction::Abort)
        || (task.kind() == TaskKind::Subagent && task.status.is_active() && task.agent.is_some())
}

/// The hint foot's keys for the picked row: the moves, then only what Enter and
/// `x` do on that row (the app's `open_task_panel_row`, the panel's `cancel_pick`).
/// A plan item or a schedule job has neither, so its foot names none.
fn pick_hints(model: &RowModel, picked: Option<u64>) -> Vec<(String, &'static str)> {
    let open = key_hint_text("app.panel.open");
    let mut keys = vec![(key_pair_hint("app.panel.up", "app.panel.down"), "move")];
    // Left and Right move between tabs only when there is more than one.
    if model.views().len() > 1 {
        keys.push((
            key_pair_hint("app.panel.prevTab", "app.panel.nextTab"),
            "tab",
        ));
    }
    if let Some(task) = picked.and_then(|id| model.task(id)) {
        if task.action == Some(RowAction::Login) {
            keys.push((open.clone(), "log in"));
        } else if task.kind() == TaskKind::Subagent && task.agent.is_some() {
            keys.push((open.clone(), "open its log"));
        } else if task.kind() == TaskKind::Shell && task.key.is_some() {
            keys.push((open.clone(), "open output"));
        }
        if is_cancellable(task) {
            let word = if task.kind() == TaskKind::Shell {
                "stop"
            } else {
                "cancel"
            };
            keys.push((key_hint_text("app.panel.cancel"), word));
        }
    }
    keys.push((key_hint_text("app.panel.exit"), "back"));
    keys
}

/// `TaskPanelComponent`.
pub struct TaskPanelComponent {
    view: TaskPanelView,
    /// The user's ctrl+x g choice; `None` shows the panel.
    shown_override: Option<bool>,
    disposed: bool,
    ticking: bool,
    model: RowModel,
    /// The panel has the keyboard (handed over from the prompt).
    focused: bool,
    /// The picked row's id. Kept by id, so a rebuild keeps the pick.
    selected: Option<u64>,
    /// The row whose cancel is armed (one `x` pressed).
    confirming: Option<u64>,
    /// Queued for the app; see [`PanelEvent`].
    events: Vec<PanelEvent>,
    /// The terminal's height in rows, set by the app ([`TaskPanelComponent::set_screen_rows`]).
    screen_rows: u16,
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

/// The terminal height assumed until the app sets one.
const DEFAULT_SCREEN_ROWS: u16 = 24;

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

impl TaskPanelComponent {
    pub fn new() -> Self {
        Self {
            view: TaskPanelView::Plan,
            shown_override: None,
            disposed: false,
            ticking: false,
            model: RowModel::default(),
            focused: false,
            selected: None,
            confirming: None,
            events: Vec::new(),
            screen_rows: DEFAULT_SCREEN_ROWS,
        }
    }

    /// The terminal's height. The console shows at most [`PANEL_MAX_ROWS`]
    /// rows, fewer on a short terminal.
    pub fn set_screen_rows(&mut self, rows: u16) {
        self.screen_rows = rows;
    }

    pub fn get_view(&self) -> TaskPanelView {
        self.view
    }

    pub fn set_view(&mut self, view: TaskPanelView) {
        self.view = view;
        self.selected = None;
    }

    /// Whether the panel is drawn: the user's ctrl+x g choice, else shown.
    pub fn is_shown(&self) -> bool {
        self.shown_override.unwrap_or(true)
    }

    /// ctrl+x g: flips shown and hidden for this view. Returns the new state.
    pub fn toggle_shown(&mut self) -> bool {
        let shown = !self.is_shown();
        self.shown_override = Some(shown);
        shown
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

    /// Left or Right from inside the panel: the next tab, with the pick on its
    /// last row, so Down returns to the prompt from there.
    fn cycle_from_pick(&mut self, forward: bool) {
        self.cycle_view(forward);
        self.selected = self.model.row_ids(self.shown_view()).last().copied();
    }

    /// Whether the panel has any rows to focus. Syncs the model first.
    pub fn can_focus(&mut self) -> bool {
        self.model.sync(task_store());
        !self.model.views().is_empty()
    }

    /// Whether Up on the prompt's first line may hand the keyboard to the panel:
    /// shown and with rows. The app adds its own scroll check.
    pub fn can_take_up(&mut self) -> bool {
        self.is_shown() && self.can_focus()
    }

    /// Up on the prompt's first line: the keyboard goes to the panel, on its
    /// bottom row. `false` when the panel is hidden or has no rows, so the
    /// prompt keeps the key.
    pub fn enter_from_below(&mut self) -> bool {
        self.model.sync(task_store());
        if !self.is_shown() {
            return false;
        }
        let view = self.shown_view();
        let Some(last) = self.model.row_ids(view).last().copied() else {
            return false;
        };
        self.selected = Some(last);
        self.confirming = None;
        true
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

    /// Takes back a queued [`PanelEvent::Exit`], so the app can return the
    /// keyboard at once, before the next key of the same input batch.
    pub fn take_exit(&mut self) -> bool {
        let before = self.events.len();
        self.events.retain(|event| *event != PanelEvent::Exit);
        self.events.len() != before
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

    /// Whether the panel draws rows this pass: shown, with rows. The
    /// notification band joins the panel's pane while this is true.
    pub fn is_drawn(&mut self) -> bool {
        self.model.sync(task_store());
        self.is_shown() && !self.model.views().is_empty()
    }

    /// Takes back the first queued [`PanelEvent::Type`], so the app can type
    /// the key into the prompt before the next key of the same input batch.
    pub fn take_typed(&mut self) -> Option<String> {
        let at = self
            .events
            .iter()
            .position(|event| matches!(event, PanelEvent::Type(_)))?;
        match self.events.remove(at) {
            PanelEvent::Type(text) => Some(text),
            _ => None,
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

    /// Whether the pick is on the shown tab's last row: Down there returns to
    /// the prompt.
    fn pick_is_last(&self) -> bool {
        let view = self.shown_view();
        let ids = self.model.row_ids(view);
        resolve_selection(&self.model, view, self.selected).is_none_or(|id| ids.last() == Some(&id))
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

    /// Whether row `id` is in the model and `x` can still cancel it.
    fn is_cancellable_row(&self, id: u64) -> bool {
        self.model.task(id).is_some_and(is_cancellable)
    }

    /// Key handling while the panel has the keyboard.
    fn handle_key(&mut self, data: &str) {
        self.model.sync(task_store());
        let kb = get_keybindings();
        let Some(id) = PANEL_KEYS.iter().copied().find(|id| kb.matches(data, id)) else {
            // A bare printable key is not the panel's: it goes back to the
            // prompt and is typed there, so no keystroke is lost.
            if is_printable(data) {
                self.confirming = None;
                self.events.push(PanelEvent::Type(data.to_string()));
            }
            return;
        };
        // Any key but `x` disarms a pending cancel. Esc only disarms when one is pending.
        // An arm on a row that is no longer cancellable is stale: it does not count.
        let armed = self
            .confirming
            .take()
            .filter(|id| self.is_cancellable_row(*id));
        match id {
            // ctrl+x g hides the panel, and a hidden panel gives the keyboard back.
            "app.panel.focus" => {
                if !self.toggle_shown() {
                    self.events.push(PanelEvent::Exit);
                }
            }
            "app.panel.exit" if armed.is_some() => {}
            "app.panel.exit" => self.events.push(PanelEvent::Exit),
            "app.panel.up" => self.move_pick(-1),
            "app.panel.down" => {
                if self.pick_is_last() {
                    self.events.push(PanelEvent::Exit);
                } else {
                    self.move_pick(1);
                }
            }
            "app.panel.prevTab" => self.cycle_from_pick(false),
            "app.panel.nextTab" => self.cycle_from_pick(true),
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
        // No rows, or hidden: nothing to draw and no clock to run.
        if self.model.views().is_empty() || !self.is_shown() {
            self.ticking = false;
            return Vec::new();
        }
        let now = now_ms();
        // Keep the stored view while it has rows, else the first tab that does.
        let view = self.shown_view();
        let selected = resolve_selection(&self.model, view, self.selected);
        self.selected = selected;
        self.ticking = self.model.any_running();
        self.confirming = self.confirming.filter(|id| self.is_cancellable_row(*id));
        render_flight_plan(
            &self.model,
            view,
            selected,
            self.confirming,
            self.focused,
            width as usize,
            now,
            row_cap(self.screen_rows),
            saved_scoped_aliases(),
        )
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

/// The saved `scopedModels` entries the widget matches models against:
/// `(pattern, alias)`, in saved order.
static SCOPED_MODEL_ALIASES: OnceLock<Vec<(String, Option<String>)>> = OnceLock::new();

/// The saved entries, read once per process from the global settings and the
/// project settings (the project list replaces the global one). A settings
/// change mid-run shows after a restart. Read once at the render boundary
/// and threaded through as a parameter, so the row builders below stay pure
/// functions of their inputs (and tests pin hermetic entries instead).
fn saved_scoped_aliases() -> &'static Vec<(String, Option<String>)> {
    SCOPED_MODEL_ALIASES.get_or_init(|| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        SettingsManager::create_default(&cwd)
            .scoped_models()
            .unwrap_or_default()
            .into_iter()
            .map(|entry| (entry.model, entry.alias))
            .collect()
    })
}

/// The short name for a run's model: the saved `scopedModels` alias when an
/// entry names this `provider/id`, else the last path segment, so
/// `opencode-go/muse-spark-1.3-contributor` reads `muse-spark-1.3-contributor`.
/// The widget cannot see the app's concrete scoped list, so it matches the
/// saved entries itself: an exact entry first, then the first glob entry with
/// an alias, so a later entry still fills in the alias the first left unset.
/// `[...]` classes are not matched (exact entries only). The entries come
/// from the caller (the component passes [`saved_scoped_aliases`]), so this
/// stays a pure function of its inputs.
fn model_alias(model: &str, entries: &[(String, Option<String>)]) -> String {
    if let Some(alias) = scoped_alias_for(model, entries) {
        return alias;
    }
    model.rsplit('/').next().unwrap_or(model).to_string()
}

/// The first alias among `entries` whose pattern names `model`, if any.
fn scoped_alias_for(model: &str, entries: &[(String, Option<String>)]) -> Option<String> {
    entries
        .iter()
        .filter(|(pattern, _)| {
            pattern.eq_ignore_ascii_case(model) || scoped_glob_matches(pattern, model)
        })
        .filter_map(|(_, alias)| alias.clone())
        .find(|alias| !alias.is_empty())
}

/// A case-insensitive glob match for model references: `*` stays within a `/`
/// segment, `?` takes one character but not `/`, `**` crosses segments.
fn scoped_glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    glob_inner(&pattern, &text)
}

fn glob_inner(pattern: &[char], text: &[char]) -> bool {
    let Some((&head, rest)) = pattern.split_first() else {
        return text.is_empty();
    };
    match head {
        '*' => {
            let cross = rest.first() == Some(&'*');
            let rest = if cross { &rest[1..] } else { rest };
            let mut index = 0;
            loop {
                if glob_inner(rest, &text[index..]) {
                    return true;
                }
                if index >= text.len() {
                    return false;
                }
                if !cross && text[index] == '/' {
                    return false;
                }
                index += 1;
            }
        }
        '?' => match text.split_first() {
            Some((&first, tail)) if first != '/' => glob_inner(rest, tail),
            _ => false,
        },
        want => match text.split_first() {
            Some((&first, tail)) if first == want => glob_inner(rest, tail),
            _ => false,
        },
    }
}

/// `attempt 2 · muse · 3:12 left` — the facts that explain a run, highest
/// priority first. The outcome replaces them once the run has one.
fn describe_run(
    owner: Option<&TaskAgent>,
    now: u64,
    scoped_aliases: &[(String, Option<String>)],
) -> Vec<String> {
    let Some(owner) = owner else {
        return Vec::new();
    };
    if let Some(outcome) = owner.outcome.as_deref().filter(|o| !o.is_empty()) {
        let mut line = outcome.to_string();
        if let Some(confidence) = owner.confidence {
            line.push_str(&format!(" · {confidence:.1}"));
        }
        if let Some(cause) = owner.cause.as_deref().filter(|c| !c.is_empty()) {
            let one_line: String = cause.split_whitespace().collect::<Vec<_>>().join(" ");
            line.push_str(&format!(
                " · {}",
                truncate_to_width(&one_line, 48, "…", false)
            ));
        }
        return vec![line];
    }
    let mut facts: Vec<String> = Vec::new();
    if let Some(attempt) = owner.attempt.filter(|n| *n > 1) {
        facts.push(format!("attempt {attempt}"));
    }
    if let Some(model) = owner.model.as_deref().filter(|m| !m.is_empty()) {
        facts.push(model_alias(model, scoped_aliases));
    }
    if let Some(deadline) = owner.deadline_at.filter(|d| *d > now) {
        facts.push(format!(
            "{} left",
            format_duration_secs(((deadline - now) / 1000) as f64)
        ));
    }
    facts
}

#[cfg(test)]
mod row_model_tests {
    use super::*;
    use hoocode_code_task_store::{
        CreateTaskOptions, TaskAgentKind, TaskAgentPatch, TaskPatch, TaskSource,
    };

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
    fn lens_stats_count_done_and_total_for_each_lens() {
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
        assert_eq!((plan_stats.done, plan_stats.total), (1, 2));
        let sub_stats = model.lens_stats(TaskPanelView::Subagents);
        assert_eq!((sub_stats.done, sub_stats.total), (1, 1));
    }

    #[test]
    fn an_unknown_row_id_has_no_task() {
        let store = TaskStore::new();
        plan(&store, "one");
        let mut model = RowModel::default();
        model.sync(&store);
        assert!(model.task(999).is_none());
    }

    fn scoped(pattern: &str, alias: Option<&str>) -> (String, Option<String>) {
        (pattern.to_string(), alias.map(str::to_string))
    }

    #[test]
    fn scoped_alias_prefers_the_first_alias_that_names_the_model() {
        let entries = vec![
            scoped("other/model", Some("other")),
            scoped("mock/pinned", None),
            scoped("mock/*", Some("mocked")),
        ];
        // An exact entry without an alias does not block the later glob's.
        assert_eq!(
            scoped_alias_for("mock/pinned", &entries).as_deref(),
            Some("mocked")
        );
        assert_eq!(
            scoped_alias_for("mock/other", &entries).as_deref(),
            Some("mocked")
        );
        assert_eq!(scoped_alias_for("elsewhere/thing", &entries), None);
    }

    #[test]
    fn scoped_glob_stays_within_a_segment_unless_doubled() {
        assert!(scoped_glob_matches("mock/*", "mock/pinned"));
        assert!(!scoped_glob_matches("mock/*", "mock/a/b"));
        assert!(scoped_glob_matches("mock/**", "mock/a/b"));
        assert!(scoped_glob_matches("*/pinned", "MOCK/pinned"));
        assert!(!scoped_glob_matches("mock/p?n", "mock/pinned"));
        assert!(scoped_glob_matches("mock/p?n", "mock/pin"));
    }

    #[test]
    fn model_alias_uses_the_entries_then_falls_back_to_the_tail() {
        // Hermetic entries: no saved settings are read, so the alias and
        // the fallback chain stay pinned whatever the machine holds.
        let entries = vec![scoped("mock/*", Some("mocked"))];
        assert_eq!(model_alias("mock/pinned", &entries), "mocked");
        assert_eq!(
            model_alias("test-fixture-xyzzy/test-model-xyzzy", &entries),
            "test-model-xyzzy"
        );
        assert_eq!(
            model_alias("test-fixture-xyzzy/test-model-xyzzy", &[]),
            "test-model-xyzzy"
        );
    }

    #[test]
    fn flight_titles_cut_at_a_word_with_an_ellipsis() {
        assert_eq!(truncate_at_word("map the repo", 12), "map the repo");
        assert_eq!(truncate_at_word("map the repo", 7), "map…");
        assert_eq!(truncate_at_word("map the repo", 0), "");
        // No space in budget: a hard cut is the only fit.
        assert_eq!(truncate_at_word("supercalifragilistic", 5), "supe…");
    }
}
