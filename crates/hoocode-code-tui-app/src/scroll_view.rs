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

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tui_keybindings::key_text;
use hoocode_tui_keys::{get_keybindings, is_key_release};
use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::{Line, Span};
use hoocode_tui_ratatui::{reversed, Palette, RatatuiComponent};
use hoocode_tui_render::{Component, ComponentHandle, Container, ScrollStatus, Tui};
use hoocode_tui_util::{truncate_to_width, visible_width};

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

/// Plain typed text rather than a chord (control bytes and escapes are keys).
fn is_typed_text(data: &str) -> bool {
    !data.is_empty() && data.chars().all(|c| (c as u32) >= 32 && c as u32 != 127)
}

/// `1 contact`, `N contacts`.
fn contacts(count: usize) -> String {
    if count == 1 {
        "1 contact".to_string()
    } else {
        format!("{count} contacts")
    }
}

/// One run of the Mission Control bar: plain text plus the palette token
/// styling it. `""` is the default style.
struct BarRun {
    text: String,
    token: &'static str,
}

fn bar_run(text: String, token: &'static str) -> BarRun {
    BarRun { text, token }
}

/// The bar's left part as runs: the leading space, the label in its
/// token colour, and the rest default. `left` is the plain left part (one
/// leading space, then the label).
fn left_runs(left: String, label: &str, token: &'static str) -> Vec<BarRun> {
    match left
        .strip_prefix(' ')
        .and_then(|rest| rest.strip_prefix(label))
    {
        Some(rest) => vec![
            bar_run(" ".to_string(), ""),
            bar_run(label.to_string(), token),
            bar_run(rest.to_string(), ""),
        ],
        None => vec![bar_run(left, "")],
    }
}

/// The bar's runs as ratatui spans: every run reversed (the bar is one
/// inverse row), the label run also in its token colour. Runs are cut to
/// `width` columns — the run holding the cut is truncated — and blanks pad
/// the row to its full width.
fn bar_spans(runs: &[BarRun], width: usize, pal: &Palette) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut used = 0;
    for run in runs {
        if used >= width {
            break;
        }
        let room = width - used;
        let run_width = visible_width(&run.text);
        if run_width <= room {
            spans.push(Span::styled(
                run.text.clone(),
                reversed(pal.style(run.token)),
            ));
            used += run_width;
        } else {
            let piece = truncate_to_width(&run.text, room, "", true);
            used += visible_width(&piece);
            spans.push(Span::styled(piece, reversed(pal.style(run.token))));
        }
    }
    if used < width {
        spans.push(Span::styled(
            " ".repeat(width - used),
            reversed(Style::default()),
        ));
    }
    spans
}

