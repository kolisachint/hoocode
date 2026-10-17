//! Chrome around the transcript: the footer, the terminal title, the session chip and colour, and the
//! editor border and prompt prefix.

use std::rc::Rc;

use hoocode_code_agent_session::mcp::McpStatus;
use hoocode_code_agent_session::AgentSession;
use hoocode_code_paths::APP_TITLE;
use hoocode_code_session::identity::{
    cycle_session_color_slot, session_color_name, CycleDirection as SessionCycleDirection,
};
use hoocode_code_settings::ChromeDensity;
use hoocode_code_tui_theme::theme;

use crate::footer::{FooterModel, FooterSource};
use crate::hull_paint::HullPaint;
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

    fn satellite_count(&self) -> usize {
        // TODO(M3): confirm the satellite count counts connected servers only.
        self.session.mcp().map_or(0, |hub| {
            hub.servers()
                .iter()
                .filter(|s| s.status == McpStatus::Connected)
                .count()
        })
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
            // The comms frame colours the glyph per state on each render.
            editor.editor.prompt_prefix = "▸".into();
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
        // The tab title takes the hull paint too (M8e5).
        let title = match self.footer_data.get_hull_paint() {
            Some(paint) => format!("{title} - {}", paint.name()),
            None => title,
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

    /// `handleColor`: bare `/color` cycles the hull paint (M8e5);
    /// `/color <name>` sets it outright, with the old session-chip colour
    /// names mapped to the nearest paint. A toast shows the name.
    pub(super) fn handle_color_command(&mut self, text: &str) -> bool {
        let arg = text.strip_prefix("/color").unwrap_or(text).trim();
        if arg.is_empty() {
            let next = HullPaint::next(self.footer_data.get_hull_paint());
            self.set_hull_paint(next);
            return true;
        }
        match HullPaint::parse(arg) {
            Ok(paint) => {
                self.set_hull_paint(paint);
            }
            Err(_) => {
                self.show_warning(
                    "Usage: /color [none|parity|cyan|violet|lime|red] (old chip names still work)",
                );
            }
        }
        true
    }

    /// The hull paint, shown as a toast by name (M8e5). The session file
    /// keeps it, so a resume restores it; the provider feeds the chrome.
    fn set_hull_paint(&mut self, paint: Option<HullPaint>) {
        self.session.set_hull_paint(paint.map(HullPaint::name));
        self.footer_data.set_hull_paint(paint);
        self.notify(
            NotificationKind::Info,
            paint.map_or("none", HullPaint::name),
        );
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

    /// `handleChromeCommand`: `/chrome` names the stop, `/chrome <full|compact>`
    /// sets it (the dial's only way in where alt never arrives).
    pub(super) fn handle_chrome_command(&mut self, text: &str) {
        let argument = text
            .strip_prefix("/chrome")
            .unwrap_or(text)
            .trim()
            .to_lowercase();
        if argument.is_empty() {
            self.show_status(&format!(
                "Chrome: {} — {}",
                self.chrome.density(),
                chrome_stops()
            ));
            return;
        }
        let density = match parse_chrome_stop(&argument) {
            Ok(density) => density,
            Err(message) => {
                self.show_error(&message);
                return;
            }
        };
        self.chrome.set_density(density);
        self.session.settings().set_chrome_density(density);
        self.dirty.set(true);
        self.show_status(&format!("Chrome: {density}"));
    }
}

/// The valid stops, as the user types them: `full · compact`.
fn chrome_stops() -> String {
    let all: Vec<&str> = ChromeDensity::ALL.iter().map(|d| d.as_str()).collect();
    all.join(" · ")
}

/// The argument of `/chrome`, parsed. `bare` was a stop until it was retired:
/// it gets its own message, and the message names the two stops that are left.
fn parse_chrome_stop(argument: &str) -> Result<ChromeDensity, String> {
    if let Some(density) = ChromeDensity::parse(argument) {
        return Ok(density);
    }
    let all: Vec<&str> = ChromeDensity::ALL.iter().map(|d| d.as_str()).collect();
    if argument == "bare" {
        return Err(format!(
            "\"bare\" is no longer a chrome stop. Use: {}",
            all.join(", ")
        ));
    }
    Err(format!(
        "Unknown chrome stop \"{argument}\". Use: {}",
        all.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_two_stops() {
        assert_eq!(parse_chrome_stop("full"), Ok(ChromeDensity::Full));
        assert_eq!(parse_chrome_stop("compact"), Ok(ChromeDensity::Compact));
    }

    #[test]
    fn bare_is_refused_and_the_error_names_the_two_stops() {
        let err = parse_chrome_stop("bare").unwrap_err();
        assert_eq!(
            err,
            "\"bare\" is no longer a chrome stop. Use: full, compact"
        );
    }

    #[test]
    fn anything_else_is_an_unknown_stop_naming_the_two_stops() {
        let err = parse_chrome_stop("nope").unwrap_err();
        assert_eq!(err, "Unknown chrome stop \"nope\". Use: full, compact");
        assert_eq!(chrome_stops(), "full · compact");
    }
}
