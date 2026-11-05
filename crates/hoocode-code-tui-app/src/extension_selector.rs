//! The extension selector dialog: `components/extension-selector.ts` with
//! its countdown timer (`countdown-timer.ts`).
//!
//! Drawn as a ratatui [`Console`](hoocode_code_tui_widgets::console::Console):
//! the title (with the countdown) names the console, the options are its
//! rows, and the hint foot names the keys.

use hoocode_code_tui_keybindings::{key_hint_text, key_pair_hint, key_text};
use std::time::{Duration, Instant};

use hoocode_code_tui_widgets::console::{help_rows, hints, Console};
use hoocode_tui_components::PickerCursor;
use hoocode_tui_keys::{get_keybindings, KeybindingsManager};
use hoocode_tui_ratatui::{Line, Palette, Span, Style};
use hoocode_tui_render::Component;

/// The dialog answers (K-P4): each letter picks the first option whose label
/// starts with one of its words. `y` yes-like, `n` no-like, `a` "always".
const YES_WORDS: [&str; 4] = ["Yes", "Trust and start", "Accept", "Answer"];
const NO_WORDS: [&str; 3] = ["No", "Decline", "Not now"];
const ALWAYS_WORDS: [&str; 1] = ["Always"];
const DIALOG_ANSWERS: [&str; 3] = ["app.dialog.yes", "app.dialog.no", "app.dialog.always"];
/// The newline byte (Enter), accepted as a confirm besides `tui.select.confirm`.
const NEWLINE: &str = "\n";
/// The list window: at most this many options draw.
const MAX_VISIBLE: usize = 10;

/// A text styler.
pub type ColorFn = Box<dyn Fn(&str) -> String>;

pub use hoocode_code_tui_widgets::dynamic_border::DynamicBorder;

/// `CountdownTimer`, driven by the owner's loop: [`CountdownTimer::poll`]
/// advances it once a second has passed.
pub struct CountdownTimer {
    remaining_seconds: u64,
    next_tick: Option<Instant>,
}

impl CountdownTimer {
    /// Starts at `ceil(timeout / 1s)`; the caller shows that first value.
    pub fn new(timeout: Duration) -> Self {
        Self {
            remaining_seconds: timeout.as_millis().div_ceil(1000) as u64,
            next_tick: Some(Instant::now() + Duration::from_secs(1)),
        }
    }

    pub fn remaining_seconds(&self) -> u64 {
        self.remaining_seconds
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.next_tick
    }

    /// Tick if due: `Some(seconds)` after a tick, and `Some(0)` means expired.
    pub fn poll(&mut self, now: Instant) -> Option<u64> {
        let due = self.next_tick?;
        if now < due {
            return None;
        }
        self.remaining_seconds = self.remaining_seconds.saturating_sub(1);
        self.next_tick = if self.remaining_seconds == 0 {
            None
        } else {
            Some(due + Duration::from_secs(1))
        };
        Some(self.remaining_seconds)
    }

    pub fn dispose(&mut self) {
        self.next_tick = None;
    }
}

/// What the selector decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorOutcome {
    Selected(String),
    Cancelled,
}

/// One option row (content only; the console adds the `›` marker or gutter
/// and the selection band): the option text, accent on the selected row.
fn option_row(option: &str, selected: bool) -> Line<'static> {
    Line::from(vec![Span::styled(
        option.to_string(),
        if selected {
            Palette::current().acc()
        } else {
            Style::default()
        },
    )])
}

/// `ExtensionSelectorComponent`: a titled console of options, navigated with
/// the select keys (and `j`/`k`).
pub struct ExtensionSelectorComponent {
    options: Vec<String>,
    cursor: PickerCursor,
    base_title: String,
    countdown: Option<CountdownTimer>,
    on_done: Box<dyn FnMut(SelectorOutcome)>,
    /// `?` shows the keys below the options.
    help: bool,
}

impl ExtensionSelectorComponent {
    pub fn new(
        title: &str,
        options: Vec<String>,
        timeout: Option<Duration>,
        on_done: Box<dyn FnMut(SelectorOutcome)>,
    ) -> Self {
        let countdown = timeout.filter(|t| !t.is_zero()).map(CountdownTimer::new);
        Self {
            cursor: PickerCursor::new(options.len(), MAX_VISIBLE),
            options,
            base_title: title.to_string(),
            countdown,
            on_done,
            help: false,
        }
    }

    /// The option a dialog letter answers (K-P4), if the options offer one.
    fn letter_answer(&self, data: &str, kb: &KeybindingsManager) -> Option<usize> {
        let id = DIALOG_ANSWERS.iter().find(|id| kb.matches(data, id))?;
        self.letter_answer_for(id)
    }

