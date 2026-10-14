//! The options pane (`components/ask-options.ts`): the agent asks the user for
//! one decision per step. Up/down move (wrapping), `app.options.next` (→)
//! confirms and advances (submitting on the last step), `app.options.back`
//! (←) steps back, 1-9 quick-pick, the custom row takes typed text (where the
//! arrows move the text cursor first), and cancel (esc) skips the lot.
//! Answered steps stay as a breadcrumb.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tools_optin::ask_options::AskQuestion;
use hoocode_code_tui_theme::{select_gutter, theme, SELECT_CURSOR};
use hoocode_code_tui_widgets::input_frame::{InputFrame, InputFrameOptions};
use hoocode_tui_components::{Input, PickerCursor, DEFAULT_INPUT_PROMPT};
use hoocode_tui_keys::{get_keybindings, matches_key};
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_to_width, visible_width};

/// `AskOptionsOptions`.
#[derive(Debug, Clone, Copy)]
pub struct AskOptionsOptions {
    /// Draw the prompt's frame around the pane; false when a host surface
    /// frames it already.
    pub framed: bool,
}

impl Default for AskOptionsOptions {
    fn default() -> Self {
        Self { framed: true }
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
        let kb = get_keybindings();
        let t = theme();
        let sep = t.fg("muted", " · ");
        let hint = |keys: &str, desc: &str| t.fg("dim", keys) + &t.fg("muted", &format!(" {desc}"));
        let up_down = format!(
            "{}/{}",
            kb.get_keys("tui.select.up").join("/"),
            kb.get_keys("tui.select.down").join("/")
        );
        let (next, back) = self.custom_arrows();
        let mut parts = vec![hint(&up_down, "move")];
        // Mid-answer the arrows are the text cursor's: name the key that commits.
        let next_keys = if next {
            kb.get_keys("app.options.next")
        } else {
            kb.get_keys("tui.select.confirm")
        };
        parts.push(hint(
            &next_keys.join("/"),
            if last { "submit" } else { "next" },
        ));
        if self.step > 0 && back {
            parts.push(hint(&kb.get_keys("app.options.back").join("/"), "back"));
        }
        parts.push(hint(&kb.get_keys("tui.select.cancel").join("/"), "skip"));
        parts.join(&sep)
    }

    fn render_body(&self, width: usize, labelled: bool) -> Vec<String> {
        let t = theme();
        let mut lines = Vec::new();
        let cur = self.current();
        let last = self.step == self.questions.len() - 1;

        let title = if labelled {
            t.bold(&t.fg("borderAccent", "INPUT NEEDED"))
        } else {
            String::new()
        };
        lines.push(spread(&title, &self.render_hints(last), width));

        for i in 0..self.step {
            let q = &self.questions[i];
            let check = t.fg("success", "✓");
            let label = t.fg("dim", q.short.as_deref().unwrap_or(&q.question));
            let arrow = t.fg("dim", "→ ");
            let answer = t.fg("accent", self.answers.get(i).map_or("", String::as_str));
            lines.push(truncate_to_width(
                &format!("{check} {label}   {arrow}{answer}"),
                width,
                "...",
                false,
            ));
        }
        if self.step > 0 {
            lines.push(String::new());
        }

        let step_label = t.fg(
            "dim",
            &format!("{}/{}", self.step + 1, self.questions.len()),
        );
        lines.push(truncate_to_width(
            &format!("{step_label} {}", t.bold(&cur.question)),
            width,
            "...",
            false,
        ));
        if let Some(detail) = &cur.detail {
            lines.push(truncate_to_width(
                &t.fg("muted", &format!("    {detail}")),
                width,
                "...",
                false,
            ));
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
            let mut line = format!("{cursor}{num} {label}");
            if o.recommended {
                line += &t.fg("success", " (recommended)");
            }
            if let Some(description) = &o.description {
                line += &t.fg("muted", &format!("   {description}"));
            }
            lines.push(truncate_to_width(&line, width, "...", false));
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
            let plus = t.fg(if active { "accent" } else { "dim" }, "+");
            if active {
                let value = self.custom_input.get_value();
                let prompt = t.fg("muted", DEFAULT_INPUT_PROMPT);
                let caret = if self.focused {
                    t.fg("accent", "▏")
                } else {
                    String::new()
                };
                let body = if value.is_empty() {
                    format!("{caret}{}", t.fg("dim", "type your own answer"))
                } else {
                    format!("{}{caret}", t.fg("text", value))
                };
                lines.push(truncate_to_width(
                    &format!("{cursor}{plus} {prompt} {body}"),
                    width,
                    "...",
                    false,
                ));
            } else {
                let label = t.fg("muted", "custom answer");
                let desc = t.fg("dim", "type your own");
                lines.push(truncate_to_width(
                    &format!("{cursor}{plus} {label}   {desc}"),
                    width,
                    "...",
                    false,
                ));
            }
        }

        lines.push(t.fg("dim", &self.cursor.position()));
        lines
    }
}

fn spread(left: &str, right: &str, width: usize) -> String {
    let gap = (width as i64 - visible_width(left) as i64 - visible_width(right) as i64).max(1);
    truncate_to_width(
        &format!("{left}{}{right}", " ".repeat(gap as usize)),
        width,
        "",
        false,
    )
}

/// The framed pane's body, recomputed from live state each frame.
struct Body(Rc<RefCell<State>>);

impl Component for Body {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.0.borrow().render_body(width as usize, false)
    }
}

/// `AskOptionsComponent`.
pub struct AskOptionsComponent {
    state: Rc<RefCell<State>>,
    frame: Option<InputFrame>,
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
        }));
        let frame = options.framed.then(|| {
            let mut frame = InputFrame::new(InputFrameOptions {
                title: Some("input needed".to_string()),
                ..Default::default()
            });
            frame.add_child(Rc::new(RefCell::new(Body(state.clone()))));
            frame
        });
        Self {
            state,
            frame,
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
        match &mut self.frame {
            Some(frame) => frame.render(width),
            None => self.state.borrow().render_body(width as usize, true),
        }
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
            (self.on_cancel)();
            return;
        }

        if on_custom {
            // Free-form typing goes to the hidden input; its value is drawn inline.
            self.state.borrow_mut().custom_input.handle_input(data);
            return;
        }

        // Quick-pick among the listed options (not the custom row).
        let mut chars = data.chars();
        if let (Some(c @ '1'..='9'), None) = (chars.next(), chars.next()) {
            let n = c as usize - '1' as usize;
            if n < option_count {
                self.state.borrow_mut().cursor.select(n);
                self.confirm();
            }
        }
    }

    fn invalidate(&mut self) {
        if let Some(frame) = &mut self.frame {
            frame.invalidate();
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        let mut state = self.state.borrow_mut();
        state.focused = focused;
        state.custom_input.set_focused(focused);
    }
}
