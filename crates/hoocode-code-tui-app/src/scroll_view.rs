//! Reading back through the transcript (`modes/interactive/scroll-view.ts`).
//!
//! The TUI owns the mechanism: a pinned window over the line buffer, painted
//! on the alternate screen. This owns what the TUI cannot know: which keys
//! mean what, and how the indicator on the bottom row looks in the theme.
//!
//! At the prompt only the pager keys are live (page up/down, the two ends,
//! jump by turn, search), none of which the prompt uses. Once the view is
//! pinned it captures keys like a picker: line steps on the arrows, escape
//! back to live. The plain arrows belong to the prompt while it holds text
//! (cursor, and the task panel from the first line), so they step the view
//! only over an empty prompt. The shifted arrows step the pinned view and are
//! the prompt's history at the live prompt. Escape goes back to live, and also
//! reaches the prompt when an agent run is going, so it aborts the run as it
//! does from the tail. Anything that is not a scroll key un-pins the view and
//! then does what it always did, so typing never echoes into a prompt scrolled
//! off screen.
//!
//! hoocode installs editor actions for the prompt keys and an input listener
//! for the pinned view. Both run here in one TUI input interceptor: an
//! interceptor gets the TUI in hand, which a Rust listener cannot. The prompt
//! keys are gated on the editor holding focus, as editor actions are, and app
//! actions win over the editor's own keys in both.

use hoocode_code_tui_keybindings::key_hint_text;
use std::cell::RefCell;
use std::rc::Rc;

use hoocode_tui_keys::{get_keybindings, is_key_release};
use hoocode_tui_render::{Component, ComponentHandle, Container, Fold, ScrollStatus, Tui};

use crate::status_line::{status_line_ansi, KeyHint, ScrollInfo, SearchInfo, StatusLineData};

/// Which way to jump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnDirection {
    Previous,
    Next,
}

/// Whether a chat child stands for something the user said.
pub type IsUserMessage = Rc<dyn Fn(&ComponentHandle) -> bool>;
/// Whether the prompt holds no text.
pub type EditorEmpty = Rc<dyn Fn() -> bool>;
/// Whether an agent run is going (streaming or backing off a retry).
pub type AgentBusy = Rc<dyn Fn() -> bool>;

/// `jumpToUserMessage`: pin the view to the previous or next user message.
/// Rows come from the last frame's offsets: a message's offset inside the
/// chat plus the chat's offset in the scrolled container. That container is
/// the full-screen `body` (the header and chat) when there is one, else the
/// root. False when there is nowhere to go (nothing rendered yet, no user
/// message, or already at the end).
pub fn jump_to_user_message(
    ui: &mut Tui,
    chat: &Rc<RefCell<Container>>,
    body: Option<&Rc<RefCell<Container>>>,
    direction: TurnDirection,
    is_user_message: &dyn Fn(&ComponentHandle) -> bool,
) -> bool {
    // The offsets are kept at the width the transcript was last laid out at.
    let width = ui.transcript_width();
    let chat_handle: ComponentHandle = chat.clone();
    let (base, chat_offsets) = match body {
        Some(body) => {
            let body = body.borrow();
            let Some(index) = body
                .children()
                .iter()
                .position(|c| Rc::ptr_eq(c, &chat_handle))
            else {
                return false;
            };
            let Some(body_offsets) = body.child_row_offsets(width) else {
                return false;
            };
            let Some(chat_offsets) = chat.borrow().child_row_offsets(width) else {
                return false;
            };
            (body_offsets[index] as i64, chat_offsets)
        }
        None => {
            let (Some(root_offsets), Some(chat_offsets)) = (
                ui.child_row_offsets(width),
                chat.borrow().child_row_offsets(width),
            ) else {
                return false;
            };
            let Some(index) = ui
                .children()
                .iter()
                .position(|c| Rc::ptr_eq(c, &chat_handle))
            else {
                return false;
            };
            (root_offsets[index] as i64, chat_offsets)
        }
    };
    let rows: Vec<i64> = chat
        .borrow()
        .children()
        .iter()
        .zip(&chat_offsets)
        .filter(|(child, _)| is_user_message(child))
        .map(|(_, offset)| base + *offset as i64)
        .collect();
    if rows.is_empty() {
        return false;
    }
    // Live counts as below the last message, so the first `Previous` lands
    // on the most recent one.
    let here = ui.get_scroll_position().map_or(i64::MAX, |(top, ..)| top);
    let target = match direction {
        TurnDirection::Previous => rows.iter().rev().find(|row| **row < here),
        TurnDirection::Next => rows.iter().find(|row| **row > here),
    };
    match target {
        Some(&row) => {
            ui.scroll_to_row(row, None);
            true
        }
        None => false,
    }
}

/// The base buffer row of `child` in `parent`'s last render at `width`.
/// `None` before the first render, at another width, or when the child is
/// gone.
pub fn child_base(parent: &Container, child: &ComponentHandle, width: u16) -> Option<usize> {
    let offsets = parent.child_row_offsets(width)?;
    let children = parent.children();
    if children.len() != offsets.len() {
        return None;
    }
    children
        .iter()
        .position(|c| Rc::ptr_eq(c, child))
        .map(|i| offsets[i])
}

