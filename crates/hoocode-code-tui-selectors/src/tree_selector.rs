//! The session tree (`/tree`), hoocode `components/tree-selector.ts`.
//!
//! Entries are read in their JSON wire form (what the session file holds,
//! and what the original's `entry.type` / `entry.message.role` checks see),
//! so the filter, search and display rules stay line for line with the TS.
//!
//! The FLIGHT PATH console (M7, ratatui): `FLIGHT PATH · /tree  N nodes`,
//! the `▸` query row, one row per entry with `›` on the selection band,
//! the position footer, and the hint foot. Shared `Console` chrome in
//! `code-tui-widgets/src/console.rs`; keys, filtering and events unchanged.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Local, Timelike};
use hoocode_code_session::SessionTreeNode;
use hoocode_code_settings::TreeFilterMode;
use hoocode_code_tui_theme::style_input;
use hoocode_code_tui_widgets::console::{hints, query_row, Console};
use hoocode_tui_components::{Input, PickerCursor};
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{bold, italic, Line, Palette, Span};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;
use serde_json::Value;

/// One session entry, flattened out of the tree.
struct Node {
    entry: Value,
    id: String,
    parent_id: Option<String>,
    label: Option<String>,
    label_timestamp: Option<String>,
    children: Vec<usize>,
}

/// `GutterInfo`: a `│` (or its gap) at an ancestor's connector column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gutter {
    position: usize,
    show: bool,
}

/// `FlatNode`: a node with its place in the drawn tree.
#[derive(Debug, Clone)]
struct FlatNode {
    node: usize,
    indent: usize,
    show_connector: bool,
    is_last: bool,
    gutters: Vec<Gutter>,
    is_virtual_root_child: bool,
}

/// What the tree asks its owner to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeEvent {
    /// Navigate to this entry.
    Select(String),
    Cancel,
    /// A label was set (`Some`) or removed (`None`) on an entry.
    LabelChange(String, Option<String>),
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn entry_type(entry: &Value) -> &str {
    str_field(entry, "type").unwrap_or("")
}

fn message_role(entry: &Value) -> Option<&str> {
    (entry_type(entry) == "message")
        .then(|| entry.get("message").and_then(|m| str_field(m, "role")))
        .flatten()
}

/// JavaScript `String.prototype.slice(0, n)` on UTF-16 units.
fn js_slice(s: &str, n: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in s.chars() {
        units += c.len_utf16();
        if units > n {
            break;
        }
        out.push(c);
    }
    out
}

fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `extractContent`: a string, or the text blocks, capped at 200.
fn extract_content(content: Option<&Value>) -> String {
    const MAX: usize = 200;
    match content {
        Some(Value::String(s)) => js_slice(s, MAX),
        Some(Value::Array(blocks)) => {
            let mut result = String::new();
            for block in blocks {
                if str_field(block, "type") == Some("text") {
                    result.push_str(str_field(block, "text").unwrap_or(""));
                    if js_len(&result) >= MAX {
                        return js_slice(&result, MAX);
                    }
                }
            }
            result
        }
        _ => String::new(),
    }
}

/// `hasTextContent`.
fn has_text_content(content: Option<&Value>) -> bool {
    match content {
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(blocks)) => blocks.iter().any(|b| {
            str_field(b, "type") == Some("text")
                && str_field(b, "text").is_some_and(|t| !t.trim().is_empty())
        }),
        _ => false,
    }
}

/// `s.replace(/[\n\t]/g, " ").trim()`.
fn normalize(s: &str) -> String {
    s.replace(['\n', '\t'], " ").trim().to_string()
}

/// `formatToolCall`: a one-line summary of a tool call.
fn format_tool_call(name: &str, args: &Value) -> String {
    let shorten = |p: String| -> String { hoocode_home::shorten_path(&p) };
    // `String(args.x || "")`: a missing or empty value is "".
    let arg = |key: &str| -> String {
        match args.get(key) {
            None | Some(Value::Null) | Some(Value::Bool(false)) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) if n.as_f64() == Some(0.0) => String::new(),
            Some(other) => other.to_string(),
        }
    };
    let path = || {
        let p = arg("path");
        shorten(if p.is_empty() { arg("file_path") } else { p })
    };
    match name {
        "Read" => {
            let mut display = path();
            let offset = args.get("offset").and_then(Value::as_i64);
            let limit = args.get("limit").and_then(Value::as_i64);
            if offset.is_some() || limit.is_some() {
                let start = offset.unwrap_or(1);
                display.push_str(&format!(":{start}"));
                if let Some(limit) = limit {
                    let end = start + limit - 1;
                    if end != 0 {
                        display.push_str(&format!("-{end}"));
                    }
                }
            }
            format!("[read: {display}]")
        }
        "Write" => format!("[write: {}]", path()),
        "Edit" => format!("[edit: {}]", path()),
        "Shell" => {
            let raw = arg("command");
            let cmd = js_slice(normalize(&raw).as_str(), 50);
            format!(
                "[bash: {cmd}{}]",
                if js_len(&raw) > 50 { "..." } else { "" }
            )
        }
        "CodeSearch" => {
            let glob = arg("glob");
            let glob = if glob.is_empty() {
                String::new()
            } else {
                format!(" in {}", shorten(glob))
            };
            format!("[CodeSearch: {}{glob}]", arg("query"))
        }
        _ => {
            let json = args.to_string();
            format!(
                "[{name}: {}{}]",
                js_slice(&json, 40),
                if js_len(&json) > 40 { "..." } else { "" }
            )
        }
    }
}

