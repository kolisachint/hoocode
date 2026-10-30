//! The session picker (`components/session-selector.ts`): the sessions of the
//! current folder or of every folder, threaded by fork parent or searched,
//! with rename and delete.
//!
//! The MISSION LOG console (M7, ratatui): `MISSION LOG · /resume`, scope
//! tabs on top, the `▸` query row, one row per session with `›` on the
//! selection band, and the hint foot. Shared `Console` chrome in
//! `code-tui-widgets/src/console.rs`; keys, filtering and events unchanged.
//!
//! Loading is asynchronous in the pin. Here a loader receives a [`LoadSink`]
//! (`Send`, so it can finish on another thread) and the picker applies what
//! arrived in [`SessionSelectorComponent::poll`], which also expires the
//! header's timed status message. The list reports what a key did as
//! [`ListEvent`]s instead of calling back into its owner.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use hoocode_code_paths::canonicalize_path;
use hoocode_code_session::identity::{session_color_slot_for, session_slug_for};
use hoocode_code_session::{subagents_dir, SessionInfo, SubagentRun, SubagentState};
use hoocode_code_tui_keybindings::key_text;
use hoocode_code_tui_theme::{session_color_token, style_input};
use hoocode_code_tui_widgets::brand::GIT_BRANCH_GLYPH;
use hoocode_code_tui_widgets::console::{hints, query_row, Console};
use hoocode_home::shorten_path;
use hoocode_tui_components::{Input, PickerCursor};
use hoocode_tui_keys::{get_keybindings, KeybindingsManager};
use hoocode_tui_ratatui::{bold, Line, Palette, Span, Style};
use hoocode_tui_render::Component;
use hoocode_tui_util::js_regex::js_trim;
use hoocode_tui_util::{truncate_to_width, visible_width};

use crate::session_selector_search::{
    filter_and_sort_sessions, has_session_name, NameFilter, SortMode,
};

/// Which sessions the picker lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionScope {
    Current,
    All,
}

/// Branches that name no particular piece of work.
const DEFAULT_BRANCHES: [&str; 4] = ["main", "master", "trunk", "develop"];
/// How often the loop polls a session list that is still loading.
const LOAD_POLL: Duration = Duration::from_millis(40);

/// `formatSessionDate`: the age of `date` in one short unit.
pub fn format_session_date(date: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let diff_ms = (now - date).num_milliseconds() as f64;
    let mins = (diff_ms / 60_000.0).floor();
    let hours = (diff_ms / 3_600_000.0).floor();
    let days = (diff_ms / 86_400_000.0).floor();
    if mins < 1.0 {
        "now".to_string()
    } else if mins < 60.0 {
        format!("{mins}m")
    } else if hours < 24.0 {
        format!("{hours}h")
    } else if days < 7.0 {
        format!("{days}d")
    } else if days < 30.0 {
        format!("{}w", (days / 7.0).floor())
    } else if days < 365.0 {
        format!("{}mo", (days / 30.0).floor())
    } else {
        format!("{}y", (days / 365.0).floor())
    }
}

fn canonical(path: &Path) -> PathBuf {
    canonicalize_path(path)
}

// ---- header ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Info,
    Error,
}

struct SessionSelectorHeader {
    scope: SessionScope,
    sort_mode: SortMode,
    name_filter: NameFilter,
    loading: bool,
    load_progress: Option<(usize, usize)>,
    show_path: bool,
    confirming_delete: bool,
    status: Option<(StatusKind, String)>,
    status_deadline: Option<Instant>,
    /// An interrupted subagent run is highlighted: the hint offers view/resume.
    choosing_subagent: bool,
    can_resume: bool,
}

impl SessionSelectorHeader {
    fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        // Progress is scoped to the current load.
        self.load_progress = None;
    }

    fn set_status(&mut self, status: Option<(StatusKind, String)>, auto_hide: Option<Duration>) {
        self.status = status;
        self.status_deadline = match (&self.status, auto_hide) {
            (Some(_), Some(d)) if !d.is_zero() => Some(Instant::now() + d),
            _ => None,
        };
    }

    /// Expire a timed status; `true` when it went.
    fn poll(&mut self, now: Instant) -> bool {
        match self.status_deadline {
            Some(deadline) if now >= deadline => {
                self.status = None;
                self.status_deadline = None;
                true
            }
            _ => false,
        }
    }
}

impl SessionSelectorHeader {
    /// The scope tabs with the name and sort flags: the console's first
    /// top row, with the active scope in accent. Literal keys only, so the
    /// goldens hold on macOS and Linux.
    fn scope_line(&self) -> Line<'static> {
        let pal = Palette::current();
        let sort_label = match self.sort_mode {
            SortMode::Threaded => "Threaded",
            SortMode::Recent => "Recent",
            SortMode::Relevance => "Fuzzy",
        };
        let name_label = match self.name_filter {
            NameFilter::All => "All",
            NameFilter::Named => "Named",
        };
        let mut spans = if self.loading {
            let progress = match self.load_progress {
                Some((loaded, total)) => format!("{loaded}/{total}"),
                None => "...".to_string(),
            };
            vec![
                Span::styled("○ Current Folder | ".to_string(), pal.faint()),
                Span::styled(format!("Loading {progress}"), pal.acc()),
            ]
        } else if self.scope == SessionScope::Current {
            vec![
                Span::styled("◉ Current Folder".to_string(), pal.acc()),
                Span::styled(" | ○ All".to_string(), pal.faint()),
            ]
        } else {
            vec![
                Span::styled("○ Current Folder | ".to_string(), pal.faint()),
                Span::styled("◉ All".to_string(), pal.acc()),
            ]
        };
        spans.push(Span::styled("  Name: ".to_string(), pal.faint()));
        spans.push(Span::styled(name_label.to_string(), pal.acc()));
        spans.push(Span::styled("  Sort: ".to_string(), pal.faint()));
        spans.push(Span::styled(sort_label.to_string(), pal.acc()));
        Line::from(spans)
    }

    /// The console's other top rows: the delete prompt, the interrupted-run
    /// choice, the timed status, or the search legend in the steady state.
    fn top_lines(&self) -> Vec<Line<'static>> {
        let pal = Palette::current();
        if self.confirming_delete {
            return vec![Line::from(vec![Span::styled(
                "Delete session? enter confirm · esc cancel".to_string(),
                pal.err(),
            )])];
        }
        if self.choosing_subagent {
            let text = if self.can_resume {
                "Interrupted run · v view · r resume · esc back"
            } else {
                "Interrupted run · v view · esc back"
            };
            return vec![Line::from(vec![Span::styled(text.to_string(), pal.acc())])];
        }
        if let Some((kind, message)) = &self.status {
            let style = if *kind == StatusKind::Error {
                pal.err()
            } else {
                pal.acc()
            };
            return vec![Line::from(vec![Span::styled(message.clone(), style)])];
        }
        vec![
            self.scope_line(),
            Line::from(vec![
                Span::styled("tab".to_string(), pal.dim()),
                Span::styled(
                    " scope · re:<pattern> regex · \"phrase\" exact".to_string(),
                    pal.faint(),
                ),
            ]),
        ]
    }

    /// The hint foot: confirm/cancel while a delete is pending, else the
    /// console keys (the target's `logRows` foot: move, jump there, close —
    /// plus the picker's real scope, sort and delete keys).
    fn hint_line(&self) -> Line<'static> {
        if self.confirming_delete {
            hints(&[("enter", "confirm"), ("esc", "cancel")])
        } else {
            hints(&[
                ("↑↓", "move"),
                ("tab", "scope"),
                ("alt+o", "sort"),
                ("alt+x", "delete"),
                ("enter", "jump there"),
                ("esc", "close"),
            ])
        }
    }
}