/// A block that opens in place, as the chat's last render drew it: the
/// chat child that holds it, where that child starts in the chat, and the
/// block's rows and state (rows count from the child's first row).
#[derive(Clone)]
pub struct FoldRef {
    /// The chat child holding the block.
    pub host: ComponentHandle,
    /// The child's first row in the chat, when the chat is laid out at the
    /// asked width.
    pub base: Option<usize>,
    /// The block in the child's last render.
    pub fold: Fold,
}

impl FoldRef {
    /// The header's first row in the chat.
    pub fn header_row(&self) -> Option<usize> {
        Some(self.base? + *self.fold.header.first()?)
    }
}

/// What a click on a transcript row hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldHit {
    /// A header row: the click toggles the block.
    Header,
    /// The fold line under a partly shown body: the click opens the block.
    FoldLine,
}

/// Every block of the chat that opens in place, in transcript order: the
/// one list a click, the focus walk, `enter`, copy and open-all read. A
/// chat child offers its blocks through `Component::fold_host`.
pub fn chat_folds(chat: &Rc<RefCell<Container>>, width: u16) -> Vec<FoldRef> {
    let chat_ref = chat.borrow();
    let offsets = chat_ref.child_row_offsets(width);
    let mut out = Vec::new();
    for (index, child) in chat_ref.children().iter().enumerate() {
        let Ok(component) = child.try_borrow() else {
            continue;
        };
        let Some(host) = component.fold_host() else {
            continue;
        };
        let base = offsets.as_ref().and_then(|o| o.get(index).copied());
        out.extend(host.folds().into_iter().map(|fold| FoldRef {
            host: child.clone(),
            base,
            fold,
        }));
    }
    out
}

/// The block with this id, if the chat still holds it.
pub fn find_fold(chat: &Rc<RefCell<Container>>, width: u16, id: &str) -> Option<FoldRef> {
    chat_folds(chat, width)
        .into_iter()
        .find(|found| found.fold.id == id)
}

/// Hit-test a chat row (`rel` counts from the chat's first row) against the
/// last render: the block whose header holds it, or whose fold line it is.
/// Any other body row, and a row outside every block, is `None` (a click
/// there does nothing).
pub fn fold_at(
    chat: &Rc<RefCell<Container>>,
    width: u16,
    rel: usize,
) -> Option<(FoldRef, FoldHit)> {
    let (child, base) = {
        let chat_ref = chat.borrow();
        let offsets = chat_ref.child_row_offsets(width)?;
        let children = chat_ref.children();
        let i = offsets.iter().rposition(|o| *o <= rel)?;
        (children[i].clone(), offsets[i])
    };
    let row = rel - base;
    let folds = child.try_borrow().ok()?.fold_host()?.folds();
    folds.into_iter().find_map(|fold| {
        let hit = if fold.header.contains(&row) {
            FoldHit::Header
        } else if fold.fold_line == Some(row) && !fold.expanded {
            FoldHit::FoldLine
        } else {
            return None;
        };
        Some((
            FoldRef {
                host: child.clone(),
                base: Some(base),
                fold,
            },
            hit,
        ))
    })
}

/// The chat's first row in the transcript buffer, from the last layout at
/// `width`: the full-screen body holds the chat.
fn chat_base(
    chat: &Rc<RefCell<Container>>,
    body: &Rc<RefCell<Container>>,
    width: u16,
) -> Option<usize> {
    let chat_handle: ComponentHandle = chat.clone();
    child_base(&body.borrow(), &chat_handle, width)
}

/// The transcript buffer row of a block's header, from the last layout.
pub fn fold_buffer_row(
    ui: &Tui,
    chat: &Rc<RefCell<Container>>,
    body: Option<&Rc<RefCell<Container>>>,
    id: &str,
) -> Option<i64> {
    let width = ui.transcript_width();
    let base = chat_base(chat, body?, width)?;
    let header = find_fold(chat, width, id)?.header_row()?;
    Some((base + header) as i64)
}

/// Opens or closes the block `id` of `host` and keeps the view still.
///
/// With `screen` (the header's screen row, counted from the transcript's
/// first screen row) the header stays on that row: a click, and `enter` on
/// a header in view. The view pins when it has to: a live view that still
/// shows the header on its row at the tail stays live, and a transcript
/// that fits the screen whole cannot hold a row, so it stays live too.
/// Without `screen` the pinned top row stays where it is (the header is
/// off screen), and a live view stays live.
///
/// The layout is taken fresh after the change, so the row is placed in the
/// frame that will be painted, not the one before it. False when the host
/// has no such block.
pub fn set_fold_held(
    ui: &mut Tui,
    chat: &Rc<RefCell<Container>>,
    body: Option<&Rc<RefCell<Container>>>,
    host: &ComponentHandle,
    id: &str,
    expanded: bool,
    screen: Option<i64>,
) -> bool {
    let top = capture_top(ui, chat, body);
    let changed = host
        .borrow_mut()
        .fold_host_mut()
        .is_some_and(|folds| folds.set_fold_expanded(id, expanded));
    if !changed {
        return false;
    }
    match (screen, body) {
        (Some(screen), Some(body)) => hold_header(ui, chat, body, id, screen),
        _ => reanchor_top(ui, chat, body, top),
    }
    true
}