/// `formatLabelTimestamp`: the time today, the date this year, else with the year.
fn format_label_timestamp(timestamp: &str) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(timestamp) else {
        return String::new();
    };
    let date = parsed.with_timezone(&Local);
    let now = Local::now();
    let time = format!("{:02}:{:02}", date.hour(), date.minute());
    if date.date_naive() == now.date_naive() {
        return time;
    }
    if date.year() == now.year() {
        return format!("{}/{} {time}", date.month(), date.day());
    }
    let year = date.year().to_string();
    let short = hoocode_tui_util::text_slice::suffix_from(&year, year.len().saturating_sub(2));
    format!("{short}/{}/{} {time}", date.month(), date.day())
}

const FILTER_MODES: [TreeFilterMode; 5] = [
    TreeFilterMode::Default,
    TreeFilterMode::NoTools,
    TreeFilterMode::UserOnly,
    TreeFilterMode::LabeledOnly,
    TreeFilterMode::All,
];

/// `TreeList`: the flattened tree with selection, filters, search and folds.
pub struct TreeList {
    nodes: Vec<Node>,
    index_by_id: HashMap<String, usize>,
    flat_nodes: Vec<FlatNode>,
    filtered_nodes: Vec<FlatNode>,
    cursor: PickerCursor,
    current_leaf_id: Option<String>,
    filter_mode: TreeFilterMode,
    search_query: String,
    tool_calls: HashMap<String, (String, Value)>,
    multiple_roots: bool,
    show_label_timestamps: bool,
    active_path_ids: HashSet<String>,
    visible_parent: HashMap<String, Option<String>>,
    visible_children: HashMap<Option<String>, Vec<String>>,
    last_selected_id: Option<String>,
    folded: HashSet<String>,
    events: Vec<TreeEvent>,
    /// The entry whose label the owner should open an editor for.
    label_edit: Option<(String, Option<String>)>,
}

impl TreeList {
    fn new(
        tree: &[SessionTreeNode],
        current_leaf_id: Option<&str>,
        max_visible_lines: usize,
        initial_selected_id: Option<&str>,
        initial_filter_mode: Option<TreeFilterMode>,
    ) -> Self {
        let mut nodes = Vec::new();
        let roots: Vec<usize> = tree.iter().map(|n| Self::add_node(&mut nodes, n)).collect();
        let index_by_id = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        let mut list = Self {
            nodes,
            index_by_id,
            flat_nodes: Vec::new(),
            filtered_nodes: Vec::new(),
            cursor: PickerCursor::new(0, max_visible_lines),
            current_leaf_id: current_leaf_id.map(String::from),
            filter_mode: initial_filter_mode.unwrap_or(TreeFilterMode::Default),
            search_query: String::new(),
            tool_calls: HashMap::new(),
            multiple_roots: roots.len() > 1,
            show_label_timestamps: false,
            active_path_ids: HashSet::new(),
            visible_parent: HashMap::new(),
            visible_children: HashMap::new(),
            last_selected_id: None,
            folded: HashSet::new(),
            events: Vec::new(),
            label_edit: None,
        };
        list.flat_nodes = list.flatten_tree(&roots);
        list.build_active_path();
        list.apply_filter();
        let target = initial_selected_id
            .map(String::from)
            .or_else(|| list.current_leaf_id.clone());
        let nearest = list.find_nearest_visible_index(target.as_deref());
        list.cursor.select(nearest);
        list.last_selected_id = list.selected_id();
        list
    }

    fn add_node(nodes: &mut Vec<Node>, tree: &SessionTreeNode) -> usize {
        let entry = serde_json::to_value(&tree.entry).unwrap_or(Value::Null);
        let index = nodes.len();
        nodes.push(Node {
            id: str_field(&entry, "id").unwrap_or_default().to_string(),
            parent_id: str_field(&entry, "parentId").map(String::from),
            entry,
            label: tree.label.clone(),
            label_timestamp: tree.label_timestamp.clone(),
            children: Vec::new(),
        });
        let children: Vec<usize> = tree
            .children
            .iter()
            .map(|c| Self::add_node(nodes, c))
            .collect();
        nodes[index].children = children;
        index
    }

    fn parent_of(&self, id: &str) -> Option<String> {
        self.index_by_id
            .get(id)
            .and_then(|&i| self.nodes[i].parent_id.clone())
    }

    fn selected_id(&self) -> Option<String> {
        self.filtered_nodes
            .get(self.cursor.selected())
            .map(|f| self.nodes[f.node].id.clone())
    }

    /// `getSelectedNode`: the selected entry's id.
    pub fn selected_entry_id(&self) -> Option<String> {
        self.selected_id()
    }

    /// `getSearchQuery`.
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// `findNearestVisibleIndex`: the entry, else its nearest visible
    /// ancestor, else the last row.
    fn find_nearest_visible_index(&self, entry_id: Option<&str>) -> usize {
        if self.filtered_nodes.is_empty() {
            return 0;
        }
        let visible: HashMap<&str, usize> = self
            .filtered_nodes
            .iter()
            .enumerate()
            .map(|(i, f)| (self.nodes[f.node].id.as_str(), i))
            .collect();
        let mut current = entry_id.map(String::from);
        while let Some(id) = current {
            if let Some(&index) = visible.get(id.as_str()) {
                return index;
            }
            if !self.index_by_id.contains_key(&id) {
                break;
            }
            current = self.parent_of(&id);
        }
        self.filtered_nodes.len() - 1
    }

    /// `buildActivePath`: the ids from the current leaf up to its root.
    fn build_active_path(&mut self) {
        self.active_path_ids.clear();
        let mut current = self.current_leaf_id.clone();
        while let Some(id) = current {
            self.active_path_ids.insert(id.clone());
            if !self.index_by_id.contains_key(&id) {
                break;
            }
            current = self.parent_of(&id);
        }
    }

