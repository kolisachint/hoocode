//! `components/user-message-selector.ts`: pick a user message to fork from.
//!
//! The STAGE SEPARATION console (M7, ratatui): `STAGE SEPARATION · /fork`,
//! the explanation on top, one row per user message, and the hint foot
//! `↑↓ move the line · enter fork · esc close`. Shared `Console` chrome in
//! `code-tui-widgets/src/console.rs`; keys and events are unchanged.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hoocode_code_tui_widgets::console::{hints, Console};
use hoocode_tui_components::PickerCursor;
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{bold, Line, Palette, Span};
use hoocode_tui_render::Component;
use hoocode_tui_util::js_regex::is_js_space;
use hoocode_tui_util::{truncate_to_width, visible_width};

/// `UserMessageItem`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMessageItem {
    /// Entry id in the session.
    pub id: String,
    pub text: String,
    pub timestamp: Option<String>,
}

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserMessageEvent {
    Select(String),
    Cancel,
}

/// `UserMessageList`: two rows per message, oldest first.
pub struct UserMessageList {
    messages: Vec<UserMessageItem>,
    cursor: PickerCursor,
    events: Vec<UserMessageEvent>,
}

impl UserMessageList {
    fn new(messages: Vec<UserMessageItem>, initial_selected_id: Option<&str>) -> Self {
        let initial = initial_selected_id.and_then(|id| messages.iter().position(|m| m.id == id));
        // The given message, else the most recent.
        let selected_index = initial.unwrap_or(messages.len().saturating_sub(1));
        let mut cursor = PickerCursor::new(messages.len(), 10);
        cursor.select(selected_index);
        Self {
            messages,
            cursor,
            events: Vec::new(),
        }
    }

    /// What the keys since the last call did.
    pub fn take_events(&mut self) -> Vec<UserMessageEvent> {
        std::mem::take(&mut self.events)
    }
}

/// One message row (content only; the console adds the `›` marker or
/// gutter and the selection band): the text with newlines flattened and
/// JS-trimmed, cut to the inner width. The selected row reads bold accent
/// and carries the separation load: how many messages launch and their
/// fuel estimate (M8d8).
fn message_row(
    text: &str,
    carries: Option<(usize, usize)>,
    selected: bool,
    width: usize,
) -> Line<'static> {
    let pal = Palette::current();
    let normalized = text.replace('\n', " ");
    let normalized = normalized.trim_matches(is_js_space);
    let suffix = carries.map(|(count, fuel)| {
        format!(
            " · carries {count} {} · {:.1}k fuel",
            if count == 1 { "message" } else { "messages" },
            fuel as f64 / 1000.0,
        )
    });
    let suffix_width = suffix.as_deref().map(visible_width).unwrap_or(0);
    let truncated = truncate_to_width(
        normalized,
        width.saturating_sub(2).saturating_sub(suffix_width),
        "...",
        false,
    );
    let mut text = truncated;
    if let Some(suffix) = suffix {
        text.push_str(&suffix);
    }
    if selected {
        Line::from(vec![Span::styled(text, bold(pal.acc()))])
    } else {
        Line::raw(text)
    }
}

/// Rough token estimate for the separation load: UTF-16 code units per
/// token, as the tool-schema measurer counts.
fn estimate_tokens(text: &str) -> usize {
    text.encode_utf16().count().div_ceil(4)
}

impl UserMessageList {
    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if self.cursor.handle_key(data, &kb) {
            // Navigation: the cursor clamps at both ends.
        } else if kb.matches(data, "tui.select.confirm") {
            if let Some(selected) = self.messages.get(self.cursor.selected()) {
                self.events
                    .push(UserMessageEvent::Select(selected.id.clone()));
            }
        } else if kb.matches(data, "tui.select.cancel") {
            self.events.push(UserMessageEvent::Cancel);
        }
    }
}

/// `UserMessageSelectorComponent`: the STAGE SEPARATION console.
/// With no messages it cancels itself after 100ms ([`Self::poll`]).
pub struct UserMessageSelectorComponent {
    list: Rc<RefCell<UserMessageList>>,
    auto_cancel_at: Option<Instant>,
}

impl UserMessageSelectorComponent {
    pub fn new(messages: Vec<UserMessageItem>, initial_selected_id: Option<&str>) -> Self {
        let auto_cancel_at = messages
            .is_empty()
            .then(|| Instant::now() + Duration::from_millis(100));
        let list = Rc::new(RefCell::new(UserMessageList::new(
            messages,
            initial_selected_id,
        )));
        Self {
            list,
            auto_cancel_at,
        }
    }

    /// `getMessageList()`.
    pub fn message_list(&self) -> Rc<RefCell<UserMessageList>> {
        self.list.clone()
    }

    /// What happened since the last call: keys, or the empty list's
    /// auto-cancel once its time is up.
    pub fn poll(&mut self, now: Instant) -> Vec<UserMessageEvent> {
        let mut events = self.list.borrow_mut().take_events();
        if self.auto_cancel_at.is_some_and(|at| now >= at) {
            self.auto_cancel_at = None;
            events.push(UserMessageEvent::Cancel);
        }
        events
    }

    /// When the auto-cancel is due, if pending.
    pub fn deadline(&self) -> Option<Instant> {
        self.auto_cancel_at
    }
}

impl Component for UserMessageSelectorComponent {
    /// The STAGE SEPARATION console: what launching means on top, one row
    /// per user message with the separation load on the selected row, the
    /// dotted amber rule below, and the position and hint foot. Keys and
    /// events unchanged.
    fn render(&mut self, width: u16) -> Vec<String> {
        let pal = Palette::current();
        let list = self.list.borrow();
        let width = width as usize;
        let mut console = Console::new("STAGE SEPARATION · /fork", "");
        console.top = vec![Line::from(vec![Span::styled(
            "Everything above the line launches as a new session; this one keeps flying.",
            pal.faint(),
        )])];
        let len = list.messages.len();
        if len == 0 {
            console.bottom = vec![Line::from(vec![Span::styled(
                "  No user messages found",
                pal.faint(),
            )])];
        } else {
            let window = list.cursor.window();
            let selected = list.cursor.selected();
            // The separation load: everything above the line launches, so
            // the selected row carries the messages through it and their
            // fuel estimate.
            let fuel: usize = list.messages[..=selected.min(len - 1)]
                .iter()
                .map(|m| estimate_tokens(&m.text))
                .sum();
            let carries = selected.saturating_add(1);
            console.rows = window
                .clone()
                .map(|i| {
                    message_row(
                        &list.messages[i].text,
                        (i == selected).then_some((carries, fuel)),
                        i == selected,
                        width,
                    )
                })
                .collect();
            console.selected = window.clone().position(|i| i == selected);
            let mut bottom = vec![Line::from(vec![Span::styled(
                format!("  ✂ {}", "╌".repeat(width.saturating_sub(6).max(8))),
                pal.warn(),
            )])];
            bottom.push(Line::from(vec![Span::styled(
                format!("  Message {} of {len}", selected + 1),
                pal.faint(),
            )]));
            if let Some(footer) = list.cursor.footer() {
                bottom.push(Line::from(vec![Span::styled(
                    format!("  {footer}"),
                    pal.faint(),
                )]));
            }
            console.bottom = bottom;
        }
        console.hint = hints(&[("↑↓", "move the line"), ("enter", "fork"), ("esc", "close")]);
        console.render(width)
    }

    fn handle_input(&mut self, data: &str) {
        self.list.borrow_mut().handle_input(data);
    }
}
