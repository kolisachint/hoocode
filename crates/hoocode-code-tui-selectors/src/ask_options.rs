//! The question card (`components/ask-options.ts`): the agent asks the user,
//! one decision per step. It draws the Mission Control amber card (M6,
//! spec §4 c2): `◉ MISSION CONTROL ASKS` with numbered answers, the
//! recommended one tagged, and the last row `N something else · type it`.
//! Up/down move (wrapping), `app.options.next` (→) confirms and advances
//! (submitting on the last step), `app.options.back` (←) steps back, 1-9
//! quick-pick (the custom row's number only moves there, typing sends with
//! `enter`), and esc picks option 1 for the tool — or skips, for MCP
//! elicitation forms, which keep esc-as-cancel. Answered steps stay as a
//! breadcrumb.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tools_optin::ask_options::AskQuestion;
use hoocode_code_tui_theme::{select_gutter, theme, SELECT_CURSOR};
use hoocode_code_tui_widgets::card_frame::render_card;
use hoocode_tui_components::{Input, PickerCursor};
use hoocode_tui_keys::{get_keybindings, matches_key};
use hoocode_tui_render::Component;

/// `AskOptionsOptions`.
#[derive(Debug, Clone, Copy)]
pub struct AskOptionsOptions {
    /// Mission Control: esc answers the current step with option 1 (the
    /// question card, spec §6.2). False keeps esc-as-skip, for MCP
    /// elicitation forms (their Cancel choice must stay reachable).
    pub esc_answers_first: bool,
}

impl Default for AskOptionsOptions {
    fn default() -> Self {
        Self {
            esc_answers_first: true,
        }
    }
}

struct State {
    questions: Vec<AskQuestion>,
    step: usize,
    /// The highlighted row: an option, or the custom row after the options.
    /// Its length is the current step's row count.
    cursor: PickerCursor,
    answers: Vec<String>,
    custom_input: Input,
    focused: bool,
    esc_answers_first: bool,
}

impl State {
    fn current(&self) -> &AskQuestion {
        &self.questions[self.step]
    }

    fn row_count(q: &AskQuestion) -> usize {
        q.options.len() + usize::from(q.allow_custom)
    }

    fn is_on_custom_row(&self) -> bool {
        let q = self.current();
        q.allow_custom && self.cursor.selected() == q.options.len()
    }

    /// How the arrows divide between the step and the custom row's text: the
    /// text gets first refusal, the step takes what is left at the ends.
    fn custom_arrows(&self) -> (bool, bool) {
        if !self.is_on_custom_row() {
            return (true, true);
        }
        let typed = self.custom_input.get_value().len();
        (self.custom_input.get_cursor() >= typed, typed == 0)
    }

    fn render_hints(&self, last: bool) -> String {
        let t = theme();
        let sep = t.fg("muted", " · ");
        let hint = |keys: &str, desc: &str| t.fg("dim", keys) + &t.fg("muted", &format!(" {desc}"));
        let range = format!("1–{}", Self::row_count(self.current()));
        let (next, back) = self.custom_arrows();
        let mut parts = vec![
            t.fg("dim", &range) + &t.fg("muted", " answer"),
            hint("↑↓", "move"),
        ];
        // Mid-answer the arrows are the text cursor's: enter commits.
        parts.push(hint(
            if next { "→" } else { "enter" },
            if last { "submit" } else { "next" },
        ));
        if self.step > 0 && back {
            parts.push(hint("←", "back"));
        }
        // Spec §6.2: esc picks option 1 on the card (elicitation skips).
        parts.push(hint(
            "esc",
            if self.esc_answers_first {
                "pick 1"
            } else {
                "skip"
            },
        ));
        parts.join(&sep)
    }

    fn render_rows(&self) -> Vec<String> {
        let t = theme();
        let mut lines = Vec::new();
        let cur = self.current();
        let last = self.step == self.questions.len() - 1;

        for i in 0..self.step {
            let q = &self.questions[i];
            let check = t.fg("success", "✓");
            let label = t.fg("dim", q.short.as_deref().unwrap_or(&q.question));
            let arrow = t.fg("dim", "→ ");
            let answer = t.fg("accent", self.answers.get(i).map_or("", String::as_str));
            lines.push(format!("{check} {label}   {arrow}{answer}"));
        }
        if self.step > 0 {
            lines.push(String::new());
        }

        let step_label = t.fg(
            "dim",
            &format!("{}/{}", self.step + 1, self.questions.len()),
        );
        lines.push(format!("{step_label} {}", t.bold(&cur.question)));
        if let Some(detail) = &cur.detail {
            lines.push(t.fg("muted", &format!("    {detail}")));
        }
        lines.push(String::new());

        for (i, o) in cur.options.iter().enumerate() {
            let active = i == self.cursor.selected();
            let cursor = t.fg(
                "accent",
                &if active {
                    SELECT_CURSOR.to_string()
                } else {
                    select_gutter()
                },
            );
            let num = t.fg(if active { "accent" } else { "dim" }, &(i + 1).to_string());
            let label = if active {
                t.bold(&o.label)
            } else {
                o.label.clone()
            };
            let mut line = format!("{cursor}{num}  {label}");
            if let Some(description) = &o.description {
                line += &t.fg("muted", &format!("   {description}"));
            }
            if o.recommended {
                line += &t.fg("warning", "   recommended");
            }
            lines.push(line);
        }

        if cur.allow_custom {
            let active = self.cursor.selected() == cur.options.len();
            let cursor = t.fg(
                "accent",
                &if active {
                    SELECT_CURSOR.to_string()
                } else {
                    select_gutter()
                },
            );
            let num = t.fg(
                if active { "accent" } else { "dim" },
                &(cur.options.len() + 1).to_string(),
            );
            if active {
                let value = self.custom_input.get_value();
                let caret = if self.focused {
                    t.fg("warning", "▏")
                } else {
                    String::new()
                };
                let body = if value.is_empty() {
                    format!("{caret}{}", t.fg("dim", "type it"))
                } else {
                    format!("{}{caret}", t.fg("text", value))
                };
                lines.push(format!(
                    "{cursor}{num}  {}  {body}",
                    t.bold("something else"),
                ));
            } else {
                lines.push(format!(
                    "{cursor}{num}  {}   {}",
                    t.fg("muted", "something else"),
                    t.fg("dim", "type it"),
                ));
            }
        }

        lines.push(self.render_hints(last));
        lines
    }