    /// `flattenTree`: pre-order with the active branch first; indentation
    /// grows only at branch points (and the first generation after one).
    fn flatten_tree(&mut self, roots: &[usize]) -> Vec<FlatNode> {
        self.tool_calls.clear();
        let leaf = self.current_leaf_id.clone();
        let mut contains_active = vec![false; self.nodes.len()];
        {
            let mut all = Vec::new();
            let mut stack: Vec<usize> = roots.to_vec();
            while let Some(n) = stack.pop() {
                all.push(n);
                stack.extend(self.nodes[n].children.iter().rev());
            }
            for &n in all.iter().rev() {
                let mut has = leaf.as_deref() == Some(self.nodes[n].id.as_str());
                if self.nodes[n].children.iter().any(|&c| contains_active[c]) {
                    has = true;
                }
                contains_active[n] = has;
            }
        }

        type Item = (usize, usize, bool, bool, bool, Vec<Gutter>, bool);
        let multiple = roots.len() > 1;
        let mut ordered_roots = roots.to_vec();
        ordered_roots.sort_by_key(|&r| !contains_active[r]);
        let mut stack: Vec<Item> = Vec::new();
        for (i, &root) in ordered_roots.iter().enumerate().rev() {
            let is_last = i == ordered_roots.len() - 1;
            let indent = if multiple { 1 } else { 0 };
            stack.push((
                root,
                indent,
                multiple,
                multiple,
                is_last,
                Vec::new(),
                multiple,
            ));
        }

        let mut result = Vec::new();
        while let Some((n, indent, just_branched, show_connector, is_last, gutters, virtual_root)) =
            stack.pop()
        {
            let entry = &self.nodes[n].entry;
            if message_role(entry) == Some("assistant") {
                if let Some(Value::Array(blocks)) =
                    entry.get("message").and_then(|m| m.get("content"))
                {
                    for block in blocks {
                        if str_field(block, "type") == Some("toolCall") {
                            if let Some(id) = str_field(block, "id") {
                                self.tool_calls.insert(
                                    id.to_string(),
                                    (
                                        str_field(block, "name").unwrap_or("").to_string(),
                                        block.get("arguments").cloned().unwrap_or(Value::Null),
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            result.push(FlatNode {
                node: n,
                indent,
                show_connector,
                is_last,
                gutters: gutters.clone(),
                is_virtual_root_child: virtual_root,
            });

            let children = &self.nodes[n].children;
            let multiple_children = children.len() > 1;
            let ordered: Vec<usize> = children
                .iter()
                .copied()
                .filter(|&c| contains_active[c])
                .chain(children.iter().copied().filter(|&c| !contains_active[c]))
                .collect();
            let child_indent = if multiple_children || (just_branched && indent > 0) {
                indent + 1
            } else {
                indent
            };
            let child_gutters =
                self.child_gutters(indent, show_connector, virtual_root, is_last, &gutters);
            for (i, &child) in ordered.iter().enumerate().rev() {
                let child_is_last = i == ordered.len() - 1;
                stack.push((
                    child,
                    child_indent,
                    multiple_children,
                    multiple_children,
                    child_is_last,
                    child_gutters.clone(),
                    false,
                ));
            }
        }
        result
    }

    /// Descendants carry a gutter at this node's connector column, when a
    /// connector is drawn.
    fn child_gutters(
        &self,
        indent: usize,
        show_connector: bool,
        virtual_root: bool,
        is_last: bool,
        gutters: &[Gutter],
    ) -> Vec<Gutter> {
        let mut child = gutters.to_vec();
        if show_connector && !virtual_root {
            let display = if self.multiple_roots {
                indent.saturating_sub(1)
            } else {
                indent
            };
            child.push(Gutter {
                position: display.saturating_sub(1),
                show: !is_last,
            });
        }
        child
    }

    fn is_settings_entry(entry: &Value) -> bool {
        matches!(
            entry_type(entry),
            "label" | "custom" | "model_change" | "thinking_level_change" | "session_info"
        )
    }

    fn passes(&self, flat: &FlatNode, tokens: &[String]) -> bool {
        let node = &self.nodes[flat.node];
        let entry = &node.entry;
        let is_current_leaf = self.current_leaf_id.as_deref() == Some(node.id.as_str());
        // An assistant message with only tool calls says nothing on its own,
        // unless it failed; the current leaf always shows.
        if message_role(entry) == Some("assistant") && !is_current_leaf {
            let message = &entry["message"];
            let has_text = has_text_content(message.get("content"));
            let stop = str_field(message, "stopReason");
            let error_or_aborted =
                stop.is_some_and(|s| !s.is_empty() && s != "stop" && s != "toolUse");
            if !has_text && !error_or_aborted {
                return false;
            }
        }
        let settings_entry = Self::is_settings_entry(entry);
        let passes = match self.filter_mode {
            TreeFilterMode::UserOnly => message_role(entry) == Some("user"),
            TreeFilterMode::NoTools => !settings_entry && message_role(entry) != Some("toolResult"),
            TreeFilterMode::LabeledOnly => node.label.is_some(),
            TreeFilterMode::All => true,
            TreeFilterMode::Default => !settings_entry,
        };
        if !passes {
            return false;
        }
        if !tokens.is_empty() {
            let text = self.searchable_text(flat.node).to_lowercase();
            return tokens.iter().all(|t| text.contains(t.as_str()));
        }
        true
    }

    /// `applyFilter`.
    fn apply_filter(&mut self) {
        if !self.filtered_nodes.is_empty() {
            if let Some(id) = self.selected_id() {
                self.last_selected_id = Some(id);
            }
        }
        let tokens: Vec<String> = self
            .search_query
            .to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();
        let mut filtered: Vec<FlatNode> = self
            .flat_nodes
            .iter()
            .filter(|f| self.passes(f, &tokens))
            .cloned()
            .collect();

        // Descendants of folded nodes go too.
        if !self.folded.is_empty() {
            let mut skip: HashSet<String> = HashSet::new();
            for flat in &self.flat_nodes {
                let node = &self.nodes[flat.node];
                if let Some(parent) = &node.parent_id {
                    if self.folded.contains(parent) || skip.contains(parent) {
                        skip.insert(node.id.clone());
                    }
                }
            }
            filtered.retain(|f| !skip.contains(&self.nodes[f.node].id));
        }
        self.filtered_nodes = filtered;
        self.cursor.set_query(self.filtered_nodes.len());
        self.recalculate_visual_structure();

        if let Some(last) = self.last_selected_id.clone() {
            let nearest = self.find_nearest_visible_index(Some(&last));
            self.cursor.select(nearest);
        }
        if !self.filtered_nodes.is_empty() {
            if let Some(id) = self.selected_id() {
                self.last_selected_id = Some(id);
            }
        }
    }

    /// `recalculateVisualStructure`: re-attach visible entries to their
    /// nearest visible ancestor and redraw the branches.
    fn recalculate_visual_structure(&mut self) {
        if self.filtered_nodes.is_empty() {
            return;
        }
        let visible: HashSet<String> = self
            .filtered_nodes
            .iter()
            .map(|f| self.nodes[f.node].id.clone())
            .collect();
        let visible_ancestor = |id: &str| -> Option<String> {
            let mut current = self.parent_of(id);
            while let Some(c) = current {
                if visible.contains(&c) {
                    return Some(c);
                }
                current = self.parent_of(&c);
            }
            None
        };
        let mut visible_parent: HashMap<String, Option<String>> = HashMap::new();
        let mut visible_children: HashMap<Option<String>, Vec<String>> = HashMap::new();
        visible_children.insert(None, Vec::new());
        for flat in &self.filtered_nodes {
            let id = self.nodes[flat.node].id.clone();
            let ancestor = visible_ancestor(&id);
            visible_parent.insert(id.clone(), ancestor.clone());
            visible_children.entry(ancestor).or_default().push(id);
        }
        let roots = visible_children[&None].clone();
        self.multiple_roots = roots.len() > 1;
        let position: HashMap<String, usize> = self
            .filtered_nodes
            .iter()
            .enumerate()
            .map(|(i, f)| (self.nodes[f.node].id.clone(), i))
            .collect();

        type Item = (String, usize, bool, bool, bool, Vec<Gutter>, bool);
        let multiple = self.multiple_roots;
        let mut stack: Vec<Item> = Vec::new();
        for (i, root) in roots.iter().enumerate().rev() {
            let is_last = i == roots.len() - 1;
            stack.push((
                root.clone(),
                if multiple { 1 } else { 0 },
                multiple,
                multiple,
                is_last,
                Vec::new(),
                multiple,
            ));
        }
        while let Some((
            id,
            indent,
            just_branched,
            show_connector,
            is_last,
            gutters,
            virtual_root,
        )) = stack.pop()
        {
            let Some(&i) = position.get(&id) else {
                continue;
            };
            let child_gutters =
                self.child_gutters(indent, show_connector, virtual_root, is_last, &gutters);
            let flat = &mut self.filtered_nodes[i];
            flat.indent = indent;
            flat.show_connector = show_connector;
            flat.is_last = is_last;
            flat.gutters = gutters;
            flat.is_virtual_root_child = virtual_root;

            let children = visible_children.get(&Some(id)).cloned().unwrap_or_default();
            let multiple_children = children.len() > 1;
            let child_indent = if multiple_children || (just_branched && indent > 0) {
                indent + 1
            } else {
                indent
            };
            for (ci, child) in children.iter().enumerate().rev() {
                stack.push((
                    child.clone(),
                    child_indent,
                    multiple_children,
                    multiple_children,
                    ci == children.len() - 1,
                    child_gutters.clone(),
                    false,
                ));
            }
        }
        self.visible_parent = visible_parent;
        self.visible_children = visible_children;
    }

    /// `getSearchableText`.
    fn searchable_text(&self, n: usize) -> String {
        let node = &self.nodes[n];
        let entry = &node.entry;
        let mut parts: Vec<String> = Vec::new();
        if let Some(label) = &node.label {
            parts.push(label.clone());
        }
        match entry_type(entry) {
            "message" => {
                let message = &entry["message"];
                let role = str_field(message, "role").unwrap_or("");
                parts.push(role.to_string());
                if message.get("content").is_some_and(|c| !c.is_null()) {
                    parts.push(extract_content(message.get("content")));
                }
                if role == "bashExecution" {
                    if let Some(command) = str_field(message, "command").filter(|c| !c.is_empty()) {
                        parts.push(command.to_string());
                    }
                }
            }
            "custom_message" => {
                parts.push(str_field(entry, "customType").unwrap_or("").to_string());
                match entry.get("content") {
                    Some(Value::String(s)) => parts.push(s.clone()),
                    other => parts.push(extract_content(other)),
                }
            }
            "compaction" => parts.push("compaction".into()),
            "branch_summary" => {
                parts.push("branch summary".into());
                parts.push(str_field(entry, "summary").unwrap_or("").to_string());
            }
            "session_info" => {
                parts.push("title".into());
                if let Some(name) = str_field(entry, "name").filter(|n| !n.is_empty()) {
                    parts.push(name.to_string());
                }
            }
            "model_change" => {
                parts.push("model".into());
                parts.push(str_field(entry, "modelId").unwrap_or("").to_string());
            }
            "thinking_level_change" => {
                parts.push("thinking".into());
                parts.push(str_field(entry, "thinkingLevel").unwrap_or("").to_string());
            }
            "custom" => {
                parts.push("custom".into());
                parts.push(str_field(entry, "customType").unwrap_or("").to_string());
            }
            "label" => {
                parts.push("label".into());
                parts.push(str_field(entry, "label").unwrap_or("").to_string());
            }
            _ => {}
        }
        parts.join(" ")
    }

    /// `updateNodeLabel`.
    fn update_node_label(&mut self, entry_id: &str, label: Option<String>) {
        if let Some(&i) = self.index_by_id.get(entry_id) {
            let node = &mut self.nodes[i];
            node.label_timestamp = label
                .as_ref()
                .map(|_| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
            node.label = label;
        }
    }

    fn status_labels(&self) -> String {
        let mut labels = match self.filter_mode {
            TreeFilterMode::NoTools => " [no-tools]",
            TreeFilterMode::UserOnly => " [user]",
            TreeFilterMode::LabeledOnly => " [labeled]",
            TreeFilterMode::All => " [all]",
            TreeFilterMode::Default => "",
        }
        .to_string();
        if self.show_label_timestamps {
            labels.push_str(" [timestamps]");
        }
        labels
    }

    /// `getEntryDisplayText`.
    /// One entry's content (the console adds the `›` marker or gutter and
    /// the selection band): the same text as the old string renderer, as
    /// [`Palette`] spans. The selected row reads bold.
    fn entry_line(&self, n: usize, selected: bool) -> Line<'static> {
        let pal = Palette::current();
        let entry = &self.nodes[n].entry;
        let mut spans: Vec<Span<'static>> = match entry_type(entry) {
            "message" => {
                let message = &entry["message"];
                match str_field(message, "role").unwrap_or("") {
                    "user" => vec![
                        Span::styled("user: ".to_string(), pal.acc()),
                        Span::raw(normalize(&extract_content(message.get("content")))),
                    ],
                    "assistant" => {
                        let text = normalize(&extract_content(message.get("content")));
                        let mut spans = vec![Span::styled("assistant: ".to_string(), pal.ok())];
                        if !text.is_empty() {
                            spans.push(Span::raw(text));
                        } else if str_field(message, "stopReason") == Some("aborted") {
                            spans.push(Span::styled("(aborted)".to_string(), pal.faint()));
                        } else if let Some(err) =
                            str_field(message, "errorMessage").filter(|e| !e.is_empty())
                        {
                            spans.push(Span::styled(js_slice(&normalize(err), 80), pal.err()));
                        } else {
                            spans.push(Span::styled("(no content)".to_string(), pal.faint()));
                        }
                        spans
                    }
                    "toolResult" => {
                        let call =
                            str_field(message, "toolCallId").and_then(|id| self.tool_calls.get(id));
                        match call {
                            Some((name, args)) => {
                                vec![Span::styled(format_tool_call(name, args), pal.faint())]
                            }
                            None => vec![Span::styled(
                                format!("[{}]", str_field(message, "toolName").unwrap_or("tool")),
                                pal.faint(),
                            )],
                        }
                    }
                    "bashExecution" => vec![Span::styled(
                        format!(
                            "[bash]: {}",
                            normalize(str_field(message, "command").unwrap_or(""))
                        ),
                        pal.dim(),
                    )],
                    role => vec![Span::styled(format!("[{role}]"), pal.dim())],
                }
            }
            "custom_message" => {
                let content = match entry.get("content") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Array(blocks)) => blocks
                        .iter()
                        .filter(|b| str_field(b, "type") == Some("text"))
                        .filter_map(|b| str_field(b, "text"))
                        .collect(),
                    _ => String::new(),
                };
                vec![
                    Span::styled(
                        format!("[{}]: ", str_field(entry, "customType").unwrap_or("")),
                        pal.legacy("customMessageLabel"),
                    ),
                    Span::raw(normalize(&content)),
                ]
            }
            "compaction" => {
                let before = entry
                    .get("tokensBefore")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                vec![Span::styled(
                    format!("[compaction: {}k tokens]", (before / 1000.0).round() as i64),
                    pal.legacy("borderAccent"),
                )]
            }
            "branch_summary" => vec![
                Span::styled("[branch summary]: ".to_string(), pal.warn()),
                Span::raw(normalize(str_field(entry, "summary").unwrap_or(""))),
            ],
            "model_change" => vec![Span::styled(
                format!("[model: {}]", str_field(entry, "modelId").unwrap_or("")),
                pal.dim(),
            )],
            "thinking_level_change" => vec![Span::styled(
                format!(
                    "[thinking: {}]",
                    str_field(entry, "thinkingLevel").unwrap_or("")
                ),
                pal.dim(),
            )],
            "custom" => vec![Span::styled(
                format!("[custom: {}]", str_field(entry, "customType").unwrap_or("")),
                pal.dim(),
            )],
            "label" => vec![Span::styled(
                format!(
                    "[label: {}]",
                    str_field(entry, "label").unwrap_or("(cleared)")
                ),
                pal.dim(),
            )],
            "session_info" => match str_field(entry, "name").filter(|n| !n.is_empty()) {
                Some(name) => vec![
                    Span::styled("[title: ".to_string(), pal.dim()),
                    Span::styled(name.to_string(), pal.dim()),
                    Span::styled("]".to_string(), pal.dim()),
                ],
                None => vec![
                    Span::styled("[title: ".to_string(), pal.dim()),
                    Span::styled("empty".to_string(), italic(pal.dim())),
                    Span::styled("]".to_string(), pal.dim()),
                ],
            },
            _ => Vec::new(),
        };
        if selected {
            for span in &mut spans {
                span.style = bold(span.style);
            }
        }
        Line::from(spans)
    }

    /// `isFoldable`: has visible children, and is a root or a segment start.
    fn is_foldable(&self, id: &str) -> bool {
        let has_children = self
            .visible_children
            .get(&Some(id.to_string()))
            .is_some_and(|c| !c.is_empty());
        if !has_children {
            return false;
        }
        match self.visible_parent.get(id).cloned().flatten() {
            None => true,
            Some(parent) => self
                .visible_children
                .get(&Some(parent))
                .is_some_and(|s| s.len() > 1),
        }
    }

    /// `findBranchSegmentStart`.
    fn find_branch_segment_start(&self, down: bool) -> usize {
        let Some(selected) = self.selected_id() else {
            return self.cursor.selected();
        };
        let index: HashMap<&str, usize> = self
            .filtered_nodes
            .iter()
            .enumerate()
            .map(|(i, f)| (self.nodes[f.node].id.as_str(), i))
            .collect();
        let mut current = selected;
        if down {
            loop {
                let children = self
                    .visible_children
                    .get(&Some(current.clone()))
                    .cloned()
                    .unwrap_or_default();
                match children.len() {
                    0 => return index[current.as_str()],
                    1 => current = children[0].clone(),
                    _ => return index[children[0].as_str()],
                }
            }
        }
        loop {
            let Some(parent) = self.visible_parent.get(&current).cloned().flatten() else {
                return index[current.as_str()];
            };
            let siblings = self
                .visible_children
                .get(&Some(parent.clone()))
                .map_or(0, Vec::len);
            if siblings > 1 {
                let start = index[current.as_str()];
                if start < self.cursor.selected() {
                    return start;
                }
            }
            current = parent;
        }
    }

    fn set_filter(&mut self, mode: TreeFilterMode) {
        self.filter_mode = mode;
        self.folded.clear();
        self.apply_filter();
    }

    fn toggle_filter(&mut self, mode: TreeFilterMode) {
        let next = if self.filter_mode == mode {
            TreeFilterMode::Default
        } else {
            mode
        };
        self.set_filter(next);
    }

    fn cycle_filter(&mut self, forward: bool) {
        let n = FILTER_MODES.len();
        let current = FILTER_MODES
            .iter()
            .position(|m| *m == self.filter_mode)
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % n
        } else {
            (current + n - 1) % n
        };
        self.set_filter(FILTER_MODES[next]);
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<TreeEvent> {
        std::mem::take(&mut self.events)
    }
}

impl TreeList {
    /// The FLIGHT PATH count: the visible entries, or the matches of the
    /// total while searching.
    fn count_text(&self) -> String {
        let total = self.flat_nodes.len();
        if self.search_query.is_empty() {
            nodes(self.filtered_nodes.len())
        } else {
            format!("{} of {}", nodes(self.filtered_nodes.len()), total)
        }
    }

    /// One entry row (content only; the console adds the `›` marker or
    /// gutter and the selection band): the graph prefix, the fold and path
    /// markers, the label, and the entry's text.
    fn console_row(&self, i: usize, selected: bool) -> Line<'static> {
        let pal = Palette::current();
        let flat = &self.filtered_nodes[i];
        let node = &self.nodes[flat.node];
        let display_indent = if self.multiple_roots {
            flat.indent.saturating_sub(1)
        } else {
            flat.indent
        };
        let connector = flat.show_connector && !flat.is_virtual_root_child;
        let connector_position = connector.then(|| display_indent as isize - 1);
        let folded = self.folded.contains(&node.id);
        let mut prefix = String::new();
        for c in 0..display_indent * 3 {
            let level = c / 3;
            let pos = c % 3;
            if let Some(gutter) = flat.gutters.iter().find(|g| g.position == level) {
                prefix.push(if pos == 0 && gutter.show { '│' } else { ' ' });
            } else if connector_position == Some(level as isize) {
                prefix.push(match pos {
                    0 if flat.is_last => '└',
                    0 => '├',
                    1 if folded => '⊞',
                    1 if self.is_foldable(&node.id) => '⊟',
                    1 => '─',
                    _ => ' ',
                });
            } else {
                prefix.push(' ');
            }
        }
        let mut spans = vec![Span::styled(prefix, pal.dim())];
        // Roots have no connector to carry the fold marker.
        if folded && !connector {
            spans.push(Span::styled("⊞ ".to_string(), pal.acc()));
        }
        if self.active_path_ids.contains(&node.id) {
            spans.push(Span::styled("• ".to_string(), pal.acc()));
        }
        // The design's cyan tag on the live node (M8d7).
        if self.current_leaf_id.as_deref() == Some(node.id.as_str()) {
            spans.push(Span::styled("YOU ARE HERE ".to_string(), pal.acc()));
        }
        if let Some(label) = &node.label {
            spans.push(Span::styled(format!("[{label}] "), pal.warn()));
        }
        match (&node.label, &node.label_timestamp) {
            (Some(_), Some(ts)) if self.show_label_timestamps => {
                spans.push(Span::styled(
                    format!("{} ", format_label_timestamp(ts)),
                    pal.faint(),
                ));
            }
            _ => {}
        }
        let mut content = self.entry_line(flat.node, selected);
        spans.append(&mut content.spans);
        Line::from(spans)
    }

    /// The cursor's window as console rows, with the selection remapped
    /// into it (the rows arrive pre-windowed, so the console draws one page).
    fn console_rows(&self) -> (Vec<Line<'static>>, Option<usize>) {
        let window = self.cursor.window();
        let selected = self.cursor.selected();
        let rows = window
            .clone()
            .map(|i| self.console_row(i, i == selected))
            .collect();
        (rows, window.clone().position(|i| i == selected))
    }

    /// Rows below the list: the empty-list line and the scroll position
    /// with the filter and timestamp flags.
    fn footer_lines(&self) -> Vec<Line<'static>> {
        let pal = Palette::current();
        let mut out = Vec::new();
        if self.filtered_nodes.is_empty() {
            out.push(Line::from(vec![Span::styled(
                "  No entries found".to_string(),
                pal.faint(),
            )]));
            out.push(Line::from(vec![Span::styled(
                format!("  (0/0){}", self.status_labels()),
                pal.faint(),
            )]));
        } else {
            out.push(Line::from(vec![Span::styled(
                format!("  {}{}", self.cursor.position(), self.status_labels()),
                pal.faint(),
            )]));
        }
        out
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if self.cursor.handle_key(data, &kb) {
            // Navigation: the cursor clamps at both ends.
        } else if kb.matches(data, "app.tree.foldOrUp") {
            match self.selected_id() {
                Some(id) if self.is_foldable(&id) && !self.folded.contains(&id) => {
                    self.folded.insert(id);
                    self.apply_filter();
                }
                _ => {
                    let target = self.find_branch_segment_start(false);
                    self.cursor.select(target);
                }
            }
        } else if kb.matches(data, "app.tree.unfoldOrDown") {
            match self.selected_id() {
                Some(id) if self.folded.contains(&id) => {
                    self.folded.remove(&id);
                    self.apply_filter();
                }
                _ => {
                    let target = self.find_branch_segment_start(true);
                    self.cursor.select(target);
                }
            }
        } else if kb.matches(data, "tui.editor.cursorLeft") {
            self.cursor.page(false);
        } else if kb.matches(data, "tui.editor.cursorRight") {
            self.cursor.page(true);
        } else if kb.matches(data, "tui.select.confirm") {
            if let Some(id) = self.selected_id() {
                self.events.push(TreeEvent::Select(id));
            }
        } else if kb.matches(data, "tui.select.cancel") {
            if self.search_query.is_empty() {
                self.events.push(TreeEvent::Cancel);
            } else {
                self.search_query.clear();
                self.folded.clear();
                self.apply_filter();
            }
        } else if kb.matches(data, "app.tree.filter.default") {
            self.set_filter(TreeFilterMode::Default);
        } else if kb.matches(data, "app.tree.filter.noTools") {
            self.toggle_filter(TreeFilterMode::NoTools);
        } else if kb.matches(data, "app.tree.filter.userOnly") {
            self.toggle_filter(TreeFilterMode::UserOnly);
        } else if kb.matches(data, "app.tree.filter.labeledOnly") {
            self.toggle_filter(TreeFilterMode::LabeledOnly);
        } else if kb.matches(data, "app.tree.filter.all") {
            self.toggle_filter(TreeFilterMode::All);
        } else if kb.matches(data, "app.tree.filter.cycleBackward") {
            self.cycle_filter(false);
        } else if kb.matches(data, "app.tree.filter.cycleForward") {
            self.cycle_filter(true);
        } else if kb.matches(data, "tui.editor.deleteCharBackward") {
            if self.search_query.pop().is_some() {
                self.folded.clear();
                self.apply_filter();
            }
        } else if kb.matches(data, "app.tree.editLabel") {
            if let Some(f) = self.filtered_nodes.get(self.cursor.selected()) {
                let node = &self.nodes[f.node];
                self.label_edit = Some((node.id.clone(), node.label.clone()));
            }
        } else if kb.matches(data, "app.tree.toggleLabelTimestamp") {
            self.show_label_timestamps = !self.show_label_timestamps;
        } else {
            let control = data.chars().any(|c| {
                let code = c as u32;
                code < 32 || code == 0x7f || (0x80..=0x9f).contains(&code)
            });
            if !control && !data.is_empty() {
                self.search_query.push_str(data);
                self.folded.clear();
                self.apply_filter();
            }
        }
    }
}

/// `1 node`, `N nodes`.
fn nodes(count: usize) -> String {
    if count == 1 {
        "1 node".to_string()
    } else {
        format!("{count} nodes")
    }
}

/// The hint foot with literal keys, so the bytes hold on macOS and Linux:
/// the longest leading run that fits `available`, like the old top row.
fn tree_hint(available: usize) -> Line<'static> {
    const HINTS: [(&str, &str); 6] = [
        ("↑↓", "move"),
        ("enter", "go here"),
        ("f", "fork"),
        ("alt+l", "label"),
        ("alt+t", "timestamps"),
        ("esc", "close"),
    ];
    for count in (1..=HINTS.len()).rev() {
        let plain = HINTS[..count]
            .iter()
            .map(|(key, description)| format!("{key} {description}"))
            .collect::<Vec<_>>()
            .join(" · ");
        if visible_width(&plain) <= available {
            return hints(&HINTS[..count]);
        }
    }
    hints(&HINTS[..1])
}

/// `LabelInput`: the label editor that replaces the tree while open.
struct LabelInput {
    input: Input,
    entry_id: String,
}

impl LabelInput {
    fn new(entry_id: String, current: Option<&str>) -> Self {
        let mut input = Input::new();
        style_input(&mut input);
        if let Some(label) = current {
            input.set_value(label);
        }
        Self { input, entry_id }
    }

