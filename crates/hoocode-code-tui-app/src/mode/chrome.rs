//! Chrome around the transcript: the footer, the terminal title, the session chip and colour, and the
//! editor border and prompt prefix.

use std::rc::Rc;

use hoocode_code_agent_session::AgentSession;
use hoocode_code_paths::APP_TITLE;
use hoocode_code_session::identity::{
    cycle_session_color_slot, parse_session_color_slot, session_color_name,
    session_color_name_list, CycleDirection as SessionCycleDirection, SESSION_COLOR_SLOTS,
};
use hoocode_code_settings::ChromeDensity;
use hoocode_code_tui_selectors::small_selectors::session_color_selector;
use hoocode_code_tui_theme::theme;
use hoocode_tui_components::{SelectItem, Spacer, Text};

use crate::footer::{FooterModel, FooterSource};
use crate::session_chip::render_session_chip;

use super::*;

/// The footer's view of the session.
pub(super) struct SessionFooter {
    pub(super) session: AgentSession,
    pub(super) is_oauth: Rc<dyn Fn(&str) -> bool>,
}

impl FooterSource for SessionFooter {
    fn usage_totals(&self) -> (u64, u64, u64, u64, f64) {
        let manager = self.session.session_manager();
        let totals = hoocode_code_agent_session::stats::sum_assistant_usage(manager.entries());
        (
            totals.input,
            totals.output,
            totals.cache_read,
            totals.cache_write,
            totals.cost,
        )
    }

    fn context_usage(&self) -> Option<(u64, Option<f64>)> {
        self.session
            .get_context_usage()
            .map(|u| (u.context_window, u.percent))
    }

    fn model(&self) -> Option<FooterModel> {
        self.session.model().map(|m| FooterModel {
            id: m.id.clone(),
            provider: m.provider.to_string(),
            context_window: m.context_window,
            reasoning: m.reasoning,
        })
    }

    fn thinking_level(&self) -> String {
        self.session.thinking_level().as_str().to_string()
    }

    fn cwd(&self) -> String {
        self.session.cwd().to_string_lossy().into_owned()
    }

    fn display_name(&self) -> String {
        self.session.display_name()
    }

    fn reserve_tokens(&self) -> u64 {
        self.session.settings().compaction_reserve_tokens()
    }

    fn is_using_oauth(&self) -> bool {
        self.session
            .model()
            .is_some_and(|m| (self.is_oauth)(&m.provider.to_string()))
    }
}

impl Mode {
    pub(super) fn update_editor_border_color(&mut self) {
        self.editor.borrow_mut().editor.border_color = if self.is_bash_mode {
            Box::new(|s: &str| theme().bash_mode_border(s))
        } else {
            let level = thinking_border_level(self.session.thinking_level().as_str());
            Box::new(move |s: &str| theme().thinking_border(level, s))
        };
        self.dirty.set(true);
    }

    /// `updateEditorPromptPrefix`: `!` in the bash-mode colour, else `❯`.
    pub(super) fn update_editor_prompt_prefix(&mut self) {
        let mut editor = self.editor.borrow_mut();
        if self.is_bash_mode {
            editor.editor.prompt_prefix = "!".into();
            editor.editor.prompt_color = Box::new(|s: &str| theme().bash_mode_border(s));
        } else {
            editor.editor.prompt_prefix = "❯".into();
            editor.editor.prompt_color = Box::new(|s: &str| s.to_string());
        }
        self.dirty.set(true);
    }

    pub(super) fn update_session_chip(&mut self) {
        let chip = render_session_chip(
            &self.session.display_name(),
            self.session.session_color_slot() as i64,
        );
        let shown = chip.is_some();
        self.editor.borrow_mut().editor.top_border_label = chip;
        self.footer.borrow_mut().set_session_chip_shown(shown);
        self.dirty.set(true);
    }

