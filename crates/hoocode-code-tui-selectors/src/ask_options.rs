//! The question card (`components/ask-options.ts`): the agent asks the user,
//! one decision per step. It draws a blocking pane in the prompt's slot
//! (declutter spec, Dialogs): a yellow `Question` chip with the question in
//! bold and `1 of 2` dim, a dim detail line, numbered options with their
//! descriptions in aligned columns, and the answer row (`or type your own
//! answer`) with the hint row last. Up/down move (wrapping, down from the
//! last option lands on the answer row), `tab` or `app.options.next` (right)
//! confirms and advances (submitting on the last step), `app.options.back`
//! (left) steps back, 1-9 quick-pick (the answer row's number only moves
//! there, typing sends with `enter`), and esc skips the question: the agent
//! is told it was skipped and states the default it uses. Answered steps
//! stay as a breadcrumb.

use hoocode_code_tui_keybindings::{key_glyph_text, key_hint_text};
use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tools_optin::ask_options::AskQuestion;
use hoocode_code_tui_widgets::pane::{
    fit_hint_line, hint_line, input_line, wrap_text, Chip, Pane, PaneStyle,
};
use hoocode_tui_components::{Input, PickerCursor};
use hoocode_tui_keys::{get_keybindings, matches_key};
use hoocode_tui_ratatui::{Line, Palette, Span};
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_to_width, visible_width};

/// `AskOptionsOptions`: nothing to set yet (esc always skips).
#[derive(Debug, Clone, Copy, Default)]
pub struct AskOptionsOptions {}

struct State {
    questions: Vec<AskQuestion>,
    step: usize,
    /// The highlighted row: an option, or the custom row after the options.
    /// Its length is the current step's row count.
    cursor: PickerCursor,
    answers: Vec<String>,
    custom_input: Input,
    focused: bool,
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