/// After a block changed, puts its header back on screen row `screen`.
fn hold_header(
    ui: &mut Tui,
    chat: &Rc<RefCell<Container>>,
    body: &Rc<RefCell<Container>>,
    id: &str,
    screen: i64,
) {
    let width = ui.transcript_width();
    // A fresh layout: the offsets and the header rows of the next frame.
    let total = body.borrow_mut().render(width).len() as i64;
    let header = chat_base(chat, body, width).and_then(|base| {
        find_fold(chat, width, id)
            .and_then(|found| found.header_row())
            .map(|row| (base + row) as i64)
    });
    let Some(header) = header else {
        return;
    };
    // Rows that moved in place are not new output.
    ui.reset_new_lines_count();
    let max_top = total - ui.transcript_view_rows();
    if max_top <= 0 {
        // Everything fits: there is nothing to scroll, so the view is live.
        ui.scroll_to_live();
        return;
    }
    let top = (header - screen).clamp(0, max_top);
    if !ui.scroll_pinned() && top >= max_top {
        // Live, and the tail still shows the header on its row.
        return;
    }
    ui.restore_scroll_top(Some(top));
}

/// Plain typed text rather than a chord (control bytes and escapes are keys).
fn is_typed_text(data: &str) -> bool {
    !data.is_empty() && data.chars().all(|c| (c as u32) >= 32 && c as u32 != 127)
}

/// A key's short name for a hint: the first key bound to `id`, lowercase,
/// `escape` as `esc`. Empty when nothing is bound.
fn hint_key(id: &str) -> String {
    key_hint_text(id).to_lowercase()
}

/// `key action` hints, leaving out keys that are not bound.
fn hints(pairs: &[(&str, &str)]) -> Vec<KeyHint> {
    pairs
        .iter()
        .filter_map(|(id, action)| {
            let key = hint_key(id);
            (!key.is_empty()).then(|| KeyHint::new(key, *action))
        })
        .collect()
}

/// The pinned view's hints with a block focused: `enter open · y copy ·
/// { } move · esc clear`. The brace pair is one hint, its two keys read from
/// the bindings and joined with a space.
fn focused_hints() -> Vec<KeyHint> {
    let mut keys = hints(&[
        ("app.scroll.expandFocused", "open"),
        ("app.scroll.copyFocused", "copy"),
    ]);
    let previous = hint_key("app.scroll.focusPrevious");
    let next = hint_key("app.scroll.focusNext");
    if !previous.is_empty() && !next.is_empty() {
        keys.push(KeyHint::new(format!("{previous} {next}"), "move"));
    }
    keys.extend(hints(&[("app.scroll.exit", "clear")]));
    keys
}

/// U21 single view (P2): which tool block the pinned view has focused, if
/// any. The id is a tool call id: blocks come and go (a retry takes chains
/// out), so a missing id resolves to no focus.
#[derive(Debug, Default)]
pub struct TranscriptFocus {
    focused_call: Option<String>,
}

impl TranscriptFocus {
    /// No focus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a block is focused.
    pub fn is_focused(&self) -> bool {
        self.focused_call.is_some()
    }

    /// The focused block's tool call id, if any.
    pub fn focused_call(&self) -> Option<&str> {
        self.focused_call.as_deref()
    }

    /// Focus the block with this tool call id.
    pub fn focus(&mut self, call_id: &str) {
        self.focused_call = Some(call_id.to_string());
    }

    /// Drop the focus.
    pub fn clear(&mut self) {
        self.focused_call = None;
    }

    /// The focused block among `blocks`, in order; `None` when its call is
    /// gone (a retry took its chain out).
    pub fn resolve<'a>(
        &self,
        blocks: &'a [hoocode_code_tui_widgets::tool_chain::ToolBlock],
    ) -> Option<&'a hoocode_code_tui_widgets::tool_chain::ToolBlock> {
        let call = self.focused_call()?;
        blocks
            .iter()
            .find(|block| block.borrow().tool_call_id() == call)
    }
}

/// What `ctrl+x o` does to the blocks that open in place: open every one,
/// or close the open ones when any is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleAll {
    /// The state every block that can open is set to.
    pub expanded: bool,
    /// Positions (in the input order) of the blocks whose state changes.
    pub changed: Vec<usize>,
}

/// The plan for `ctrl+x o`, from each block's `(expanded, can_open)` state
/// in transcript order. Opens every block that can open unless one is
/// already open, in which case it closes the open ones. A block that cannot
/// open (no output, a stopped call) is left alone and never counts as open.
/// An older tool row keeps its output, so it opens and closes like any
/// other.
pub fn toggle_all_plan(blocks: &[(bool, bool)]) -> ToggleAll {
    let openable: Vec<(usize, bool)> = blocks
        .iter()
        .enumerate()
        .filter(|(_, (_, can_open))| *can_open)
        .map(|(index, (expanded, _))| (index, *expanded))
        .collect();
    let expanded = !openable.iter().any(|(_, open)| *open);
    let changed = openable
        .iter()
        .filter(|(_, open)| *open != expanded)
        .map(|(index, _)| *index)
        .collect();
    ToggleAll { expanded, changed }
}