    /// The label editor as console rows: the prompt and the value with a
    /// `▏` caret at the input's cursor while focused.
    fn console_lines(&self, focused: bool) -> Vec<Line<'static>> {
        let pal = Palette::current();
        let value = self.input.get_value().to_string();
        let cursor = self.input.get_cursor().min(value.len());
        let (before, after) = value.split_at(if value.is_char_boundary(cursor) {
            cursor
        } else {
            value.len()
        });
        let mut row = vec![Span::raw("  ".to_string()), Span::raw(before.to_string())];
        if focused {
            row.push(Span::styled("▏".to_string(), pal.warn()));
        }
        row.push(Span::raw(after.to_string()));
        vec![
            Line::from(vec![Span::styled(
                "  Label (empty to remove):".to_string(),
                pal.faint(),
            )]),
            Line::from(row),
        ]
    }
}

/// `TreeSelectorComponent`: the FLIGHT PATH console, titled
/// `FLIGHT PATH · /tree`, with the query row and the hint foot.
pub struct TreeSelectorComponent {
    list: Rc<RefCell<TreeList>>,
    label_input: Rc<RefCell<Option<LabelInput>>>,
    focused: bool,
    auto_cancel_at: Option<Instant>,
}

impl TreeSelectorComponent {
    pub fn new(
        tree: &[SessionTreeNode],
        current_leaf_id: Option<&str>,
        terminal_height: usize,
        initial_selected_id: Option<&str>,
        initial_filter_mode: Option<TreeFilterMode>,
    ) -> Self {
        let max_visible = (terminal_height / 2).max(5);
        let list = Rc::new(RefCell::new(TreeList::new(
            tree,
            current_leaf_id,
            max_visible,
            initial_selected_id,
            initial_filter_mode,
        )));
        let label_input: Rc<RefCell<Option<LabelInput>>> = Rc::default();
        let auto_cancel_at = tree
            .is_empty()
            .then(|| Instant::now() + Duration::from_millis(100));
        Self {
            list,
            label_input,
            focused: false,
            auto_cancel_at,
        }
    }

