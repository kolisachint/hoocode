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
use std::collections::HashMap;
use std::rc::Rc;

use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_tui_keys::{get_keybindings, is_key_release};
use hoocode_tui_render::{Component, ComponentHandle, Container, ScrollStatus, Tui};

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

/// A header click waiting for the mode to act: the chain and block the
/// header toggles, and where the header sat when clicked. The mode drains
/// these after the TUI takes the event, while the layout still matches the
/// painted frame the click mapped to.
pub struct BlockToggle {
    pub chain: ComponentHandle,
    pub block: usize,
    pub buffer_row: i64,
    pub context: i64,
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

/// Hit-test a transcript buffer row against the chat: the chain and block
/// whose header holds it. Body rows and rows outside every chain are `None`
/// (a click there does nothing).
pub fn hit_chain(
    chat: &Rc<RefCell<Container>>,
    width: u16,
    rel: usize,
) -> Option<(ComponentHandle, usize)> {
    let chat_ref = chat.borrow();
    let offsets = chat_ref.child_row_offsets(width)?;
    let children = chat_ref.children();
    if children.len() != offsets.len() {
        return None;
    }
    let i = offsets.iter().rposition(|o| *o <= rel)?;
    if offsets.get(i + 1).is_some_and(|next| rel >= *next) {
        return None;
    }
    let chain = children[i].clone();
    let row = rel - offsets[i];
    let block = chain
        .borrow()
        .as_any()?
        .downcast_ref::<ToolChainComponent>()?
        .header_hit(row)?;
    Some((chain, block))
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

/// U21 single view (P2): which tool block the pinned view has focused, if
/// any. The id is a tool call id: blocks come and go (a retry takes chains
/// out), so a missing id resolves to no focus. The focused toggle keeps its
/// `expanded` overrides here by call id; the tool output dial wipes them
/// when it moves every block at once.
#[derive(Debug, Default)]
pub struct TranscriptFocus {
    focused_call: Option<String>,
    expanded: HashMap<String, bool>,
}

impl TranscriptFocus {
    /// No focus, no overrides.
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

    /// Drop the focus, keeping the toggle overrides.
    pub fn clear(&mut self) {
        self.focused_call = None;
    }

    /// Remember the focused toggle's stop for one block.
    pub fn set_expanded(&mut self, call_id: &str, expanded: bool) {
        self.expanded.insert(call_id.to_string(), expanded);
    }

    /// The focused toggle's stop for one block, if it moved it.
    pub fn expanded_override(&self, call_id: &str) -> Option<bool> {
        self.expanded.get(call_id).copied()
    }

    /// Forget every focused toggle stop (the dial moved every block).
    pub fn clear_expanded(&mut self) {
        self.expanded.clear();
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

/// The state file's key for the focus-keys notice (one per version).
const FOCUS_NOTICE_KEY: &str = "transcriptFocusNoticeVersion";

/// The one-time focus-keys notice for `version`, shown from `state` (the
/// keybindings state file). `None` once this version has seen it. The file
/// keeps its other keys: the write merges the notice's key in.
pub fn take_focus_notice(state: &std::path::Path, version: &str) -> Option<String> {
    let text = std::fs::read_to_string(state).unwrap_or_default();
    let mut record: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&text).unwrap_or_default();
    if record.get(FOCUS_NOTICE_KEY).and_then(|v| v.as_str()) == Some(version) {
        return None;
    }
    record.insert(
        FOCUS_NOTICE_KEY.to_string(),
        serde_json::Value::String(version.to_string()),
    );
    if let Some(dir) = state.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // A write that fails means the notice may show again next focus. That is
    // harmless, so the error is not reported.
    let _ = std::fs::write(state, serde_json::Value::Object(record).to_string());
    Some(
        "Tool blocks: { } move between calls, enter opens, y copies the raw output, esc clears, G goes live."
            .to_string(),
    )
}

/// A pinned-view transcript key the mode runs after the pass: the scroll
/// view queues it and stays dumb, so the mode keeps the chains, the session
/// and the clipboard. Esc and G clear the shared focus themselves, so they
/// ride no intent.
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
/// while the query is typed), `esc live` for the scrolled view, and with a
/// block focused `enter open · y copy · esc clear`. With a
/// picker focused the arrows and escape are its keys, so none are named.
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
        hints(&[
            ("app.scroll.expandFocused", "open"),
            ("app.scroll.copyFocused", "copy"),
            ("app.scroll.exit", "clear"),
        ])
    } else {
        hints(&[("app.scroll.exit", "live")])
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

    /// Drop the focus through the shared state (esc, G).
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
        // Letters are the picker's (K-P4): `g` there is a picker key, not the top.
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

        // `/` opens the search from the pinned view over an empty prompt (K-P4).
        let bare_ok = !is_typed_text(data) || (self.editor_empty)();
        if matches(data, "app.scroll.search")
            || (bare_ok && matches(data, "app.scroll.searchInView"))
        {
            self.open_search(ui);
            return true;
        }
        if ui.scroll_search_active() {
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

        // U21 single view (P2): the transcript focus keys, over an empty prompt
        // and outside a search. They queue for the mode (which owns the chains)
        // instead of unpinning; esc and G clear the shared focus themselves.
        // With no focus set, enter and y keep their old behaviour below.
        if !ui.scroll_search_active() && (self.editor_empty)() {
            if matches(data, "app.scroll.bottom") && self.focus_is_set() {
                self.clear_focus();
            }
            if matches(data, "app.scroll.focusPrevious") {
                self.queue_focus(FocusIntent::Previous);
                return true;
            }
            if matches(data, "app.scroll.focusNext") {
                self.queue_focus(FocusIntent::Next);
                return true;
            }
            if self.focus_is_set() {
                if matches(data, "app.scroll.expandFocused") {
                    self.queue_focus(FocusIntent::Toggle);
                    return true;
                }
                if matches(data, "app.scroll.copyFocused") {
                    self.queue_focus(FocusIntent::Copy);
                    return true;
                }
                // Esc clears the focus only and stays pinned: it never stops
                // the turn. `q` (the exit's other key) keeps leaving the view.
                if data == "\x1b" && matches(data, "app.scroll.exit") {
                    self.clear_focus();
                    return true;
                }
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
        // A bare letter (j, k, g, G, [, ], q) steps the view only over an empty
        // prompt (K-P4). Otherwise it goes back to live and types.
        if is_typed_text(data) && !(self.editor_empty)() {
            ui.scroll_to_live();
            return false;
        }
        // Exit before the ends, so bindings sharing a key still get out.
        if matches(data, "app.scroll.exit") {
            ui.scroll_to_live();
            // A running agent takes the escape as its abort, as it does from the
            // tail. `q` is only ever a pager key, so it never reaches the prompt.
            return !(self.agent_busy)() || is_typed_text(data);
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
            // Anything else: back to live, and the key does its usual job.
            ui.scroll_to_live();
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
        });
    }
    let offsets = chat.child_row_offsets(width)?;
    let i = offsets.iter().rposition(|o| (*o as i64) <= rel)?;
    Some(TopAnchor {
        child: Some(chat.children()[i].clone()),
        rows_in: rel - offsets[i] as i64,
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
            // Stay inside the message: a fold can make it shorter than it was.
            let within = match offsets.get(i + 1) {
                Some(next) => anchor.rows_in.min(*next as i64 - start - 1),
                None => anchor.rows_in,
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
    /// hits `(chain, 1)`, while an expanded body row hits nothing. Rows are
    /// chat-relative, the same mapping the mode's click drain uses.
    #[test]
    fn hit_chain_places_header_and_body_rows() {
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
        let hit = hit_chain(&chat, width, header_rel).expect("header row hits");
        assert!(Rc::ptr_eq(&hit.0, &chain1_handle));
        assert_eq!(hit.1, 1);

        // The row just above the second header is the expanded first
        // block's body (no blank opens a run that shows a body), so it
        // hits nothing.
        assert!(header > 0, "second header leaves a body row above it");
        assert_eq!(chain1.borrow().header_hit(header - 1), None);
        assert!(hit_chain(&chat, width, header_rel - 1).is_none());
    }
}
