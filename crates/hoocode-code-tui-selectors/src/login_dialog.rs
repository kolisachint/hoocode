//! The login dialog that replaces the prompt during `/login`, hoocode
//! `components/login-dialog.ts`.
//!
//! The dialog draws as the DOCKING console (M7, spec §4 d10): the heavy
//! frame names the flow, the rows carry the URL and any code prompt, the
//! `▸` query row takes the typed answer, and the hint foot names the keys.
//! There is no QR facility in the repo and no new deps are allowed, so the
//! dialog shows the URL/code only and notes that gap on the auth step.
//!
//! Adaptations: the original's `showPrompt`/`showManualInput` return a
//! promise the flow awaits; here they arm the input and the answer comes out
//! as a [`LoginDialogEvent`] from [`LoginDialogComponent::take_events`].
//! `showAuth` does not open the browser itself; the owner calls the URL
//! opener next to it (`utils/open-url.ts`). The provider's display name is
//! resolved by the caller.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_ai_types::AbortSignal;
use hoocode_code_tui_theme::style_input;
use hoocode_code_tui_widgets::console::{hints, Console};
use hoocode_tui_components::Input;
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{Line, Palette, Span, Style};
use hoocode_tui_render::Component;

/// What the dialog hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginDialogEvent {
    /// The armed input was submitted.
    Submitted(String),
    /// Escape: the login is cancelled and the signal aborted.
    Cancelled,
}

fn line(text: String, style: Style) -> Line<'static> {
    Line::from(vec![Span::styled(text, style)])
}

/// `LoginDialogComponent`.
pub struct LoginDialogComponent {
    provider_name: String,
    rows: Vec<Line<'static>>,
    /// The `Starting…` row is still up: the flow's first step clears it.
    placeholder: bool,
    hint: Line<'static>,
    input: Rc<RefCell<Input>>,
    signal: AbortSignal,
    /// An answer is being waited for (`inputResolver`).
    awaiting_input: bool,
    submitted: Rc<RefCell<Option<String>>>,
    events: Vec<LoginDialogEvent>,
    /// Whether the dialog has focus: the query caret draws while set.
    focused: bool,
}

impl LoginDialogComponent {
    /// Titled `Login to <provider>` unless `title_override` names it.
    pub fn new(provider_name: &str, title_override: Option<&str>) -> Self {
        let _ = title_override;
        let pal = Palette::current();
        let mut input = Input::new();
        style_input(&mut input);
        let submitted: Rc<RefCell<Option<String>>> = Rc::default();
        let sink = submitted.clone();
        input.on_submit = Some(Box::new(move |value: &str| {
            *sink.borrow_mut() = Some(value.to_string())
        }));
        Self {
            provider_name: provider_name.to_string(),
            rows: vec![line("Starting…".to_string(), pal.faint())],
            placeholder: true,
            hint: hints(&[("esc", "cancel")]),
            input: Rc::new(RefCell::new(input)),
            signal: AbortSignal::new(),
            awaiting_input: false,
            submitted,
            events: Vec::new(),
            focused: false,
        }
    }

    /// Aborted when the user cancels.
    pub fn signal(&self) -> AbortSignal {
        self.signal.clone()
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<LoginDialogEvent> {
        std::mem::take(&mut self.events)
    }

    /// A blank row before the next step, unless nothing is shown yet.
    fn separate(&mut self) {
        if self.placeholder {
            self.rows.clear();
            self.placeholder = false;
        } else if !self.rows.is_empty() {
            self.rows.push(Line::raw(""));
        }
    }

    fn cancel(&mut self) {
        self.signal.abort();
        self.awaiting_input = false;
        self.events.push(LoginDialogEvent::Cancelled);
    }

    /// `showAuth`: the URL (a link) and any instructions. No QR facility
    /// exists in the repo, so the dialog says so and points at the link.
    pub fn show_auth(&mut self, url: &str, instructions: Option<&str>) {
        let pal = Palette::current();
        self.separate();
        self.rows.push(line(url.to_string(), pal.acc()));
        self.rows.push(line(
            "open the link in a browser to dock".to_string(),
            pal.faint(),
        ));
        self.rows.push(line(
            "No QR code in this terminal build — the link above docks the same provider."
                .to_string(),
            pal.faint(),
        ));
        if let Some(instructions) = instructions.filter(|i| !i.is_empty()) {
            self.rows.push(Line::raw(""));
            self.rows.push(line(instructions.to_string(), pal.warn()));
        }
        self.hint = hints(&[("esc", "cancel")]);
    }

    /// `showManualInput`: an input for a pasted code or redirect URL.
    pub fn show_manual_input(&mut self, prompt: &str) {
        let pal = Palette::current();
        self.separate();
        self.rows.push(line(prompt.to_string(), pal.faint()));
        self.awaiting_input = true;
        self.hint = hints(&[("enter", "submit"), ("esc", "cancel")]);
    }

    /// `showPrompt`: a question under what is already shown.
    pub fn show_prompt(&mut self, message: &str, placeholder: Option<&str>) {
        let pal = Palette::current();
        self.separate();
        self.rows.push(line(message.to_string(), Style::default()));
        if let Some(example) = placeholder.filter(|p| !p.is_empty()) {
            self.rows
                .push(line(format!("e.g., {example}"), pal.faint()));
        }
        self.input.borrow_mut().set_value("");
        self.awaiting_input = true;
        self.hint = hints(&[("enter", "submit"), ("esc", "cancel")]);
    }

    /// `showInfo`: lines to read, nothing to answer.
    pub fn show_info(&mut self, lines: &[String]) {
        let pal = Palette::current();
        self.separate();
        for text in lines {
            self.rows.push(line(text.clone(), pal.ink()));
        }
        self.hint = hints(&[("esc", "close")]);
    }

    /// `showWaiting`: a polling flow's wait.
    pub fn show_waiting(&mut self, message: &str) {
        let pal = Palette::current();
        self.separate();
        self.rows.push(line(format!("◐ {message}"), pal.faint()));
        self.hint = hints(&[("esc", "cancel")]);
    }

    /// `showProgress`.
    pub fn show_progress(&mut self, message: &str) {
        let pal = Palette::current();
        self.separate();
        self.rows.push(line(message.to_string(), pal.faint()));
    }
}

impl Component for LoginDialogComponent {
    /// The DOCKING console: the provider in the title count, the rows above
    /// the `▸` query row while an answer is armed, and the hint foot. Keys
    /// and events are unchanged.
    fn render(&mut self, width: u16) -> Vec<String> {
        let mut console = Console::new("DOCKING · /login", self.provider_name.clone());
        console.top = self.rows.clone();
        if self.awaiting_input {
            let query = self.input.borrow().get_value().to_string();
            console.query = Some(hoocode_code_tui_widgets::console::query_row(
                &query,
                self.focused,
            ));
        }
        console.hint = self.hint.clone();
        console.render(width as usize)
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "tui.select.cancel") {
            self.cancel();
            return;
        }
        self.input.borrow_mut().handle_input_with(data, &kb);
        let submitted = self.submitted.borrow_mut().take();
        if let Some(value) = submitted {
            // Only an answer something is waiting for counts.
            if std::mem::take(&mut self.awaiting_input) {
                self.events.push(LoginDialogEvent::Submitted(value));
            }
        }
    }

    fn invalidate(&mut self) {
        self.input.borrow_mut().invalidate();
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.input.borrow_mut().set_focused(focused);
    }
}