    /// `getTreeList()`.
    pub fn tree_list(&self) -> Rc<RefCell<TreeList>> {
        self.list.clone()
    }

    /// Whether the label editor is open.
    pub fn is_editing_label(&self) -> bool {
        self.label_input.borrow().is_some()
    }

    /// Events since the last call, including the empty tree's auto-cancel
    /// once its time is up.
    pub fn poll(&mut self, now: Instant) -> Vec<TreeEvent> {
        let mut events = self.list.borrow_mut().take_events();
        if self.auto_cancel_at.is_some_and(|at| now >= at) {
            self.auto_cancel_at = None;
            events.push(TreeEvent::Cancel);
        }
        events
    }

    /// When the auto-cancel is due, if pending.
    pub fn deadline(&self) -> Option<Instant> {
        self.auto_cancel_at
    }

    fn label_input_key(&mut self, data: &str) {
        let kb = get_keybindings();
        let mut slot = self.label_input.borrow_mut();
        let Some(input) = slot.as_mut() else {
            return;
        };
        if kb.matches(data, "tui.select.confirm") {
            let value = input.input.get_value().trim().to_string();
            let label = (!value.is_empty()).then_some(value);
            let id = input.entry_id.clone();
            *slot = None;
            drop(slot);
            let mut list = self.list.borrow_mut();
            list.update_node_label(&id, label.clone());
            list.events.push(TreeEvent::LabelChange(id, label));
        } else if kb.matches(data, "tui.select.cancel") {
            *slot = None;
        } else {
            input.input.handle_input_with(data, &kb);
        }
    }