    fn render_body(&self, width: usize) -> Vec<String> {
        let mut rows = self.render_rows();
        let foot = rows.pop().unwrap_or_default();
        render_card(
            "◉ MISSION CONTROL ASKS",
            "sets the default for every call",
            &rows,
            &foot,
            width,
        )
    }
}

/// `AskOptionsComponent`.
pub struct AskOptionsComponent {
    state: Rc<RefCell<State>>,
    on_submit: Box<dyn FnMut(Vec<String>)>,
    on_cancel: Box<dyn FnMut()>,
}

impl AskOptionsComponent {
    pub fn new(
        questions: Vec<AskQuestion>,
        on_submit: Box<dyn FnMut(Vec<String>)>,
        on_cancel: Box<dyn FnMut()>,
        options: AskOptionsOptions,
    ) -> Self {
        let rows = questions.first().map_or(0, State::row_count);
        let state = Rc::new(RefCell::new(State {
            cursor: PickerCursor::new(rows, 10),
            questions,
            step: 0,
            answers: Vec::new(),
            custom_input: Input::new(),
            focused: false,
            esc_answers_first: options.esc_answers_first,
        }));
        Self {
            state,
            on_submit,
            on_cancel,
        }
    }

    fn confirm(&mut self) {
        let mut state = self.state.borrow_mut();
        let value = if state.is_on_custom_row() {
            let value = state.custom_input.get_value().trim().to_string();
            if value.is_empty() {
                return; // An empty custom answer cannot be submitted.
            }
            value
        } else {
            let index = state.cursor.selected();
            state.current().options[index].label.clone()
        };
        let step = state.step;
        if state.answers.len() <= step {
            state.answers.resize(step + 1, String::new());
        }
        state.answers[step] = value;
        if step < state.questions.len() - 1 {
            state.step += 1;
            let rows = State::row_count(state.current());
            state.cursor.reset(rows);
            state.custom_input.set_value("");
        } else {
            let answers = state.answers.clone();
            drop(state);
            (self.on_submit)(answers);
        }
    }

    /// Esc on the card: answer this step with option 1 (Mission Control,
    /// spec §6.2). Anything else skips: elicitation forms, and a step with
    /// no options to pick.
    fn escape(&mut self) {
        let first = self.state.borrow().esc_answers_first
            && !self.state.borrow().current().options.is_empty();
        if !first {
            (self.on_cancel)();
            return;
        }
        self.state.borrow_mut().cursor.select(0);
        self.confirm();
    }

    fn back(&mut self) {
        let mut state = self.state.borrow_mut();
        if state.step == 0 {
            return;
        }
        state.step -= 1;
        let rows = State::row_count(state.current());
        state.cursor.reset(rows);
        state.custom_input.set_value("");
    }
}

impl Component for AskOptionsComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.state.borrow().render_body(width as usize)
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        let (on_custom, option_count) = {
            let state = self.state.borrow();
            let cur = state.current();
            (state.is_on_custom_row(), cur.options.len())
        };

        // Home and End belong to the custom answer's text while it has focus.
        let text_jump = on_custom && (matches_key(data, "home") || matches_key(data, "end"));
        if !text_jump && self.state.borrow_mut().cursor.handle_key(data, &kb) {
            return;
        }
        let (next, back) = self.state.borrow().custom_arrows();
        if back && kb.matches(data, "app.options.back") {
            self.back();
            return;
        }
        if (next && kb.matches(data, "app.options.next")) || kb.matches(data, "tui.select.confirm")
        {
            self.confirm();
            return;
        }
        if kb.matches(data, "tui.select.cancel") {
            self.escape();
            return;
        }

        if on_custom {
            // Free-form typing goes to the hidden input; its value is drawn inline.
            self.state.borrow_mut().custom_input.handle_input(data);
            return;
        }

        // Quick-pick among the rows: an option answers at once; the custom
        // row's number only moves there, and typing sends with `enter`.
        let mut chars = data.chars();
        if let (Some(c @ '1'..='9'), None) = (chars.next(), chars.next()) {
            let n = c as usize - '1' as usize;
            if n < option_count {
                self.state.borrow_mut().cursor.select(n);
                self.confirm();
            } else if n == option_count && self.state.borrow().current().allow_custom {
                self.state.borrow_mut().cursor.select(n);
            }
        }
    }

    fn invalidate(&mut self) {}

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        let mut state = self.state.borrow_mut();
        state.focused = focused;
        state.custom_input.set_focused(focused);
    }
}