/// One inverse bar row `width` columns wide, drawn with ratatui
/// (`RatatuiComponent`): the label run in its token colour, the rest
/// default, every run reversed. Exactly `width` columns when `width > 0`.
fn paint_bar(runs: Vec<BarRun>, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let render_width = width.min(u16::MAX as usize) as u16;
    // One palette per render pass, not per draw: the closure may draw again
    // after an invalidate.
    let pal = Palette::current();
    let mut bar = RatatuiComponent::new(1, move |area: Rect, buf: &mut Buffer| {
        let spans = bar_spans(&runs, usize::from(area.width), &pal);
        buf.set_line(area.x, area.y, &Line::from(spans), area.width);
    });
    bar.render(render_width)
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// The SCAN bar's keys from the bindings: `{confirm} lock|next` and
/// `{exit} clear`, plus the `n/p` step pair while locked when bound.
/// Unbound names are left out; the bar never shows a bare verb.
fn scan_keys(typing: bool) -> String {
    let verb = if typing { "lock" } else { "next" };
    let mut keys = Vec::new();
    let confirm = key_text("tui.select.confirm");
    keys.push(if confirm.is_empty() {
        verb.to_string()
    } else {
        format!("{confirm} {verb}")
    });
    let exit = key_text("app.scroll.exit");
    if !exit.is_empty() {
        keys.push(format!("{exit} clear"));
    }
    if !typing {
        let mut step = Vec::new();
        for id in ["app.scroll.searchNext", "app.scroll.searchPrevious"] {
            let name = key_text(id);
            if !name.is_empty() {
                step.push(name);
            }
        }
        if !step.is_empty() {
            keys.push(step.join("/"));
        }
    }
    keys.join(" · ")
}

/// `formatStatus`: the Mission Control bar above the comms frame while the
/// view is pinned: the REWIND bar (`◂◂ REWIND  43–61 of 78 · 55%  ↓ 3 new`,
/// label in vio), or the SCAN bar (`⌕ SCAN  query  N contact(s)`, label in
/// warn) while searching. `editor_empty` says whether the plain arrows step
/// the view (the prompt holds no text); otherwise the line keys are the
/// shifted arrows.
pub fn format_scroll_status(status: &ScrollStatus, editor_empty: bool) -> String {
    let width = status.width.max(0) as usize;
    if let Some(search) = &status.search {
        // Mission Control SCAN bar (M8e3): the query, its contacts, and the
        // scan keys. While typing, enter locks the query; once locked, enter
        // steps to the next contact.
        let caret = if search.typing { "\u{2588}" } else { "" };
        let contacts = if search.query.is_empty() {
            String::new()
        } else if search.count == 0 {
            "  no contacts".to_string()
        } else if search.typing || search.count == 1 {
            format!("  {}", contacts(search.count))
        } else {
            format!("  {}/{} contacts", search.index, search.count)
        };
        let left = format!(" ⌕ SCAN  {}{caret}{contacts}", search.query);
        let keys = scan_keys(search.typing);
        let gap = status.width - visible_width(&left) as i64 - visible_width(&keys) as i64 - 2;
        let mut runs = left_runs(left, "⌕ SCAN", "warn");
        if gap >= 0 {
            runs.push(bar_run(" ".repeat(gap as usize + 1), ""));
            runs.push(bar_run(keys, ""));
            runs.push(bar_run(" ".to_string(), ""));
        } else {
            runs.push(bar_run(" ".to_string(), ""));
        }
        return paint_bar(runs, width);
    }

    let position = format!(
        "{}\u{2013}{} of {}",
        status.top, status.bottom, status.total
    );
    let place = if status.at_top {
        "start of session".to_string()
    } else {
        // JS Math.round on a non-negative ratio.
        let percent = (status.top as f64 / status.total.max(1) as f64 * 100.0 + 0.5).floor();
        format!("{percent}%")
    };
    // Output that arrived while the view was pinned, to be read when back at the tail.
    let new = if status.new_lines > 0 {
        format!("  \u{2193} {} new", status.new_lines)
    } else {
        String::new()
    };
    let left = format!(" ◂◂ REWIND  {position}  {place}{new}");
    // With a picker focused the arrows and escape are its keys: only paging
    // is the transcript's to advertise.
    let keys = if status.picker {
        "PgUp/PgDn page".to_string()
    } else {
        let (up, down) = if editor_empty {
            ("app.scroll.lineUp", "app.scroll.lineDown")
        } else {
            ("app.scroll.shiftLineUp", "app.scroll.shiftLineDown")
        };
        [
            format!("{}/{} line", key_text(up), key_text(down)),
            "PgUp/PgDn page".to_string(),
            format!("{} top", key_text("app.scroll.top")),
            format!("{} live", key_text("app.scroll.exit")),
        ]
        .join(" · ")
    };
    // The keys only when they fit whole: a truncated chord reads as another.
    let gap = status.width - visible_width(&left) as i64 - visible_width(&keys) as i64 - 3;
    let mut runs = vec![bar_run(" ".to_string(), "")];
    runs.extend(left_runs(left, "◂◂ REWIND", "vio"));
    if gap >= 0 {
        runs.push(bar_run(" ".repeat(gap as usize + 1), ""));
        runs.push(bar_run(keys, ""));
        runs.push(bar_run(" ".to_string(), ""));
    } else {
        runs.push(bar_run(" ".to_string(), ""));
    }
    paint_bar(runs, width)
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
        if is_key_release(data) {
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
            // Pin first, at the bottom, so the search works back from the
            // newest thing.
            ui.scroll_to_row(i64::MAX, None);
            if ui.scroll_pinned() {
                self.open_search(ui);
            }
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

        if matches(data, "app.scroll.search") || matches(data, "app.scroll.searchInView") {
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
            // One escape drops the search, the next leaves the view.
            if matches(data, "app.scroll.exit") {
                ui.clear_scroll_search();
                return true;
            }
        }

        // Plain arrows are the prompt's while it holds text: they keep the
        // cursor and the history, and the pin stays where it is.
        let plain_line = matches(data, "app.scroll.lineUp") || matches(data, "app.scroll.lineDown");
        if plain_line && !(self.editor_empty)() {
            return false;
        }
        // Exit before the ends, so bindings sharing a key still get out.
        if matches(data, "app.scroll.exit") {
            ui.scroll_to_live();
            // A running agent takes the escape as its abort, as it does from the tail.
            return !(self.agent_busy)();
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

/// `installScrollView`: the indicator, the picker's share of the pinned
/// view, and the keys. `editor` is the prompt. Pinning works from any focus;
/// with a picker focused, the picker keeps its own keys (see `handle_picker`).
pub fn install_scroll_view(
    ui: &mut Tui,
    editor: ComponentHandle,
    chat: Rc<RefCell<Container>>,
    body: Option<Rc<RefCell<Container>>>,
    is_user_message: IsUserMessage,
    editor_empty: EditorEmpty,
    agent_busy: AgentBusy,
) {
    let bar_editor_empty = editor_empty.clone();
    ui.set_scroll_status_formatter(Box::new(move |status: &ScrollStatus| {
        format_scroll_status(status, bar_editor_empty())
    }));
    ui.scroll_prompt = Some(editor.clone());
    let mut view = ScrollView {
        editor,
        chat,
        body,
        is_user_message,
        editor_empty,
        agent_busy,
        typing: None,
    };
    ui.set_input_interceptor(Some(Box::new(move |ui: &mut Tui, data: &str| {
        view.handle_input(ui, data)
    })));
}
