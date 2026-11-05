//! Ctrl+R at the prompt: a reverse incremental search over the prompt history
//! (`docs/design/themes-keybindings.md`, K-7).
//!
//! The search owns the keys while it is open. Typing narrows the query to the
//! newest past prompt that contains it (ignoring case). Ctrl+R goes to the next
//! older match. Enter puts the match in the prompt for editing and does not
//! submit it. Esc (or Ctrl+C) cancels, and the prompt keeps what it held. The
//! prompt's text is not touched until Enter, so a cancel restores it as it was.
//! The keys come from the keymap, and so do the hints on the search rows.

use hoocode_code_tui_keybindings::key_hint_text;
use hoocode_tui_components::Editor;
use hoocode_tui_keys::get_keybindings;

use crate::transient_lines::sanitize_status_text;

use super::input::is_plain_text;
use super::*;

/// The index of the first history entry at or after `from` that contains
/// `query`, ignoring case. History is newest first, so a larger index is an
/// older prompt. An empty query matches every entry.
pub(super) fn find_match(history: &[String], query: &str, from: usize) -> Option<usize> {
    let needle = query.to_lowercase();
    (from..history.len()).find(|&i| history[i].to_lowercase().contains(&needle))
}

/// An open search: the query typed so far and the history entry it shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct HistorySearch {
    pub(super) query: String,
    /// The index into the history (0 is the newest) that the search shows.
    pub(super) found: Option<usize>,
}

impl HistorySearch {
    /// A search with an empty query: the newest prompt shows.
    pub(super) fn start(history: &[String]) -> Self {
        Self {
            query: String::new(),
            found: find_match(history, "", 0),
        }
    }

    /// Adds typed text to the query and shows the newest match for it.
    pub(super) fn type_text(&mut self, text: &str, history: &[String]) {
        self.query.push_str(text);
        self.found = find_match(history, &self.query, 0);
    }

    /// Removes the query's last character and shows the newest match for the rest.
    pub(super) fn backspace(&mut self, history: &[String]) {
        self.query.pop();
        self.found = find_match(history, &self.query, 0);
    }

    /// Ctrl+R: shows the next older match. With none older, the shown match stays.
    pub(super) fn older(&mut self, history: &[String]) {
        if let Some(current) = self.found {
            if let Some(next) = find_match(history, &self.query, current + 1) {
                self.found = Some(next);
            }
        }
    }

    /// The prompt the search shows, if it has a match.
    pub(super) fn matched<'a>(&self, history: &'a [String]) -> Option<&'a str> {
        self.found.and_then(|i| history.get(i)).map(String::as_str)
    }
}

/// What one key does to an open search.
enum Step {
    Continue,
    Cancel,
    Accept(String),
    /// Enter with no match: nothing to put in the prompt.
    AcceptNothing,
}

/// The search's two rows above the prompt: the query and the match, then the
/// keys that act on it. Built from the keymap, so a user's keys show.
pub(super) fn search_rows(search: &HistorySearch, history: &[String]) -> (String, String) {
    let shown = match search.matched(history) {
        Some(text) => format!("`{}': {}", search.query, sanitize_status_text(text)),
        None => format!("`{}': no match", search.query),
    };
    let hints = format!(
        "{} older · {} edit · {} cancel",
        key_hint_text("app.history.search"),
        key_hint_text("tui.input.submit"),
        key_hint_text("app.interrupt"),
    );
    (format!("(reverse-i-search){shown}"), hints)
}

fn step_for(search: &mut HistorySearch, data: &str, editor: &Editor) -> Step {
    let kb = get_keybindings();
    let history = editor.history();
    if kb.matches(data, "app.interrupt") || kb.matches(data, "app.clear") {
        return Step::Cancel;
    }
    if kb.matches(data, "tui.input.submit") {
        return match search.matched(history) {
            Some(text) => Step::Accept(text.to_string()),
            None => Step::AcceptNothing,
        };
    }
    if kb.matches(data, "app.history.search") {
        search.older(history);
    } else if kb.matches(data, "tui.editor.deleteCharBackward") {
        search.backspace(history);
    } else if is_plain_text(data) {
        search.type_text(data, history);
    }
    Step::Continue
}

impl Mode {
    /// `app.history.search` at the prompt: opens the search with an empty query.
    pub(super) fn open_history_search(&mut self) {
        let search = HistorySearch::start(self.editor.borrow().editor.history());
        self.history_search = Some(search);
        self.sync_history_search();
    }

    /// Ends the search, if one is open, and keeps the prompt as it was.
    pub(super) fn cancel_history_search(&mut self) {
        if self.history_search.take().is_some() {
            self.sync_history_search();
        }
    }

    /// One key while the search is open. It never reaches the prompt editor.
    pub(super) fn history_search_key(&mut self, data: &str) {
        let Some(mut search) = self.history_search.take() else {
            return;
        };
        let step = {
            let editor = self.editor.borrow();
            step_for(&mut search, data, &editor.editor)
        };
        match step {
            Step::Continue => self.history_search = Some(search),
            Step::Cancel => {}
            // The match replaces the prompt: a draft is not joined to it.
            Step::Accept(text) => self.editor.borrow_mut().editor.set_text(&text),
            Step::AcceptNothing => self.show_status("No matching prompt in history"),
        }
        self.sync_history_search();
    }

    /// Shows the search rows (or removes them) and tells the editor whether
    /// it must hand every key to the search.
    fn sync_history_search(&mut self) {
        self.history_search_active
            .set(self.history_search.is_some());
        let rows = self.history_search.as_ref().map(|search| {
            let editor = self.editor.borrow();
            search_rows(search, editor.editor.history())
        });
        self.transient.borrow_mut().set_history_search(rows);
        self.dirty.set(true);
    }
}