// ---- list ------------------------------------------------------------------

struct TreeNode {
    session: SessionInfo,
    children: Vec<TreeNode>,
}

/// A row: a session with its place in the thread tree.
#[derive(Debug, Clone)]
struct FlatNode {
    session: SessionInfo,
    depth: usize,
    is_last: bool,
    /// For each ancestor level, whether more siblings follow it.
    ancestor_continues: Vec<bool>,
}

/// `buildSessionTree`: fork children under their parent, newest first.
fn build_session_tree(sessions: &[SessionInfo]) -> Vec<TreeNode> {
    let keys: Vec<PathBuf> = sessions.iter().map(|s| canonical(&s.path)).collect();
    let parent_of: Vec<Option<usize>> = sessions
        .iter()
        .map(|s| {
            let parent = s.parent_session_path.as_deref().filter(|p| !p.is_empty())?;
            let parent = canonical(Path::new(parent));
            // The last session with a path wins, as a Map keeps the last set.
            keys.iter().rposition(|k| *k == parent)
        })
        .collect();

    fn build(i: usize, sessions: &[SessionInfo], parent_of: &[Option<usize>]) -> TreeNode {
        let children = (0..sessions.len())
            .filter(|&j| parent_of[j] == Some(i) && j != i)
            .map(|j| build(j, sessions, parent_of))
            .collect();
        TreeNode {
            session: sessions[i].clone(),
            children,
        }
    }

    let mut roots: Vec<TreeNode> = (0..sessions.len())
        .filter(|&i| parent_of[i].is_none_or(|p| p == i))
        .map(|i| build(i, sessions, &parent_of))
        .collect();

    fn sort_nodes(nodes: &mut [TreeNode]) {
        nodes.sort_by_key(|b| std::cmp::Reverse(b.session.modified));
        for node in nodes {
            sort_nodes(&mut node.children);
        }
    }
    sort_nodes(&mut roots);
    roots
}

/// `flattenSessionTree`.
fn flatten_session_tree(roots: &[TreeNode]) -> Vec<FlatNode> {
    fn walk(
        node: &TreeNode,
        depth: usize,
        ancestor_continues: Vec<bool>,
        is_last: bool,
        out: &mut Vec<FlatNode>,
    ) {
        out.push(FlatNode {
            session: node.session.clone(),
            depth,
            is_last,
            ancestor_continues: ancestor_continues.clone(),
        });
        let n = node.children.len();
        for (i, child) in node.children.iter().enumerate() {
            // Only non-root ancestors draw a continuation line.
            let continues = depth > 0 && !is_last;
            let mut next = ancestor_continues.clone();
            next.push(continues);
            walk(child, depth + 1, next, i == n - 1, out);
        }
    }
    let mut out = Vec::new();
    let n = roots.len();
    for (i, root) in roots.iter().enumerate() {
        walk(root, 0, Vec::new(), i == n - 1, &mut out);
    }
    out
}

/// The sessions the name filter keeps. A subagent run is kept only with its
/// parent: under the named filter a run whose parent is unnamed would
/// otherwise show as a top-level session.
fn name_filtered_sessions(all: &[SessionInfo], name_filter: NameFilter) -> Vec<SessionInfo> {
    match name_filter {
        NameFilter::All => all.to_vec(),
        NameFilter::Named => {
            let kept_parents: Vec<PathBuf> = all
                .iter()
                .filter(|s| s.subagent.is_none() && has_session_name(s))
                .map(|s| canonical(&s.path))
                .collect();
            all.iter()
                .filter(|s| match &s.subagent {
                    None => has_session_name(s),
                    Some(_) => s
                        .parent_session_path
                        .as_deref()
                        .is_some_and(|p| kept_parents.contains(&canonical(Path::new(p)))),
                })
                .cloned()
                .collect()
        }
    }
}

#[cfg(test)]
mod name_filter_tests {
    use super::*;

    fn session(path: &str, name: Option<&str>, parent: Option<&str>, run: bool) -> SessionInfo {
        SessionInfo {
            path: PathBuf::from(path),
            id: path.to_string(),
            cwd: String::new(),
            name: name.map(str::to_string),
            color: None,
            branch: None,
            parent_session_path: parent.map(str::to_string),
            created: DateTime::<Utc>::UNIX_EPOCH,
            modified: DateTime::<Utc>::UNIX_EPOCH,
            message_count: 1,
            model: None,
            all_messages_text: String::new(),
            first_message: String::new(),
            subagent: run.then(|| SubagentRun {
                task_id: "run".into(),
                agent_type: "explore".into(),
                state: SubagentState::Interrupted,
            }),
        }
    }

    /// Under the named filter, a run whose parent is unnamed goes with it: it
    /// does not surface as a top-level row. A run of a named parent stays.
    #[test]
    fn a_run_is_kept_under_the_named_filter_only_with_its_named_parent() {
        let all = vec![
            session("/s/unnamed.jsonl", None, None, false),
            session(
                "/s/unnamed/subagents/a.jsonl",
                None,
                Some("/s/unnamed.jsonl"),
                true,
            ),
            session("/s/named.jsonl", Some("Work"), None, false),
            session(
                "/s/named/subagents/b.jsonl",
                Some("renamed run"),
                Some("/s/named.jsonl"),
                true,
            ),
        ];
        let named: Vec<String> = name_filtered_sessions(&all, NameFilter::Named)
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(named, ["/s/named.jsonl", "/s/named/subagents/b.jsonl"]);
        assert_eq!(name_filtered_sessions(&all, NameFilter::All).len(), 4);
    }
}