    /// The hint row: `1–2 pick · tab next question · esc skip`. The move
    /// item drops first at narrow widths, and `next question` shortens to
    /// `next` where the long row would overflow (mock §S3).
    fn render_hints(&self, last: bool, width: usize) -> Line<'static> {
        let pick = format!("1–{}", self.current().options.len().max(1));
        let (_, back) = self.custom_arrows();
        let tab = key_hint_text("tui.input.tab");
        let back_key = key_glyph_text("app.options.back");
        let skip = key_hint_text("tui.select.cancel");
        let mut hints: Vec<(&str, &str)> = vec![(pick.as_str(), "pick")];
        let next = if last { "submit" } else { "next question" };
        hints.push((tab.as_str(), next));
        if self.step > 0 && back {
            hints.push((back_key.as_str(), "back"));
        }
        hints.push((skip.as_str(), "skip"));
        let budget = width.saturating_sub(2);
        if !last && hint_line(&hints).width() > budget {
            hints[1].1 = "next";
        }
        fit_hint_line(&hints, false, width)
    }

    /// The pane: title chip, breadcrumb and detail rows, the option rows
    /// and the answer row.
    fn pane(&self, width: usize) -> Pane {
        let st = PaneStyle::current();
        let pal = Palette::current();
        let cur = self.current();
        let last = self.step == self.questions.len() - 1;
        // The yellow chip holds the question (the blocking title). The
        // dim `1 of 2` follows it. A question too long for the chip keeps
        // its first line in the chip, cut with `…`, and the whole question
        // wraps under the title (mock §S3).
        let mut pane = Pane::new(cur.question.clone());
        pane.chip = Chip::Warn;
        let count = if self.questions.len() > 1 {
            format!("{} of {}", self.step + 1, self.questions.len())
        } else {
            String::new()
        };
        let ink = st.text_style();
        let fits = visible_width(&cur.question) + visible_width(&count) + 14 <= width;
        let count_w = if count.is_empty() {
            0
        } else {
            visible_width(&count) + 2
        };
        let mut question_rows: Vec<String> = Vec::new();
        if fits {
            if !count.is_empty() {
                pane.facts = Line::from(vec![Span::styled(count.clone(), st.dim_style())]);
            }
        } else {
            let wrapped = wrap_text(&cur.question, width.saturating_sub(4));
            let chip_budget = width.saturating_sub(count_w + 14);
            let first = wrapped.first().cloned().unwrap_or_default();
            let head = truncate_to_width(&first, chip_budget, "…", false);
            // The first line already sits in the chip when it fits whole.
            let skip = usize::from(head == first);
            pane.title = head;
            question_rows = wrapped.into_iter().skip(skip).collect();
            if !count.is_empty() {
                pane.facts = Line::from(vec![Span::styled(count.clone(), st.dim_style())]);
            }
        }
        // A narrow pane wraps the question and the detail instead of
        // cutting them: two cells of caller indent under the row's pad.
        let wrap = width.saturating_sub(4);

        for i in 0..self.step {
            let q = &self.questions[i];
            pane.top.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("● ", pal.ok()),
                Span::styled(
                    q.short.as_deref().unwrap_or(&q.question).to_string(),
                    st.dim_style(),
                ),
                Span::styled(" → ", st.dim_style()),
                Span::styled(
                    self.answers.get(i).map_or(String::new(), |s| s.clone()),
                    st.text_style(),
                ),
            ]));
        }
        for part in question_rows {
            pane.top
                .push(Line::from(vec![Span::raw("  "), Span::styled(part, ink)]));
        }
        if let Some(detail) = &cur.detail {
            for part in wrap_text(detail, wrap) {
                pane.top.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(part, st.dim_style()),
                ]));
            }
        }

        // Option rows: number, label and description in aligned columns.
        // A description that would overflow moves to its own row at the
        // label's column, so labels stay aligned (mock §S3).
        let label_w = cur
            .options
            .iter()
            .map(|o| visible_width(&o.label))
            .max()
            .unwrap_or(0);
        let item_budget = width.saturating_sub(4);
        let on_custom = self.is_on_custom_row();
        for (i, o) in cur.options.iter().enumerate() {
            let mut spans = vec![
                Span::styled(format!("{}  ", i + 1), ink),
                Span::styled(o.label.clone(), ink),
            ];
            if let Some(description) = &o.description {
                let num_w = visible_width(&format!("{}", i + 1)) + 2;
                let aligned = num_w + label_w + 2 + visible_width(description) <= item_budget;
                if aligned {
                    let fill = label_w.saturating_sub(visible_width(&o.label));
                    spans.push(Span::raw(" ".repeat(fill + 2)));
                    spans.push(Span::styled(description.clone(), st.dim_style()));
                } else {
                    pane.flow.entry(i).or_default().push(Line::from(vec![
                        Span::raw("     "),
                        Span::styled(description.clone(), st.dim_style()),
                    ]));
                }
            }
            if o.recommended {
                // The recommended answer reads green: it is the safe
                // default, not a warning.
                spans.push(Span::styled("  recommended", pal.ok()));
            }
            pane.rows.push(Line::from(spans));
        }
        pane.selected = (!on_custom && !cur.options.is_empty()).then(|| self.cursor.selected());

        if cur.allow_custom {
            pane.input = Some(if on_custom {
                input_line(self.custom_input.get_value(), "", self.focused)
            } else {
                input_line("", "or type your own answer", false)
            });
        }
        pane.hint = self.render_hints(last, width);
        pane.style = Some(st);
        pane
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
        _options: AskOptionsOptions,
    ) -> Self {
        let rows = questions.first().map_or(0, State::row_count);
        let state = Rc::new(RefCell::new(State {
            cursor: PickerCursor::new(rows, 10),
            questions,
            step: 0,
            answers: Vec::new(),
            custom_input: Input::new(),
            focused: false,
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

    /// Esc skips the question: the agent is told and states its default.
    fn escape(&mut self) {
        (self.on_cancel)();
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
        self.state
            .borrow()
            .pane(width as usize)
            .render(width as usize)
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
        if (next && kb.matches(data, "app.options.next"))
            || data == "\t"
            || kb.matches(data, "tui.select.confirm")
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

        // A letter on an option row starts the answer row's text.
        let allow_custom = self.state.borrow().current().allow_custom;
        let mut typed = data.chars();
        if let (true, Some(c), None) = (allow_custom, typed.next(), typed.next()) {
            if c.is_alphabetic() {
                let mut state = self.state.borrow_mut();
                state.cursor.select(option_count);
                state.custom_input.handle_input(data);
                return;
            }
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