/// A pinned-view transcript key the mode runs after the pass: the scroll
/// view queues it and stays dumb, so the mode keeps the chains, the session
/// and the clipboard. Esc, `q` and End clear the shared focus themselves, so
/// they ride no intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusIntent {
    /// `{`: focus the previous tool block.
    Previous,
    /// `}`: focus the next tool block.
    Next,
    /// `enter`: toggle the focused block.
    Toggle,
    /// `y`: copy the focused block's raw output.
    Copy,
}

/// The status line's left slot for a pinned view: `search  query  1 match`
/// while searching, else `scrolled 43–61 of 78 · ↓ 3 new`. The keys come
/// from the bindings: `enter next · esc close` for a search (`enter find`
/// while the query is typed), `ctrl+x u focus · esc live` for the scrolled
/// view, and with a block focused `enter open · y copy · { } move · esc
/// clear`. With a picker focused the arrows and escape are its keys, so none
/// are named.
pub fn scroll_slot(status: &ScrollStatus, focus: bool) -> (Option<ScrollInfo>, Option<SearchInfo>) {
    if let Some(search) = &status.search {
        let enter = if search.typing { "find" } else { "next" };
        return (
            None,
            Some(SearchInfo {
                query: search.query.clone(),
                count: search.count,
                index: if search.typing { 0 } else { search.index },
                keys: hints(&[("tui.select.confirm", enter), ("app.scroll.exit", "close")]),
            }),
        );
    }
    let keys = if status.picker {
        Vec::new()
    } else if focus {
        focused_hints()
    } else {
        hints(&[
            ("app.scroll.focusNewest", "focus"),
            ("app.scroll.exit", "live"),
        ])
    };
    (
        Some(ScrollInfo {
            top: status.top.max(0) as usize,
            bottom: status.bottom.max(0) as usize,
            total: status.total.max(0) as usize,
            new_below: status.new_lines.max(0) as usize,
            keys,
        }),
        None,
    )
}

/// `formatStatus`: the status line while the view is pinned: `base` (the
/// session facts the mode keeps current) with the scroll or search slot on
/// the left, `status.width` columns wide. `focus` names the focused block's
/// keys once the mode has focused one.
pub fn format_scroll_status(status: &ScrollStatus, base: &StatusLineData, focus: bool) -> String {
    let (scroll, search) = scroll_slot(status, focus);
    let data = StatusLineData {
        scroll,
        search,
        ..base.clone()
    };
    status_line_ansi(&data, status.width.max(0) as usize)
}

/// The key state of the scroll view: the query being typed, if any.
pub struct ScrollView {
    editor: ComponentHandle,
    chat: Rc<RefCell<Container>>,
    /// The full-screen body that holds the chat, when the screen has one.
    body: Option<Rc<RefCell<Container>>>,
    is_user_message: IsUserMessage,
    /// The prompt holds no text: the plain arrows step the pinned view.
    editor_empty: EditorEmpty,
    /// An agent run is going: escape goes to the prompt as well, to abort it.
    agent_busy: AgentBusy,
    typing: Option<String>,
    /// U21 single view (P2): the focused tool block, once the mode sets it.
    focus: Option<Rc<RefCell<TranscriptFocus>>>,
    /// U21 single view (P2): transcript keys queued for the mode to run.
    intents: Rc<RefCell<Vec<FocusIntent>>>,
}

fn matches(data: &str, id: &str) -> bool {
    get_keybindings().matches(data, id)
}

/// The scroll keys every surface shares, in the order they are tried.
const SCROLL_STEPS: [(&str, ScrollStep); 4] = [
    ("app.scroll.top", ScrollStep::Top),
    ("app.scroll.bottom", ScrollStep::Bottom),
    ("app.scroll.pageUp", ScrollStep::PageUp),
    ("app.scroll.pageDown", ScrollStep::PageDown),
];

#[derive(Clone, Copy)]
enum ScrollStep {
    Top,
    Bottom,
    PageUp,
    PageDown,
}

/// Runs the shared scroll key `data` matches; `false` when it matches none.
fn scroll_step(ui: &mut Tui, data: &str) -> bool {
    let Some((_, step)) = SCROLL_STEPS.iter().find(|(id, _)| matches(data, id)) else {
        return false;
    };
    match step {
        ScrollStep::Top => {
            ui.scroll_to_top();
        }
        ScrollStep::Bottom => {
            ui.scroll_to_live();
        }
        ScrollStep::PageUp => {
            ui.scroll_by_pages(-1);
        }
        ScrollStep::PageDown => {
            ui.scroll_by_pages(1);
        }
    }
    true
}