/// A subagent run's row label: its type, and where the run stands.
fn subagent_label(run: &SubagentRun) -> String {
    let state = match &run.state {
        SubagentState::Finished(status) => status.clone(),
        SubagentState::Running => "running".to_string(),
        SubagentState::Interrupted => "interrupted".to_string(),
    };
    if run.agent_type.is_empty() {
        state
    } else {
        format!("{} · {state}", run.agent_type)
    }
}

/// A subagent run, as the picker hands it to its owner to resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRunRef {
    /// The run's transcript.
    pub child: PathBuf,
    /// The session that dispatched the run.
    pub parent: PathBuf,
    /// The run's task id: the transcript's file stem.
    pub task_id: String,
    pub agent_type: String,
}

/// The run a row is, when it stopped before it settled (`interrupted`).
fn interrupted_run(session: &SessionInfo) -> Option<SubagentRunRef> {
    let run = session.subagent.as_ref()?;
    if run.state != SubagentState::Interrupted {
        return None;
    }
    Some(SubagentRunRef {
        child: session.path.clone(),
        parent: PathBuf::from(session.parent_session_path.as_ref()?),
        task_id: run.task_id.clone(),
        agent_type: run.agent_type.clone(),
    })
}

/// What a key did to the list, for its owner to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListEvent {
    Select(PathBuf),
    Cancel,
    ToggleScope,
    ToggleSort,
    ToggleNameFilter,
    TogglePath(bool),
    DeleteConfirmationChange(Option<PathBuf>),
    DeleteSession(PathBuf),
    Rename(PathBuf),
    /// The choice on an interrupted run opened (`true`) or closed.
    SubagentChoiceChange(bool),
    /// Resume an interrupted run (`r` in its choice).
    ResumeSubagent(SubagentRunRef),
    Error(String),
}

/// The ship that flew a session: the short model id from its latest
/// model change, or `—` when the session recorded none.
fn ship_name(session: &SessionInfo) -> String {
    session
        .model
        .as_deref()
        .map(|id| id.rsplit('/').next().unwrap_or(id))
        .unwrap_or("—")
        .to_string()
}

/// The search hit count for a session: matches of the current query in
/// its searchable text, or zero while unfiltered.
fn hit_count(session: &SessionInfo, query: &str) -> usize {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return 0;
    }
    let haystack = format!(
        "{} {} {} {}",
        session.id,
        session.name.as_deref().unwrap_or(""),
        session.all_messages_text,
        session.cwd
    )
    .to_lowercase();
    haystack.match_indices(&needle).count()
}

/// `SessionList`: the search line and one row per session.
pub struct SessionList {
    all_sessions: Vec<SessionInfo>,
    filtered: Vec<FlatNode>,
    cursor: PickerCursor,
    search_input: Input,
    show_cwd: bool,
    sort_mode: SortMode,
    name_filter: NameFilter,
    keybindings: Rc<KeybindingsManager>,
    show_path: bool,
    confirming_delete_path: Option<PathBuf>,
    current_session_canonical: Option<PathBuf>,
    now: Option<DateTime<Utc>>,
    /// An interrupted run whose choice (view or resume) is open.
    choosing: Option<SubagentRunRef>,
    can_resume: bool,
}

impl SessionList {
    fn new(
        sort_mode: SortMode,
        name_filter: NameFilter,
        keybindings: Rc<KeybindingsManager>,
        current_session_file: Option<&Path>,
    ) -> Self {
        let mut search_input = Input::new();
        style_input(&mut search_input);
        let mut list = Self {
            all_sessions: Vec::new(),
            filtered: Vec::new(),
            cursor: PickerCursor::new(0, 10),
            search_input,
            show_cwd: false,
            sort_mode,
            name_filter,
            keybindings,
            show_path: false,
            confirming_delete_path: None,
            current_session_canonical: current_session_file.map(canonical),
            now: None,
            choosing: None,
            can_resume: false,
        };
        list.filter_sessions("");
        list
    }

    /// The path of the highlighted session.
    pub fn selected_session_path(&self) -> Option<&Path> {
        self.filtered
            .get(self.cursor.selected())
            .map(|n| n.session.path.as_path())
    }

    /// Fix "now" for the age column (tests).
    pub fn set_now(&mut self, now: Option<DateTime<Utc>>) {
        self.now = now;
    }

    fn set_sort_mode(&mut self, sort_mode: SortMode) {
        self.sort_mode = sort_mode;
        let query = self.search_input.get_value().to_string();
        self.filter_sessions(&query);
    }

    fn set_name_filter(&mut self, name_filter: NameFilter) {
        self.name_filter = name_filter;
        let query = self.search_input.get_value().to_string();
        self.filter_sessions(&query);
    }

    fn set_sessions(&mut self, sessions: Vec<SessionInfo>, show_cwd: bool) {
        self.all_sessions = sessions;
        self.show_cwd = show_cwd;
        let query = self.search_input.get_value().to_string();
        self.filter_sessions(&query);
    }

    fn filter_sessions(&mut self, query: &str) {
        let name_filtered = name_filtered_sessions(&self.all_sessions, self.name_filter);
        if self.sort_mode == SortMode::Threaded && js_trim(query).is_empty() {
            self.filtered = flatten_session_tree(&build_session_tree(&name_filtered));
        } else {
            // A subagent run is listed under its parent in the threaded view
            // only. A search or another sort matches the parent rows alone.
            let parents: Vec<SessionInfo> = name_filtered
                .iter()
                .filter(|s| s.subagent.is_none())
                .cloned()
                .collect();
            self.filtered =
                filter_and_sort_sessions(&parents, query, self.sort_mode, NameFilter::All)
                    .into_iter()
                    .map(|session| FlatNode {
                        session,
                        depth: 0,
                        is_last: true,
                        ancestor_continues: Vec::new(),
                    })
                    .collect();
        }
        self.cursor.set_query(self.filtered.len());
    }

    fn set_confirming_delete_path(&mut self, path: Option<PathBuf>, events: &mut Vec<ListEvent>) {
        self.confirming_delete_path = path.clone();
        events.push(ListEvent::DeleteConfirmationChange(path));
    }