    /// The label editor as console rows (see `LabelInput::console_lines`).
    fn label_lines(&self) -> Vec<Line<'static>> {
        let focused = self.focused;
        let slot = self.label_input.borrow();
        match slot.as_ref() {
            Some(editor) => editor.console_lines(focused),
            None => Vec::new(),
        }
    }
}

impl Component for TreeSelectorComponent {
    /// The FLIGHT PATH console: the `▸` query row, one row per entry with
    /// `›` on the selection band, the position footer, and the hint foot.
    /// The label editor replaces the tree while open. Keys, filtering
    /// and events are unchanged.
    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        let mut console = Console::new("FLIGHT PATH · /tree", self.list.borrow().count_text());
        if self.label_input.borrow().is_some() {
            console.top = self.label_lines();
            console.hint = hints(&[("enter", "save"), ("esc", "cancel")]);
        } else {
            let list = self.list.borrow();
            console.query = Some(query_row(list.search_query(), false));
            let (rows, selected) = list.console_rows();
            console.rows = rows;
            console.selected = selected;
            console.bottom = list.footer_lines();
            drop(list);
            console.hint = tree_hint(width.saturating_sub(12));
        }
        console.render(width)
    }

    fn handle_input(&mut self, data: &str) {
        if self.label_input.borrow().is_some() {
            self.label_input_key(data);
            return;
        }
        self.list.borrow_mut().handle_input(data);
        let edit = self.list.borrow_mut().label_edit.take();
        if let Some((id, current)) = edit {
            let mut input = LabelInput::new(id, current.as_deref());
            input.input.set_focused(self.focused);
            *self.label_input.borrow_mut() = Some(input);
        }
    }
    fn invalidate(&mut self) {}

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        if let Some(input) = self.label_input.borrow_mut().as_mut() {
            input.input.set_focused(focused);
        }
    }
}
