//! The extension selector dialog: `components/extension-selector.ts` with
//! its countdown timer (`countdown-timer.ts`).
//!
//! Drawn as a ratatui [`Console`](hoocode_code_tui_widgets::console::Console):
//! the title (with the countdown) names the console, the options are its
//! rows, and the hint foot names the keys.

use std::time::{Duration, Instant};

use hoocode_code_tui_widgets::console::{hints, Console};
use hoocode_tui_components::PickerCursor;
use hoocode_tui_keys::get_keybindings;
use hoocode_tui_ratatui::{Line, Palette, Span, Style};
use hoocode_tui_render::Component;

/// Vi-style moves, on top of the `tui.select.up` and `tui.select.down` bindings.
const VI_UP: &str = "k";
const VI_DOWN: &str = "j";
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
        }
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
        console.hint = hints(&[("↑↓", "navigate"), ("enter", "select"), ("esc", "cancel")]);
        console.render(width as usize)
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if data == VI_UP || data == VI_DOWN {
            self.cursor.step(if data == VI_UP { -1 } else { 1 });
        } else if !self.cursor.handle_key(data, &kb) {
            if kb.matches(data, "tui.select.confirm") || data == NEWLINE {
                if let Some(selected) = self.options.get(self.cursor.selected()).cloned() {
                    (self.on_done)(SelectorOutcome::Selected(selected));
                }
            } else if kb.matches(data, "tui.select.cancel") {
                (self.on_done)(SelectorOutcome::Cancelled);
            }
        }
    }
}