    fn start_delete_confirmation(&mut self, events: &mut Vec<ListEvent>) {
        let Some(selected) = self.filtered.get(self.cursor.selected()) else {
            return;
        };
        let path = selected.session.path.clone();
        if self.is_current_session_path(&path) {
            events.push(ListEvent::Error(
                "Cannot delete the currently active session".to_string(),
            ));
            return;
        }
        self.set_confirming_delete_path(Some(path), events);
    }

    fn is_current_session_path(&self, path: &Path) -> bool {
        self.current_session_canonical
            .as_ref()
            .is_some_and(|current| canonical(path) == *current)
    }

    fn build_tree_prefix(node: &FlatNode) -> String {
        if node.depth == 0 {
            return String::new();
        }
        let mut prefix: String = node
            .ancestor_continues
            .iter()
            .map(|&c| if c { "│  " } else { "   " })
            .collect();
        prefix.push_str(if node.is_last { "└─ " } else { "├─ " });
        prefix
    }

    fn search_value(&self) -> String {
        self.search_input.get_value().to_string()
    }

    fn forward_to_search(&mut self, data: &str) {
        self.search_input.handle_input(data);
        let query = self.search_value();
        self.filter_sessions(&query);
    }

    /// Handle a key; returns what it did.
    pub fn handle_key(&mut self, data: &str) -> Vec<ListEvent> {
        let kb = get_keybindings();
        let mut events = Vec::new();

        // An open choice on an interrupted run takes every key: v views the
        // transcript, r resumes the run (when resuming is possible), esc backs out.
        if let Some(run) = self.choosing.take() {
            let close = ListEvent::SubagentChoiceChange(false);
            if kb.matches(data, "tui.select.cancel") {
                events.push(close);
            } else if data == "v" {
                events.push(close);
                events.push(ListEvent::Select(run.child));
            } else if data == "r" && self.can_resume {
                events.push(close);
                events.push(ListEvent::ResumeSubagent(run));
            } else {
                self.choosing = Some(run);
            }
            return events;
        }

        // A pending delete takes every key.
        if let Some(path) = self.confirming_delete_path.clone() {
            if kb.matches(data, "tui.select.confirm") {
                self.set_confirming_delete_path(None, &mut events);
                events.push(ListEvent::DeleteSession(path));
            } else if kb.matches(data, "tui.select.cancel") {
                self.set_confirming_delete_path(None, &mut events);
            }
            return events;
        }

        if kb.matches(data, "tui.input.tab") {
            events.push(ListEvent::ToggleScope);
            return events;
        }
        if kb.matches(data, "app.session.toggleSort") {
            events.push(ListEvent::ToggleSort);
            return events;
        }
        if self
            .keybindings
            .matches(data, "app.session.toggleNamedFilter")
        {
            events.push(ListEvent::ToggleNameFilter);
            return events;
        }
        if kb.matches(data, "app.session.togglePath") {
            self.show_path = !self.show_path;
            events.push(ListEvent::TogglePath(self.show_path));
            return events;
        }
        if kb.matches(data, "app.session.delete") {
            self.start_delete_confirmation(&mut events);
            return events;
        }
        if kb.matches(data, "app.session.rename") {
            if let Some(selected) = self.filtered.get(self.cursor.selected()) {
                events.push(ListEvent::Rename(selected.session.path.clone()));
            }
            return events;
        }
        // Ctrl+Backspace deletes only with an empty query; otherwise it edits it.
        if kb.matches(data, "app.session.deleteNoninvasive") {
            if !self.search_input.get_value().is_empty() {
                self.forward_to_search(data);
                return events;
            }
            self.start_delete_confirmation(&mut events);
            return events;
        }

        if self.cursor.handle_query_key(data, &kb) {
            // Navigation: the cursor clamps at both ends.
        } else if kb.matches(data, "tui.select.confirm") {
            if let Some(selected) = self.filtered.get(self.cursor.selected()) {
                match interrupted_run(&selected.session) {
                    Some(run) => {
                        self.choosing = Some(run);
                        events.push(ListEvent::SubagentChoiceChange(true));
                    }
                    None => events.push(ListEvent::Select(selected.session.path.clone())),
                }
            }
        } else if kb.matches(data, "tui.select.cancel") {
            events.push(ListEvent::Cancel);
        } else {
            self.forward_to_search(data);
        }
        events
    }
}

impl SessionList {
    /// The MISSION LOG count: the listed sessions, or the matches of the
    /// total while searching.
    fn count_text(&self) -> String {
        let total = self.all_sessions.len();
        if self.search_value().is_empty() {
            sessions(self.filtered.len())
        } else {
            format!("{} of {}", sessions(self.filtered.len()), total)
        }
    }

