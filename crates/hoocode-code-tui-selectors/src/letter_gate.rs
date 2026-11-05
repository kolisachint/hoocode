//! The letter gate of the list pickers (`/scoped-models`, `/resume`, `/tree`).
//!
//! A picker starts in list mode. There a bare letter is one of the picker's
//! keys (`x` deletes a session, `s` saves the models), and a letter with no
//! key of its own does nothing. `/` switches to typing mode, where every
//! printable key types into the query. A query that holds text keeps the
//! picker typing, so letters always type once there is text. In typing mode
//! Esc clears a non-empty query and, with an empty query, returns to list
//! mode; a second Esc in list mode closes the picker.

/// The mode of a picker's query line: list mode, or typing mode.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LetterGate {
    typing: bool,
}

/// What Esc does in a picker with a query line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EscapeAction {
    /// The query had text. It is cleared, and the picker keeps typing.
    ClearQuery,
    /// Typing with an empty query: the picker is back in list mode.
    LeaveTyping,
    /// List mode: the picker closes.
    Close,
}

impl LetterGate {
    /// Whether the picker is in list mode, where a bare letter acts as a key.
    pub(crate) fn lists(self, query_empty: bool) -> bool {
        !self.typing && query_empty
    }

    /// `/` in list mode: printable keys type from now on.
    pub(crate) fn start_typing(&mut self) {
        self.typing = true;
    }

    /// What Esc does, given whether the query is empty.
    pub(crate) fn escape(&mut self, query_empty: bool) -> EscapeAction {
        if !query_empty {
            self.typing = true;
            EscapeAction::ClearQuery
        } else if self.typing {
            self.typing = false;
            EscapeAction::LeaveTyping
        } else {
            EscapeAction::Close
        }
    }
}

/// Whether `data` is one printable character (a letter, digit, symbol or
/// space). A control key or an escape sequence is not.
pub(crate) fn is_printable(data: &str) -> bool {
    let mut chars = data.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_mode_until_slash_then_typing_until_the_query_empties_and_escape_leaves() {
        let mut gate = LetterGate::default();
        assert!(gate.lists(true));
        gate.start_typing();
        assert!(!gate.lists(true), "typing with an empty query");
        assert_eq!(gate.escape(false), EscapeAction::ClearQuery);
        assert!(!gate.lists(true), "still typing after clearing");
        assert_eq!(gate.escape(true), EscapeAction::LeaveTyping);
        assert!(gate.lists(true));
        assert_eq!(gate.escape(true), EscapeAction::Close);
    }

    #[test]
    fn text_in_the_query_always_types() {
        assert!(!LetterGate::default().lists(false));
    }

    #[test]
    fn printable_is_one_character_that_is_not_control() {
        assert!(is_printable("x") && is_printable("/") && is_printable(" "));
        assert!(!is_printable("\x1b[A") && !is_printable("\r") && !is_printable("ab"));
    }
}