impl ScrollView {
    fn editor_focused(&self, ui: &Tui) -> bool {
        ui.focused().is_some_and(|f| Rc::ptr_eq(&f, &self.editor))
    }

    fn open_search(&mut self, ui: &mut Tui) {
        self.typing = Some(String::new());
        ui.set_scroll_search("", true);
    }

    /// The pinned-view transcript focus the mode owns; `None` until set.
    /// While unset the focus keys keep their old behaviour (they unpin).
    pub fn set_focus_state(&mut self, focus: Rc<RefCell<TranscriptFocus>>) {
        self.focus = Some(focus);
    }

    /// The transcript keys queued since the last drain.
    pub fn take_focus_intents(&mut self) -> Vec<FocusIntent> {
        std::mem::take(&mut *self.intents.borrow_mut())
    }

    /// Whether the mode has a block focused.
    fn focus_is_set(&self) -> bool {
        self.focus.as_ref().is_some_and(|f| f.borrow().is_focused())
    }

    /// Drop the focus through the shared state (esc, q, End).
    fn clear_focus(&mut self) {
        if let Some(focus) = &self.focus {
            focus.borrow_mut().clear();
        }
    }

    /// Queue one transcript key for the mode to run after the pass.
    fn queue_focus(&mut self, intent: FocusIntent) {
        self.intents.borrow_mut().push(intent);
    }

    /// `app.scroll.search` from the prompt, as the leader runs it: pin at the
    /// bottom, so the search works back from the newest thing, and open the
    /// search. The same steps the live Ctrl+R branch takes.
    pub fn search_from_bottom(&mut self, ui: &mut Tui) {
        ui.scroll_to_row(i64::MAX, None);
        if ui.scroll_pinned() {
            self.open_search(ui);
        }
    }

    fn jump(&self, ui: &mut Tui, direction: TurnDirection) {
        jump_to_user_message(
            ui,
            &self.chat,
            self.body.as_ref(),
            direction,
            &*self.is_user_message,
        );
    }

    /// A picker has focus: it keeps every key it answers, and only the paging
    /// keys move the transcript. Never un-pins, so an open question stays
    /// readable while the reader is up in the history.
    fn handle_picker(&mut self, ui: &mut Tui, data: &str) -> bool {
        // Letters are the picker's (K-P4): typed text goes to the picker,
        // while Home/End and the paging keys move the transcript.
        if is_key_release(data) || is_typed_text(data) {
            return false;
        }
        if !scroll_step(ui, data) {
            return false;
        }
        // The query line belongs to the prompt; it goes with the focus.
        self.typing = None;
        ui.clear_scroll_search();
        true
    }

    /// The prompt's scroll keys (hoocode's editor actions), while live.
    fn handle_live(&mut self, ui: &mut Tui, data: &str) -> bool {
        if !self.editor_focused(ui) {
            return self.handle_picker(ui, data);
        }
        if is_typed_text(data) {
            return false;
        }
        if scroll_step(ui, data) {
            return true;
        }
        if matches(data, "app.scroll.previousMessage") {
            self.jump(ui, TurnDirection::Previous);
        } else if matches(data, "app.scroll.nextMessage") {
            self.jump(ui, TurnDirection::Next);
        } else if matches(data, "app.scroll.search") {
            self.search_from_bottom(ui);
        } else {
            return false;
        }
        true
    }