    /// One session row (content only; the console adds the `›` marker or
    /// gutter and the selection band): the colour swatch, the tree prefix,
    /// the branch, the name, and the stats right-aligned, laid out like
    /// the old framed rows.
    fn console_row(
        &self,
        i: usize,
        selected: bool,
        inner: usize,
        now: DateTime<Utc>,
    ) -> Line<'static> {
        let pal = Palette::current();
        let node = &self.filtered[i];
        let session = &node.session;
        let is_confirming_delete =
            self.confirming_delete_path.as_deref() == Some(session.path.as_path());
        let is_current = self.is_current_session_path(&session.path);
        let prefix = Self::build_tree_prefix(node);
        let has_name = session.name.is_some();
        let display = session.name.as_deref().unwrap_or(&session.first_message);
        let normalized: String = display
            .chars()
            .map(|c| {
                if (c as u32) < 0x20 || c == '\x7f' {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let normalized = js_trim(&normalized);
        // A swatch in the session's own colour, as its chip resolves it.
        let slot = session
            .color
            .map(i64::from)
            .unwrap_or_else(|| i64::from(session_color_slot_for(&session.id)));
        let branch = match &session.branch {
            Some(b) if !has_name && !b.is_empty() && !DEFAULT_BRANCHES.contains(&b.as_str()) => {
                b.as_str()
            }
            _ => "",
        };
        let mut spans = vec![
            Span::styled(" ".to_string(), pal.fill(session_color_token(slot))),
            Span::raw(" ".to_string()),
            Span::styled(prefix.clone(), pal.dim()),
        ];
        if !branch.is_empty() {
            spans.push(Span::styled(format!("{GIT_BRANCH_GLYPH} "), pal.dim()));
            spans.push(Span::styled(branch.to_string(), pal.faint()));
            spans.push(Span::raw("  ".to_string()));
        }
        let age = format_session_date(session.modified, now);
        // The mission table's right columns: the ship that flew the
        // session, its step count, the search hit count, and when. Hits
        // read red; unnamed rows keep the id slug so they stay
        // distinguishable.
        let body = if is_confirming_delete {
            pal.err()
        } else {
            pal.dim()
        };
        let hits = hit_count(session, &self.search_value());
        let mut right: Vec<(String, Style)> = match &session.subagent {
            Some(run) => vec![(format!("{} {age}", subagent_label(run)), body)],
            None => {
                let mut columns = Vec::new();
                if !has_name {
                    columns.push((session_slug_for(&session.id), body));
                }
                columns.push((
                    ship_name(session),
                    if session.model.is_some() && !is_confirming_delete {
                        Style::default()
                    } else {
                        body
                    },
                ));
                columns.push((session.message_count.to_string(), body));
                columns.push((hits.to_string(), if hits > 0 { pal.err() } else { body }));
                columns.push((age, body));
                columns
            }
        };
        if self.show_cwd && !session.cwd.is_empty() {
            right.insert(0, (shorten_path(&session.cwd).to_string(), body));
        }
        if self.show_path {
            right.insert(
                0,
                (
                    shorten_path(&session.path.to_string_lossy()).to_string(),
                    body,
                ),
            );
        }
        let prefix_width = visible_width(&prefix) as i64;
        let branch_width = if branch.is_empty() {
            0
        } else {
            visible_width(&format!("{GIT_BRANCH_GLYPH} {branch}  ")) as i64
        };
        let right_width = right
            .iter()
            .map(|(text, _)| visible_width(text) as i64)
            .sum::<i64>()
            + right.len().saturating_sub(1) as i64;
        let available = inner as i64 - 2 - prefix_width - branch_width - right_width - 2;
        let truncated = truncate_to_width(normalized, available.max(10) as usize, "…", false);
        let name_width = visible_width(&truncated) as i64;
        let mut name_span = if is_confirming_delete {
            Span::styled(truncated, pal.err())
        } else if is_current {
            Span::styled(truncated, pal.acc())
        } else if has_name {
            Span::styled(truncated, pal.warn())
        } else {
            Span::raw(truncated)
        };
        if selected {
            name_span.style = bold(name_span.style);
        }
        spans.push(name_span);
        let spacing =
            (inner as i64 - 2 - prefix_width - branch_width - name_width - right_width).max(1);
        spans.push(Span::raw(" ".repeat(spacing as usize)));
        for (n, (text, style)) in right.into_iter().enumerate() {
            if n > 0 {
                spans.push(Span::raw(" ".to_string()));
            }
            spans.push(Span::styled(text, style));
        }
        Line::from(spans)
    }

    /// The cursor's window as console rows, with the selection remapped
    /// into it, plus the rows below the list (the empty-list message or
    /// the scroll position).
    fn console_rows(
        &self,
        inner: usize,
    ) -> (Vec<Line<'static>>, Option<usize>, Vec<Line<'static>>) {
        let pal = Palette::current();
        if self.filtered.is_empty() {
            let message = if self.name_filter == NameFilter::Named {
                let toggle = key_text("app.session.toggleNamedFilter");
                if self.show_cwd {
                    format!("  No named sessions found. Press {toggle} to show all.")
                } else {
                    format!(
                        "  No named sessions in current folder. Press {toggle} to show all, or tab to view all."
                    )
                }
            } else if self.show_cwd {
                "  No sessions found".to_string()
            } else {
                "  No sessions in current folder. Press tab to view all.".to_string()
            };
            return (
                Vec::new(),
                None,
                vec![Line::from(vec![Span::styled(message, pal.faint())])],
            );
        }
        let window = self.cursor.window();
        let selected = self.cursor.selected();
        let now = self.now.unwrap_or_else(Utc::now);
        let rows = window
            .clone()
            .map(|i| self.console_row(i, i == selected, inner, now))
            .collect();
        let mut bottom = Vec::new();
        if let Some(footer) = self.cursor.footer() {
            bottom.push(Line::from(vec![Span::styled(
                format!("  {footer}"),
                pal.faint(),
            )]));
        }
        (rows, window.clone().position(|i| i == selected), bottom)
    }

    fn set_focused(&mut self, focused: bool) {
        self.search_input.set_focused(focused);
    }
}

/// `1 session`, `N sessions`.
fn sessions(count: usize) -> String {
    if count == 1 {
        "1 session".to_string()
    } else {
        format!("{count} sessions")
    }
}

// ---- deleting --------------------------------------------------------------

/// A deleted session takes its subagent transcripts with it: `<session>/subagents/`.
/// They are trashed like the session file when a `trash` command exists, else
/// removed. A subagent run's own row has no such dir, so this is a no-op there.
pub fn delete_subagent_transcripts(session: &Path) {
    let dir = subagents_dir(session);
    if !dir.is_dir() {
        return;
    }
    let trashed = std::process::Command::new("trash")
        .arg(&dir)
        .stdin(std::process::Stdio::null())
        .output()
        .is_ok_and(|out| out.status.success());
    if !trashed {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

enum DeleteMethod {
    Trash,
    Unlink,
}

/// `deleteSessionFile`: the `trash` CLI first, then a plain unlink.
fn delete_session_file(path: &Path) -> Result<DeleteMethod, String> {
    let lossy = path.to_string_lossy();
    let mut cmd = std::process::Command::new("trash");
    if lossy.starts_with('-') {
        cmd.arg("--");
    }
    let trash = cmd.arg(path).stdin(std::process::Stdio::null()).output();
    let trash_ok = matches!(&trash, Ok(out) if out.status.success());
    if trash_ok || !path.exists() {
        return Ok(DeleteMethod::Trash);
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(DeleteMethod::Unlink),
        Err(err) => {
            let mut parts = Vec::new();
            match &trash {
                Err(e) => parts.push(e.to_string()),
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stderr = stderr.trim();
                    if !stderr.is_empty() {
                        parts.push(stderr.lines().next().unwrap_or(stderr).to_string());
                    }
                }
            }
            let hint = (!parts.is_empty()).then(|| {
                let joined = parts.join(" · ");
                format!("trash: {}", joined.chars().take(200).collect::<String>())
            });
            Err(match hint {
                Some(h) => format!("{err} ({h})"),
                None => err.to_string(),
            })
        }
    }
}

// ---- loading ---------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadReason {
    Initial,
    Refresh,
    Toggle,
}

enum LoadMsg {
    Progress {
        scope: SessionScope,
        seq: Option<u64>,
        loaded: usize,
        total: usize,
    },
    Done {
        scope: SessionScope,
        seq: Option<u64>,
        reason: LoadReason,
        result: Result<Vec<SessionInfo>, String>,
    },
}

/// Where a loader reports: progress, then one result. It may finish on
/// another thread; the picker applies it in [`SessionSelectorComponent::poll`].
///
/// A load settles once: by [`LoadSink::finish`], or, when the last clone goes
/// with no result sent, with `Err("loader stopped")`. So a loader that stops
/// (a panic, a dropped task) cannot leave the picker loading forever.
#[derive(Clone)]
pub struct LoadSink {
    inner: Arc<LoadSinkInner>,
}

/// The state the clones share. It drops once, with the last clone, so the
/// "loader stopped" result is sent exactly once and only then.
struct LoadSinkInner {
    tx: Sender<LoadMsg>,
    scope: SessionScope,
    seq: Option<u64>,
    reason: LoadReason,
    /// Set once the load's result has been sent.
    settled: AtomicBool,
}

impl LoadSink {
    fn new(tx: Sender<LoadMsg>, scope: SessionScope, seq: Option<u64>, reason: LoadReason) -> Self {
        Self {
            inner: Arc::new(LoadSinkInner {
                tx,
                scope,
                seq,
                reason,
                settled: AtomicBool::new(false),
            }),
        }
    }

    /// `onProgress(loaded, total)`.
    pub fn progress(&self, loaded: usize, total: usize) {
        let _ = self.inner.tx.send(LoadMsg::Progress {
            scope: self.inner.scope,
            seq: self.inner.seq,
            loaded,
            total,
        });
    }

    /// The loaded sessions, or why loading failed. Only the first call sends.
    pub fn finish(&self, result: Result<Vec<SessionInfo>, String>) {
        if self.inner.settled.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.inner.tx.send(LoadMsg::Done {
            scope: self.inner.scope,
            seq: self.inner.seq,
            reason: self.inner.reason,
            result,
        });
    }
}

impl Drop for LoadSinkInner {
    /// Runs once, when the last clone goes: settles the load if nothing did.
    fn drop(&mut self) {
        if !self.settled.swap(true, Ordering::SeqCst) {
            let _ = self.tx.send(LoadMsg::Done {
                scope: self.scope,
                seq: self.seq,
                reason: self.reason,
                result: Err("loader stopped".to_string()),
            });
        }
    }
}

/// Starts loading one scope's sessions.
pub type SessionsLoader = Box<dyn FnMut(LoadSink)>;

/// Renames a session; the picker reloads afterwards.
pub type RenameSessionFn = Box<dyn FnMut(&Path, &str) -> Result<(), String>>;

/// Resumes an interrupted subagent run (see [`SubagentRunRef`]).
pub type ResumeSubagentFn = Box<dyn FnMut(SubagentRunRef)>;

#[derive(Default)]
pub struct SessionSelectorOptions {
    pub rename_session: Option<RenameSessionFn>,
    /// Defaults to whether renaming is possible.
    pub show_rename_hint: Option<bool>,
    /// The manager the named-filter key is read from (the app's by default).
    pub keybindings: Option<Rc<KeybindingsManager>>,
    /// Resumes an interrupted subagent run. Without it the choice offers view only.
    pub resume_subagent: Option<ResumeSubagentFn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    List,
    Rename,
}

/// `SessionSelectorComponent`.
pub struct SessionSelectorComponent {
    header: Rc<RefCell<SessionSelectorHeader>>,
    session_list: Rc<RefCell<SessionList>>,
    scope: SessionScope,
    sort_mode: SortMode,
    name_filter: NameFilter,
    current_sessions: Option<Vec<SessionInfo>>,
    all_sessions: Option<Vec<SessionInfo>>,
    current_loader: SessionsLoader,
    all_loader: SessionsLoader,
    on_select: Box<dyn FnMut(PathBuf)>,
    on_cancel: Box<dyn FnMut()>,
    rename_session: Option<RenameSessionFn>,
    resume_subagent: Option<ResumeSubagentFn>,
    current_loading: bool,
    all_loading: bool,
    all_load_seq: u64,
    mode: Mode,
    rename_input: Rc<RefCell<Input>>,
    rename_submitted: Rc<RefCell<Option<String>>>,
    rename_target: Option<PathBuf>,
    tx: Sender<LoadMsg>,
    rx: Receiver<LoadMsg>,
    focused: bool,
}

impl SessionSelectorComponent {
    pub fn new(
        current_loader: SessionsLoader,
        all_loader: SessionsLoader,
        on_select: Box<dyn FnMut(PathBuf)>,
        on_cancel: Box<dyn FnMut()>,
        options: SessionSelectorOptions,
        current_session_file: Option<&Path>,
    ) -> Self {
        let keybindings = options.keybindings.unwrap_or_else(|| {
            Rc::new(
                hoocode_code_tui_keybindings::AppKeybindingsManager::create(None).into_manager(),
            )
        });
        let can_resume = options.resume_subagent.is_some();
        let header = Rc::new(RefCell::new(SessionSelectorHeader {
            scope: SessionScope::Current,
            sort_mode: SortMode::Threaded,
            name_filter: NameFilter::All,
            loading: false,
            load_progress: None,
            show_path: false,
            confirming_delete: false,
            status: None,
            status_deadline: None,
            choosing_subagent: false,
            can_resume,
        }));
        let session_list = Rc::new(RefCell::new(SessionList::new(
            SortMode::Threaded,
            NameFilter::All,
            keybindings,
            current_session_file,
        )));

        let mut rename_input = Input::new();
        style_input(&mut rename_input);
        let rename_submitted = Rc::new(RefCell::new(None));
        let sink = rename_submitted.clone();
        rename_input.on_submit = Some(Box::new(move |value: &str| {
            *sink.borrow_mut() = Some(value.to_string());
        }));

        session_list.borrow_mut().can_resume = can_resume;
        let (tx, rx) = channel();
        let mut selector = Self {
            header,
            session_list,
            scope: SessionScope::Current,
            sort_mode: SortMode::Threaded,
            name_filter: NameFilter::All,
            current_sessions: None,
            all_sessions: None,
            current_loader,
            all_loader,
            on_select,
            on_cancel,
            rename_session: options.rename_session,
            resume_subagent: options.resume_subagent,
            current_loading: false,
            all_loading: false,
            all_load_seq: 0,
            mode: Mode::List,
            rename_input: Rc::new(RefCell::new(rename_input)),
            rename_submitted,
            rename_target: None,
            tx,
            rx,
            focused: false,
        };
        selector.load_scope(SessionScope::Current, LoadReason::Initial);
        selector
    }

    /// The list (tests drive it directly, as the pin's do).
    pub fn session_list(&self) -> Rc<RefCell<SessionList>> {
        self.session_list.clone()
    }

    /// When `poll` next has work: a short step while a load runs (its results
    /// arrive on a channel that does not wake the loop), else the status expiry.
    pub fn deadline(&self) -> Option<Instant> {
        if self.current_loading || self.all_loading {
            return Some(Instant::now() + LOAD_POLL);
        }
        self.header.borrow().status_deadline
    }

    /// Apply loads that finished and expire the timed status. `true` when
    /// something changed (render again).
    pub fn poll(&mut self) -> bool {
        let mut changed = self.header.borrow_mut().poll(Instant::now());
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            self.apply(msg);
        }
        changed
    }

    fn apply(&mut self, msg: LoadMsg) {
        match msg {
            LoadMsg::Progress {
                scope,
                seq,
                loaded,
                total,
            } => {
                if scope != self.scope || seq.is_some_and(|s| s != self.all_load_seq) {
                    return;
                }
                self.header.borrow_mut().load_progress = Some((loaded, total));
            }
            LoadMsg::Done {
                scope,
                seq,
                reason,
                result,
            } => self.finish_load(scope, seq, reason, result),
        }
    }

    fn finish_load(
        &mut self,
        scope: SessionScope,
        seq: Option<u64>,
        reason: LoadReason,
        result: Result<Vec<SessionInfo>, String>,
    ) {
        let show_cwd = scope == SessionScope::All;
        match scope {
            SessionScope::Current => self.current_loading = false,
            SessionScope::All => self.all_loading = false,
        }
        match result {
            Ok(sessions) => {
                match scope {
                    SessionScope::Current => self.current_sessions = Some(sessions.clone()),
                    SessionScope::All => self.all_sessions = Some(sessions.clone()),
                }
                if scope != self.scope || seq.is_some_and(|s| s != self.all_load_seq) {
                    return;
                }
                self.header.borrow_mut().set_loading(false);
                let empty = sessions.is_empty();
                self.session_list
                    .borrow_mut()
                    .set_sessions(sessions, show_cwd);
                if scope == SessionScope::All
                    && empty
                    && self.current_sessions.as_ref().is_none_or(Vec::is_empty)
                {
                    (self.on_cancel)();
                }
            }
            Err(message) => {
                if scope != self.scope || seq.is_some_and(|s| s != self.all_load_seq) {
                    return;
                }
                let mut header = self.header.borrow_mut();
                header.set_loading(false);
                header.set_status(
                    Some((
                        StatusKind::Error,
                        format!("Failed to load sessions: {message}"),
                    )),
                    Some(Duration::from_millis(4000)),
                );
                drop(header);
                if reason == LoadReason::Initial {
                    self.session_list
                        .borrow_mut()
                        .set_sessions(Vec::new(), show_cwd);
                }
            }
        }
    }

    fn load_scope(&mut self, scope: SessionScope, reason: LoadReason) {
        match scope {
            SessionScope::Current => self.current_loading = true,
            SessionScope::All => self.all_loading = true,
        }
        let seq = (scope == SessionScope::All).then(|| {
            self.all_load_seq += 1;
            self.all_load_seq
        });
        {
            let mut header = self.header.borrow_mut();
            header.scope = scope;
            header.set_loading(true);
        }
        let sink = LoadSink::new(self.tx.clone(), scope, seq, reason);
        match scope {
            SessionScope::Current => (self.current_loader)(sink),
            SessionScope::All => (self.all_loader)(sink),
        }
    }

    fn visible_sessions(&self) -> Vec<SessionInfo> {
        let sessions = match self.scope {
            SessionScope::All => &self.all_sessions,
            SessionScope::Current => &self.current_sessions,
        };
        sessions.clone().unwrap_or_default()
    }

    fn enter_rename_mode(&mut self, path: PathBuf, current_name: Option<String>) {
        self.mode = Mode::Rename;
        self.rename_target = Some(path);
        {
            let mut input = self.rename_input.borrow_mut();
            input.set_value(current_name.unwrap_or_default());
            input.set_focused(true);
        }
    }

    fn exit_rename_mode(&mut self) {
        self.mode = Mode::List;
        self.rename_target = None;
    }

    fn confirm_rename(&mut self, value: &str) {
        let next = js_trim(value);
        if next.is_empty() {
            return;
        }
        let Some(target) = self.rename_target.clone() else {
            self.exit_rename_mode();
            return;
        };
        let Some(rename) = self.rename_session.as_mut() else {
            self.exit_rename_mode();
            return;
        };
        if rename(&target, next).is_ok() {
            self.load_scope(self.scope, LoadReason::Refresh);
        }
        self.exit_rename_mode();
    }

    fn toggle_sort_mode(&mut self) {
        self.sort_mode = match self.sort_mode {
            SortMode::Threaded => SortMode::Recent,
            SortMode::Recent => SortMode::Relevance,
            SortMode::Relevance => SortMode::Threaded,
        };
        self.header.borrow_mut().sort_mode = self.sort_mode;
        self.session_list.borrow_mut().set_sort_mode(self.sort_mode);
    }

    fn toggle_name_filter(&mut self) {
        self.name_filter = match self.name_filter {
            NameFilter::All => NameFilter::Named,
            NameFilter::Named => NameFilter::All,
        };
        self.header.borrow_mut().name_filter = self.name_filter;
        self.session_list
            .borrow_mut()
            .set_name_filter(self.name_filter);
    }

    fn toggle_scope(&mut self) {
        if self.scope == SessionScope::Current {
            self.scope = SessionScope::All;
            self.header.borrow_mut().scope = self.scope;
            if let Some(all) = self.all_sessions.clone() {
                self.header.borrow_mut().set_loading(false);
                self.session_list.borrow_mut().set_sessions(all, true);
                return;
            }
            if !self.all_loading {
                self.load_scope(SessionScope::All, LoadReason::Toggle);
            }
            return;
        }
        self.scope = SessionScope::Current;
        {
            let mut header = self.header.borrow_mut();
            header.scope = self.scope;
            header.set_loading(self.current_loading);
        }
        let current = self.current_sessions.clone().unwrap_or_default();
        self.session_list.borrow_mut().set_sessions(current, false);
    }

    fn delete_session(&mut self, path: PathBuf) {
        match delete_session_file(&path) {
            Ok(method) => {
                delete_subagent_transcripts(&path);
                if let Some(s) = &mut self.current_sessions {
                    s.retain(|x| x.path != path);
                }
                if let Some(s) = &mut self.all_sessions {
                    s.retain(|x| x.path != path);
                }
                let sessions = self.visible_sessions();
                let show_cwd = self.scope == SessionScope::All;
                self.session_list
                    .borrow_mut()
                    .set_sessions(sessions, show_cwd);
                let message = match method {
                    DeleteMethod::Trash => "Session moved to trash",
                    DeleteMethod::Unlink => "Session deleted",
                };
                self.header.borrow_mut().set_status(
                    Some((StatusKind::Info, message.to_string())),
                    Some(Duration::from_millis(2000)),
                );
                self.load_scope(self.scope, LoadReason::Refresh);
            }
            Err(error) => {
                self.header.borrow_mut().set_status(
                    Some((StatusKind::Error, format!("Failed to delete: {error}"))),
                    Some(Duration::from_millis(3000)),
                );
            }
        }
    }

    /// Rename as a console: the title on top, the name in the query row
    /// with a caret, and save/cancel in the foot.
    fn render_rename(&self, width: usize) -> Vec<String> {
        let pal = Palette::current();
        let mut console = Console::new("MISSION LOG · /resume", String::new());
        console.top = vec![Line::from(vec![Span::styled(
            "Rename Session".to_string(),
            bold(pal.acc()),
        )])];
        let value = self.rename_input.borrow().get_value().to_string();
        console.query = Some(query_row(&value, self.focused));
        console.hint = hints(&[("enter", "save"), ("esc", "cancel")]);
        console.render(width)
    }

    /// Act on what the list reported.
    pub fn apply_list_events(&mut self, events: Vec<ListEvent>) {
        for event in events {
            match event {
                ListEvent::Select(path) => {
                    self.header.borrow_mut().set_status(None, None);
                    (self.on_select)(path);
                }
                ListEvent::Cancel => {
                    self.header.borrow_mut().set_status(None, None);
                    (self.on_cancel)();
                }
                ListEvent::ToggleScope => self.toggle_scope(),
                ListEvent::ToggleSort => self.toggle_sort_mode(),
                ListEvent::ToggleNameFilter => self.toggle_name_filter(),
                ListEvent::TogglePath(show) => self.header.borrow_mut().show_path = show,
                ListEvent::DeleteConfirmationChange(path) => {
                    self.header.borrow_mut().confirming_delete = path.is_some();
                }
                ListEvent::SubagentChoiceChange(open) => {
                    self.header.borrow_mut().choosing_subagent = open;
                }
                ListEvent::ResumeSubagent(run) => {
                    self.header.borrow_mut().set_status(None, None);
                    if let Some(resume) = self.resume_subagent.as_mut() {
                        resume(run);
                    }
                }
                ListEvent::DeleteSession(path) => self.delete_session(path),
                ListEvent::Rename(path) => {
                    if self.rename_session.is_none() {
                        continue;
                    }
                    let loading = match self.scope {
                        SessionScope::Current => self.current_loading,
                        SessionScope::All => self.all_loading,
                    };
                    if loading {
                        continue;
                    }
                    let name = self
                        .visible_sessions()
                        .into_iter()
                        .find(|s| s.path == path)
                        .and_then(|s| s.name);
                    self.enter_rename_mode(path, name);
                }
                ListEvent::Error(message) => {
                    self.header.borrow_mut().set_status(
                        Some((StatusKind::Error, message)),
                        Some(Duration::from_millis(3000)),
                    );
                }
            }
        }
    }
}

impl Component for SessionSelectorComponent {
    /// The MISSION LOG console: scope tabs on top, the `▸` query row, one
    /// row per session with `›` on the selection band, and the hint foot.
    /// Rename draws as a console too. Keys, filtering and events unchanged.
    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        if self.mode == Mode::Rename {
            return self.render_rename(width);
        }
        let header = self.header.borrow();
        let list = self.session_list.borrow();
        let mut console = Console::new("MISSION LOG · /resume", list.count_text());
        console.top = header.top_lines();
        console.query = Some(query_row(&list.search_value(), self.focused));
        let inner = width.saturating_sub(2);
        let (rows, selected, bottom) = list.console_rows(inner);
        console.rows = rows;
        console.selected = selected;
        console.bottom = bottom;
        console.hint = header.hint_line();
        console.render(width)
    }