    /// The first option the dialog answer `id` names.
    fn letter_answer_for(&self, id: &str) -> Option<usize> {
        let words: &[&str] = match id {
            "app.dialog.yes" => &YES_WORDS,
            "app.dialog.no" => &NO_WORDS,
            _ => &ALWAYS_WORDS,
        };
        self.options.iter().position(|option| {
            words
                .iter()
                .any(|word| option == word || option.starts_with(word))
        })
    }

    /// The keys under the options (`?`): the moves, the answers offered, and the exits.
    fn help_rows(&self) -> Vec<Line<'static>> {
        let mut pairs: Vec<(String, String)> = vec![
            (
                format!("{}/{}", key_text("app.list.up"), key_text("app.list.down")),
                "move".to_string(),
            ),
            (key_text("tui.select.confirm"), "select".to_string()),
        ];
        for (id, label) in [
            ("app.dialog.yes", "yes"),
            ("app.dialog.no", "no"),
            ("app.dialog.always", "always"),
        ] {
            if self.letter_answer_for(id).is_some() {
                pairs.push((key_text(id), label.to_string()));
            }
        }
        pairs.push((
            format!(
                "{}/{}",
                key_text("tui.select.cancel"),
                key_text("app.list.close")
            ),
            "close".to_string(),
        ));
        pairs.push((key_text("app.list.help"), "hide keys".to_string()));
        let refs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(k, a)| (k.as_str(), a.as_str()))
            .collect();
        help_rows(&refs)
    }

    /// The console name: the title, with the seconds left while timed.
    fn title(&self) -> String {
        match &self.countdown {
            Some(c) => format!("{} ({}s)", self.base_title, c.remaining_seconds()),
            None => self.base_title.clone(),
        }
    }

    /// The countdown's next tick, if it runs.
    pub fn deadline(&self) -> Option<Instant> {
        self.countdown.as_ref().and_then(CountdownTimer::deadline)
    }

    /// Advance the countdown; true when it ticked (or expired).
    pub fn poll(&mut self) -> bool {
        let Some(countdown) = &mut self.countdown else {
            return false;
        };
        match countdown.poll(Instant::now()) {
            None => false,
            Some(seconds) => {
                if seconds == 0 {
                    self.dispose();
                    (self.on_done)(SelectorOutcome::Cancelled);
                }
                true
            }
        }
    }

    pub fn dispose(&mut self) {
        if let Some(c) = &mut self.countdown {
            c.dispose();
        }
    }
}

impl Component for ExtensionSelectorComponent {
    /// The console: the title (with the countdown), the options with `›`
    /// on the selection band, and the hint foot. Keys are unchanged.
    fn render(&mut self, width: u16) -> Vec<String> {
        let selected = self.cursor.selected();
        let mut console = Console::new(self.title(), "");
        console.rows = self
            .options
            .iter()
            .enumerate()
            .map(|(i, option)| option_row(option, i == selected))
            .collect();
        console.selected = (!self.options.is_empty()).then_some(selected);
        console.max_visible = Some(self.cursor.max_visible());
        let moves = key_pair_hint("tui.select.up", "tui.select.down");
        let confirm = key_hint_text("tui.select.confirm");
        let cancel = key_hint_text("tui.select.cancel");
        console.hint = hints(&[
            (moves.as_str(), "navigate"),
            (confirm.as_str(), "select"),
            (cancel.as_str(), "cancel"),
        ]);
        if self.help {
            console.bottom = self.help_rows();
        }
        console.render(width as usize)
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "app.list.help") {
            self.help = !self.help;
        } else if kb.matches(data, "app.list.up") {
            self.cursor.step(-1);
        } else if kb.matches(data, "app.list.down") {
            self.cursor.step(1);
        } else if !self.cursor.handle_key(data, &kb) {
            if kb.matches(data, "tui.select.confirm") || data == NEWLINE {
                if let Some(selected) = self.options.get(self.cursor.selected()).cloned() {
                    (self.on_done)(SelectorOutcome::Selected(selected));
                }
            } else if kb.matches(data, "tui.select.cancel") || kb.matches(data, "app.list.close") {
                (self.on_done)(SelectorOutcome::Cancelled);
            } else if let Some(index) = self.letter_answer(data, &kb) {
                self.cursor.select(index);
                if let Some(selected) = self.options.get(index).cloned() {
                    (self.on_done)(SelectorOutcome::Selected(selected));
                }
            }
        }
    }
}