    /// One input; `true` when it was consumed.
    pub fn handle_input(&mut self, ui: &mut Tui, data: &str) -> bool {
        if !ui.scroll_pinned() {
            // The view can un-pin without passing through here (a wheel notch
            // reaching the bottom): a stale query must not swallow keys.
            self.typing = None;
            // A typed key at the live prompt types, and a focus left over from
            // a pinned view does not outlive it.
            if is_typed_text(data) && self.editor_focused(ui) {
                self.clear_focus();
            }
            return self.handle_live(ui, data);
        }
        if !self.editor_focused(ui) {
            return self.handle_picker(ui, data);
        }
        // A key coming back up is not a decision to stop reading.
        if is_key_release(data) {
            return true;
        }
        if self.typing.is_some() && !ui.scroll_search_active() {
            self.typing = None;
        }
        // The query line owns every printable character while it is open.
        if let Some(mut query) = self.typing.take() {
            if matches(data, "tui.select.confirm") {
                // An empty query committed is a cancelled one.
                if ui.scroll_search_query().is_empty() {
                    ui.clear_scroll_search();
                } else {
                    ui.commit_scroll_search();
                }
                return true;
            }
            if matches(data, "app.scroll.exit") {
                ui.clear_scroll_search();
                return true;
            }
            if matches(data, "tui.editor.deleteCharBackward") {
                query.pop();
                ui.set_scroll_search(&query, true);
                self.typing = Some(query);
                return true;
            }
            if is_typed_text(data) {
                query.push_str(data);
                ui.set_scroll_search(&query, true);
                self.typing = Some(query);
                return true;
            }
            // A chord while composing: commit, then the chord as usual.
            ui.commit_scroll_search();
        }

        let typed = is_typed_text(data);
        let empty = (self.editor_empty)();
        let focused = self.focus_is_set();
        let searching = ui.scroll_search_active();
        // While a search is open, a bare key is the search's over an empty
        // prompt (K-P4). Otherwise `/` opens a search only from a focused block
        // over an empty prompt: at the prompt a typed `/` is a slash command.
        let bare_ok = !typed || empty;
        let search_bare_ok = if searching {
            bare_ok
        } else {
            !typed || (focused && empty)
        };
        if matches(data, "app.scroll.search")
            || (search_bare_ok && matches(data, "app.scroll.searchInView"))
        {
            self.open_search(ui);
            return true;
        }
        if searching {
            // Locked: enter steps to the next contact (M8e3).
            if matches(data, "tui.select.confirm") || matches(data, "app.scroll.searchNext") {
                ui.scroll_search_step(-1);
                return true;
            }
            if matches(data, "app.scroll.searchPrevious") {
                ui.scroll_search_step(1);
                return true;
            }
            // One escape drops the search, the next leaves the view. `q` drops it
            // only over an empty prompt, as the other bare letters do.
            if matches(data, "app.scroll.exit") && (bare_ok || !is_typed_text(data)) {
                ui.clear_scroll_search();
                return true;
            }
        }

        // K-P4 (2026-11-11): at the prompt a bare printable key always types.
        // Only a focused block over an empty prompt lets one act as a
        // transcript key. Any other typed key clears the focus, goes back to
        // live and types. A search that is open keeps its own rule below.
        if !searching && typed && (!focused || !empty) {
            self.clear_focus();
            ui.scroll_to_live();
            return false;
        }

        // U21 single view (P2): the transcript focus keys, with a block focused
        // over an empty prompt and outside a search. They queue for the mode
        // (which owns the chains) instead of unpinning. Enter and y keep their
        // old behaviour below when no block is focused.
        if !searching && focused && empty {
            if matches(data, "app.scroll.focusPrevious") {
                self.queue_focus(FocusIntent::Previous);
                return true;
            }
            if matches(data, "app.scroll.focusNext") {
                self.queue_focus(FocusIntent::Next);
                return true;
            }
            if matches(data, "app.scroll.expandFocused") {
                self.queue_focus(FocusIntent::Toggle);
                return true;
            }
            if matches(data, "app.scroll.copyFocused") {
                self.queue_focus(FocusIntent::Copy);
                return true;
            }
            // Esc clears the focus only and stays pinned: it never stops the
            // turn. `q` (the exit's other key) is handled below.
            if data == "\x1b" && matches(data, "app.scroll.exit") {
                self.clear_focus();
                return true;
            }
        }

        // Plain arrows are the prompt's while it holds text: they keep the
        // cursor and the history, and the pin stays where it is. (`j` and `k`
        // are typed characters, so they are not arrows here.)
        let plain_line = (matches(data, "app.scroll.lineUp")
            || matches(data, "app.scroll.lineDown"))
            && !is_typed_text(data);
        if plain_line && !(self.editor_empty)() {
            return false;
        }
        // Only reached while a search is open: a typed key over prompt text
        // goes back to live and types, as it always has.
        if typed && !empty {
            ui.scroll_to_live();
            return false;
        }
        // Exit before the ends, so bindings sharing a key still get out. `q`
        // with a block focused clears the focus and goes live.
        if matches(data, "app.scroll.exit") {
            self.clear_focus();
            ui.scroll_to_live();
            // A running agent takes the escape as its abort, as it does from the
            // tail. `q` is only ever a pager key here, so it never reaches the prompt.
            return !(self.agent_busy)() || typed;
        }
        if matches(data, "app.scroll.bottom") {
            self.clear_focus();
        }
        if scroll_step(ui, data) {
            return true;
        }
        if matches(data, "app.scroll.lineUp") || matches(data, "app.scroll.shiftLineUp") {
            ui.scroll_by_lines(-1);
        } else if matches(data, "app.scroll.lineDown") || matches(data, "app.scroll.shiftLineDown")
        {
            ui.scroll_by_lines(1);
        } else if matches(data, "app.scroll.previousMessage") {
            self.jump(ui, TurnDirection::Previous);
        } else if matches(data, "app.scroll.nextMessage") {
            self.jump(ui, TurnDirection::Next);
        } else {
            // Anything else: back to live, and the key does its usual job. A
            // typed key also drops the focus, so it cannot outlive the typing.
            ui.scroll_to_live();
            if typed && !searching {
                self.clear_focus();
            }
            return false;
        }
        true
    }
}

/// Where the pinned top row sits in the transcript: the chat child that holds
/// it, and how many rows into that child. Taken before a change that moves rows
/// in place (the tool output view), and put back after it with
/// [`reanchor_top`], so the same message stays on top. A row above the chat
/// (the header) has no child and keeps its distance from the chat's start.
#[derive(Clone)]
pub struct TopAnchor {
    child: Option<ComponentHandle>,
    rows_in: i64,
    /// The block of that child the top row sits in (its header at or above
    /// the row), and how far under the header. A child's rows move when a
    /// block above the top row opens or closes inside it (a chain holds
    /// many tool rows), so the row is found again from its own block.
    fold: Option<(String, i64)>,
}