    fn handle_input(&mut self, data: &str) {
        if self.mode == Mode::Rename {
            if get_keybindings().matches(data, "tui.select.cancel") {
                self.exit_rename_mode();
                return;
            }
            self.rename_input.borrow_mut().handle_input(data);
            let submitted = self.rename_submitted.borrow_mut().take();
            if let Some(value) = submitted {
                self.confirm_rename(&value);
            }
            return;
        }
        let events = self.session_list.borrow_mut().handle_key(data);
        self.apply_list_events(events);
    }

    fn invalidate(&mut self) {
        self.session_list.borrow_mut().search_input.invalidate();
        self.rename_input.borrow_mut().invalidate();
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.session_list.borrow_mut().set_focused(focused);
        self.rename_input.borrow_mut().set_focused(focused);
    }
}

#[cfg(test)]
mod load_sink_tests {
    use super::*;

    fn sink() -> (LoadSink, Receiver<LoadMsg>) {
        let (tx, rx) = channel();
        let sink = LoadSink::new(tx, SessionScope::Current, None, LoadReason::Initial);
        (sink, rx)
    }

    /// The results of every `Done` message sent so far.
    fn done_results(rx: &Receiver<LoadMsg>) -> Vec<Result<Vec<SessionInfo>, String>> {
        rx.try_iter()
            .filter_map(|msg| match msg {
                LoadMsg::Done { result, .. } => Some(result),
                LoadMsg::Progress { .. } => None,
            })
            .collect()
    }

    #[test]
    fn a_dropped_sink_settles_its_load_with_an_error() {
        let (sink, rx) = sink();
        drop(sink);
        let results = done_results(&rx);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].as_ref().err().map(String::as_str),
            Some("loader stopped")
        );
    }

    #[test]
    fn finishing_sends_one_result_and_the_drop_adds_none() {
        let (sink, rx) = sink();
        sink.finish(Ok(Vec::new()));
        sink.finish(Err("second".to_string()));
        drop(sink);
        let results = done_results(&rx);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok());
    }

    #[test]
    fn a_clone_dropping_does_not_settle_the_load() {
        let (sink, rx) = sink();
        let other = sink.clone();
        drop(sink);
        assert!(done_results(&rx).is_empty());
        other.finish(Ok(Vec::new()));
        drop(other);
        assert_eq!(done_results(&rx).len(), 1);
    }
}
