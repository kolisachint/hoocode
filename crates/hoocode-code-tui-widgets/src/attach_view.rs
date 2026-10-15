//! The attached view's two rows (T3.3). While the screen shows a subagent
//! run's transcript, the header row sits above it (`◂ explore · <description>
//! · <state> · Esc back`), and the prompt's place at the bottom shows a hint:
//! the view is read-only, and Esc goes back to the session. The prompt's
//! stand-in takes the keyboard, so typed text goes nowhere.

use hoocode_code_tui_keybindings::key_display_label;
use hoocode_code_tui_theme::theme;
use hoocode_tui_keys::{get_keybindings, is_key_release};
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_to_width, visible_width};

/// The header row: the run's type, what it was asked, and its state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttachHeader {
    agent_type: String,
    description: String,
    state: String,
}

impl AttachHeader {
    pub fn new(agent_type: &str, description: &str, state: &str) -> Self {
        Self {
            agent_type: agent_type.to_string(),
            description: description.to_string(),
            state: state.to_string(),
        }
    }

    /// Replaces the state word (`running`, `complete`, `interrupted`, ...).
    pub fn set_state(&mut self, state: &str) {
        self.state = state.to_string();
    }

    /// The row's text at `width`, unstyled. The description gives way first,
    /// so the state and `Esc back` stay on screen.
    pub fn text(&self, width: usize) -> String {
        let fixed = format!("◂ {} ·  · {} · Esc back", self.agent_type, self.state);
        let room = width.saturating_sub(visible_width(&fixed));
        let description = truncate_to_width(&self.description, room, "…", false);
        let line = format!(
            "◂ {} · {} · {} · Esc back",
            self.agent_type, description, self.state
        );
        // A row too narrow for the fixed parts still fits the screen.
        truncate_to_width(&line, width, "…", false)
    }
}

impl Component for AttachHeader {
    fn render(&mut self, width: u16) -> Vec<String> {
        let line = self.text(width as usize);
        vec![theme().bold(&line)]
    }
}

/// The prompt's place while attached: a hint, and the keyboard for the view.
#[derive(Debug, Default)]
pub struct AttachPrompt {
    detach: bool,
}

impl AttachPrompt {
    pub fn new() -> Self {
        Self::default()
    }

    /// True once when Esc was pressed (the app goes back to the session).
    pub fn take_detach(&mut self) -> bool {
        std::mem::take(&mut self.detach)
    }

    /// The hint's text. The key is the binding's label, so a rebound key shows.
    pub fn hint() -> String {
        format!(
            "Read-only: a subagent's transcript, so the prompt is off. {} goes back.",
            key_display_label("app.subagent.detach")
        )
    }
}

impl Component for AttachPrompt {
    fn render(&mut self, width: u16) -> Vec<String> {
        let line = truncate_to_width(&Self::hint(), width as usize, "…", false);
        vec![theme().fg("dim", &line)]
    }

    fn handle_input(&mut self, data: &str) {
        if is_key_release(data) {
            return;
        }
        if get_keybindings().matches(data, "app.subagent.detach") {
            self.detach = true;
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }
}