/// The block of `child` whose header sits at or above row `rows_in`, and
/// the distance from that header.
fn fold_above(child: &ComponentHandle, rows_in: i64) -> Option<(String, i64)> {
    let component = child.try_borrow().ok()?;
    let folds = component.fold_host()?.folds();
    folds
        .iter()
        .filter_map(|fold| Some((fold, *fold.header.first()? as i64)))
        .filter(|(_, header)| *header <= rows_in)
        .max_by_key(|(_, header)| *header)
        .map(|(fold, header)| (fold.id.clone(), rows_in - header))
}

/// Where the anchored row sits in `child` now: under its block's header,
/// no further down than the block reaches. `None` when the block is gone.
fn fold_rows_in(child: &ComponentHandle, id: &str, under: i64) -> Option<i64> {
    let component = child.try_borrow().ok()?;
    let folds = component.fold_host()?.folds();
    let at = folds.iter().position(|fold| fold.id == id)?;
    let header = *folds[at].header.first()? as i64;
    // The rows of a closed block are gone: stay on its header.
    let reach = match folds.get(at + 1).and_then(|next| next.header.first()) {
        Some(next) => (*next as i64 - header - 1).max(0),
        None => i64::MAX,
    };
    let under = under.min(reach);
    Some(header + under)
}

/// The anchor of the pinned top row, from the last frame's layout. `None` when
/// the view is live or the row cannot be placed in the chat.
pub fn capture_top(
    ui: &Tui,
    chat: &Rc<RefCell<Container>>,
    body: Option<&Rc<RefCell<Container>>>,
) -> Option<TopAnchor> {
    let (top, ..) = ui.get_scroll_position()?;
    let body = body?;
    let width = ui.transcript_width();
    let chat_handle: ComponentHandle = chat.clone();
    let body = body.borrow();
    let index = body
        .children()
        .iter()
        .position(|c| Rc::ptr_eq(c, &chat_handle))?;
    let chat_base = body.child_row_offsets(width)?[index] as i64;
    let rel = top - chat_base;
    let chat = chat.borrow();
    if rel < 0 {
        return Some(TopAnchor {
            child: None,
            rows_in: rel,
            fold: None,
        });
    }
    let offsets = chat.child_row_offsets(width)?;
    let i = offsets.iter().rposition(|o| (*o as i64) <= rel)?;
    let child = chat.children()[i].clone();
    let rows_in = rel - offsets[i] as i64;
    Some(TopAnchor {
        fold: fold_above(&child, rows_in),
        child: Some(child),
        rows_in,
    })
}

/// After the transcript's rows moved, puts the pinned top back on the same
/// message: the body is laid out again so its offsets are current, then the
/// view moves to the anchored child's new start (clamped inside that child).
/// The view goes live when there is no anchor, no body (not full-screen), or
/// the anchored child is gone. A live view stays live.
pub fn reanchor_top(
    ui: &mut Tui,
    chat: &Rc<RefCell<Container>>,
    body: Option<&Rc<RefCell<Container>>>,
    anchor: Option<TopAnchor>,
) {
    if !ui.scroll_pinned() {
        return;
    }
    ui.reset_new_lines_count();
    let placed = match (anchor, body) {
        (Some(anchor), Some(body)) => place_top(ui, chat, body, &anchor),
        _ => None,
    };
    match placed {
        Some(top) => ui.restore_scroll_top(Some(top)),
        None => {
            ui.scroll_to_live();
        }
    }
}

/// The new top row for `anchor`, from a fresh layout of the body.
fn place_top(
    ui: &Tui,
    chat: &Rc<RefCell<Container>>,
    body: &Rc<RefCell<Container>>,
    anchor: &TopAnchor,
) -> Option<i64> {
    let width = ui.transcript_width();
    body.borrow_mut().render(width);
    let chat_handle: ComponentHandle = chat.clone();
    let body = body.borrow();
    let index = body
        .children()
        .iter()
        .position(|c| Rc::ptr_eq(c, &chat_handle))?;
    let chat_base = body.child_row_offsets(width)?[index] as i64;
    let chat = chat.borrow();
    let rel = match &anchor.child {
        None => anchor.rows_in,
        Some(child) => {
            let i = chat.children().iter().position(|c| Rc::ptr_eq(c, child))?;
            let offsets = chat.child_row_offsets(width)?;
            let start = offsets[i] as i64;
            let rows_in = anchor
                .fold
                .as_ref()
                .and_then(|(id, under)| fold_rows_in(child, id, *under))
                .unwrap_or(anchor.rows_in);
            // Stay inside the message: a fold can make it shorter than it was.
            let within = match offsets.get(i + 1) {
                Some(next) => rows_in.min(*next as i64 - start - 1),
                None => rows_in,
            };
            start + within.max(0)
        }
    };
    Some((chat_base + rel).max(0))
}