    pub(super) fn update_terminal_title(&mut self) {
        let cwd = self.session.cwd().to_path_buf();
        let base = cwd.file_name().map_or_else(
            || cwd.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        let title = match self.session.session_name() {
            Some(name) => format!("{APP_TITLE} - {name} - {base}"),
            None => format!("{APP_TITLE} - {base}"),
        };
        self.tui.terminal.set_title(&title);
    }

    /// The session chip, or the plain display name without a theme slot.
    pub(super) fn current_chip(&self) -> String {
        render_session_chip(
            &self.session.display_name(),
            self.session.session_color_slot() as i64,
        )
        .map(|chip| chip.styled)
        .unwrap_or_else(|| self.session.display_name())
    }

    /// `handleColor`: `/color <slot|name>`; false for a bare `/color`.
    pub(super) fn handle_color_command(&mut self, text: &str) -> bool {
        let arg = text.strip_prefix("/color").unwrap_or(text).trim();
        if arg.is_empty() {
            return false;
        }
        let Some(slot) = parse_session_color_slot(arg) else {
            self.show_warning(&format!(
                "Usage: /color <1-{SESSION_COLOR_SLOTS}> or /color <{}> (first letter works too), or /color on its own to pick one",
                session_color_name_list().join("|")
            ));
            return true;
        };
        self.session.set_session_color(slot);
        let t = theme();
        let name = session_color_name(slot)
            .map(|name| format!("  {}", t.fg("dim", name)))
            .unwrap_or_default();
        let line = format!(
            "{} {}{name}",
            t.fg("dim", "Session color set:"),
            self.current_chip()
        );
        self.add_to_chat(as_component(&handle(Spacer::new(1))));
        self.add_to_chat(as_component(&handle(Text::new(line, 1, 0))));
        true
    }

    /// `cycleSessionColor`: one step on the colour dial.
    pub(super) fn cycle_session_color(&mut self, forward: bool) {
        let direction = if forward {
            SessionCycleDirection::Forward
        } else {
            SessionCycleDirection::Backward
        };
        let slot =
            cycle_session_color_slot(f64::from(self.session.session_color_slot()), direction);
        self.session.set_session_color(slot);
        let name = session_color_name(slot).map_or_else(|| slot.to_string(), str::to_string);
        self.show_dial_step(
            "app.session.color.cycleBackward",
            &format!("Session color: {name}"),
        );
    }

    /// `showSessionColorSelector`: swatches in the prompt's slot; moving
    /// through them repaints the live chip.
    pub(super) fn show_session_color_selector(&mut self) {
        let original = self.session.session_color_slot();
        let selector = session_color_selector(&self.session.display_name(), original);
        {
            let list = selector.select_list();
            let mut list = list.borrow_mut();
            let slot_of = |item: &SelectItem| item.value.parse::<u8>().ok();
            let sink = self.actions.clone();
            list.on_selection_change = Some(Box::new(move |item| {
                if let Some(slot) = slot_of(item) {
                    sink.borrow_mut().push(Action::SessionColorPreview(slot));
                }
            }));
            let sink = self.actions.clone();
            list.on_select = Some(Box::new(move |item| {
                sink.borrow_mut()
                    .push(Action::SessionColorDone(slot_of(item)));
            }));
            let sink = self.actions.clone();
            list.on_cancel = Some(Box::new(move || {
                sink.borrow_mut().push(Action::SessionColorDone(None));
            }));
        }
        self.show_in_editor_slot(as_component(&handle(selector)));
    }

    /// Paint the prompt's chip in `slot` without saving it.
    pub(super) fn preview_session_color(&mut self, slot: u8) {
        let chip = render_session_chip(&self.session.display_name(), i64::from(slot));
        self.editor.borrow_mut().editor.top_border_label = chip;
        self.dirty.set(true);
    }

    /// The colour picker closed: save the choice, or put back the colour
    /// the session actually has.
    pub(super) fn close_session_color_selector(&mut self, slot: Option<u8>) {
        self.restore_editor();
        match slot {
            Some(slot) => {
                self.session.set_session_color(slot);
                let name =
                    session_color_name(slot).map_or_else(|| slot.to_string(), str::to_string);
                self.show_status(&format!("Session color: {name}"));
            }
            None => self.update_session_chip(),
        }
    }

    /// `handleChromeCommand`: `/chrome` names the stop, `/chrome <stop>`
    /// sets it (the dial's only way in where alt never arrives).
    pub(super) fn handle_chrome_command(&mut self, text: &str) {
        let argument = text
            .strip_prefix("/chrome")
            .unwrap_or(text)
            .trim()
            .to_lowercase();
        let all: Vec<&str> = ChromeDensity::ALL.iter().map(|d| d.as_str()).collect();
        if argument.is_empty() {
            self.show_status(&format!(
                "Chrome: {} — {}",
                self.chrome.density(),
                all.join(" · ")
            ));
            return;
        }
        let Some(density) = ChromeDensity::parse(&argument) else {
            self.show_error(&format!(
                "Unknown chrome density \"{argument}\". Try: {}",
                all.join(", ")
            ));
            return;
        };
        self.chrome.set_density(density);
        self.session.settings().set_chrome_density(density);
        self.dirty.set(true);
        self.show_status(&format!("Chrome: {density}"));
    }
}