/// Draws the pinned view's status as the status line: `status_line` holds the
/// session facts the mode keeps current. `focus` is the pinned-view
/// transcript focus: while a block is focused the slot names its keys.
pub fn install_scroll_status(
    ui: &mut Tui,
    status_line: Rc<RefCell<StatusLineData>>,
    focus: Rc<RefCell<TranscriptFocus>>,
) {
    ui.set_scroll_status_formatter(Box::new(move |status: &ScrollStatus| {
        format_scroll_status(status, &status_line.borrow(), focus.borrow().is_focused())
    }));
}

/// `installScrollView`: the picker's share of the pinned view, and the keys.
/// `editor` is the prompt. Pinning works from any focus; with a picker
/// focused, the picker keeps its own keys (see `handle_picker`). The status
/// row is [`install_scroll_status`]'s.
pub fn install_scroll_view(
    ui: &mut Tui,
    editor: ComponentHandle,
    chat: Rc<RefCell<Container>>,
    body: Option<Rc<RefCell<Container>>>,
    is_user_message: IsUserMessage,
    editor_empty: EditorEmpty,
    agent_busy: AgentBusy,
) -> Rc<RefCell<ScrollView>> {
    ui.scroll_prompt = Some(editor.clone());
    let view = Rc::new(RefCell::new(ScrollView {
        editor,
        chat,
        body,
        is_user_message,
        editor_empty,
        agent_busy,
        typing: None,
        focus: None,
        intents: Rc::new(RefCell::new(Vec::new())),
    }));
    // The leader runs the search from outside the input path (`search_from_bottom`),
    // so the caller keeps a handle to the same view.
    let shared = view.clone();
    ui.set_input_interceptor(Some(Box::new(move |ui: &mut Tui, data: &str| {
        view.borrow_mut().handle_input(ui, data)
    })));
    shared
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_ai_types::Content;
    use hoocode_code_tui_widgets::tool_chain::{ToolBlock, ToolChainComponent};
    use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
    use hoocode_code_tui_widgets::tool_signal::ToolResult;
    use hoocode_code_tui_widgets::tools::builtin_tool_definition;
    use serde_json::json;

    fn shell_block(id: &str) -> ToolBlock {
        let definition = builtin_tool_definition("Shell").expect("shell renderer");
        let mut block = ToolExecutionComponent::new(
            "Shell",
            id,
            json!({"command": "echo hi"}),
            ToolExecutionOptions::default(),
            Some(definition),
            "/",
        );
        block.update_result(
            ToolResult {
                content: vec![Content::text("line one\nline two\nline three")],
                details: json!({}),
                is_error: false,
            },
            false,
        );
        Rc::new(RefCell::new(block))
    }

    /// Two stacked chains: a header row of the second chain's second block
    /// hits that block, while an expanded body row hits nothing. Rows are
    /// chat-relative, the same mapping the mode's click drain uses.
    #[test]
    fn fold_at_places_header_and_body_rows() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let width: u16 = 80;
        let chain0 = Rc::new(RefCell::new(ToolChainComponent::new()));
        chain0.borrow_mut().add(shell_block("chain-0"));
        // Two blocks so block 1 has a header; the first is expanded so the
        // chain has body rows to miss on.
        let chain1 = Rc::new(RefCell::new(ToolChainComponent::new()));
        chain1.borrow_mut().add(shell_block("chain-1-a"));
        chain1.borrow_mut().add(shell_block("chain-1-b"));
        chain1.borrow_mut().toggle_block(0);
        let chain0_handle: ComponentHandle = chain0.clone();
        let chain1_handle: ComponentHandle = chain1.clone();
        let chat = Rc::new(RefCell::new(Container::new()));
        chat.borrow_mut().add_child(chain0_handle);
        chat.borrow_mut().add_child(chain1_handle.clone());
        let body = Rc::new(RefCell::new(Container::new()));
        let chat_handle: ComponentHandle = chat.clone();
        body.borrow_mut().add_child(chat_handle);
        body.borrow_mut().render(width);

        let chat_offsets = chat
            .borrow()
            .child_row_offsets(width)
            .expect("chat laid out");
        assert_eq!(chat_offsets.len(), 2);
        let header = chain1
            .borrow()
            .header_start_row(1)
            .expect("second block header");
        let header_rel = chat_offsets[1] + header;
        let (found, hit) = fold_at(&chat, width, header_rel).expect("header row hits");
        assert!(Rc::ptr_eq(&found.host, &chain1_handle));
        assert_eq!(found.fold.id, "chain-1-b");
        assert_eq!(hit, FoldHit::Header);
        assert_eq!(found.header_row(), Some(header_rel));

        // The row just above the second header is the expanded first
        // block's body (no blank opens a run that shows a body), so it
        // hits nothing.
        assert!(header > 0, "second header leaves a body row above it");
        assert_eq!(chain1.borrow().header_hit(header - 1), None);
        assert!(fold_at(&chat, width, header_rel - 1).is_none());

        // The one list every path reads holds all three blocks, in order.
        let ids: Vec<String> = chat_folds(&chat, width)
            .into_iter()
            .map(|found| found.fold.id)
            .collect();
        assert_eq!(ids, ["chain-0", "chain-1-a", "chain-1-b"]);
    }
}
