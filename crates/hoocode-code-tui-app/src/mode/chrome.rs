//! The rows around the transcript: the status line's data, the terminal
//! title, and the prompt's bash-mode marker.

use hoocode_code_paths::APP_TITLE;
use hoocode_code_tui_widgets::trajectory::MissionHeader;

use crate::status_line::StatusLineData;

use super::*;

/// Whether a transcript child stands for a turn: something the user said
/// (a message, a mission header, a skill run, a background notice). The
/// start screen ends with the first one; the scroll view jumps between them.
pub(super) fn is_turn_row(child: &ComponentHandle) -> bool {
    child.try_borrow().is_ok_and(|child| {
        child.as_any().is_some_and(|any| {
            any.is::<UserMessageComponent>()
                || any.is::<MissionHeader>()
                || any.is::<SkillBlockComponent>()
                || any.is::<BackgroundNoticeComponent>()
        })
    })
}

impl Mode {
    /// `updateEditorPromptPrefix`: the prompt's marker is `!` in the
    /// bash-mode colour while the input is a `!` command, else ` ›`.
    pub(super) fn update_editor_prompt_prefix(&mut self) {
        self.bash_marker.set(self.is_bash_mode);
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

    /// What the status line shows now, read from the session and the mode.
    pub(super) fn status_line_data(&self) -> StatusLineData {
        let session = &self.session;
        let model = session.model();
        let signed_out = model.is_none();
        let busy = session.is_streaming() || session.is_retrying() || self.prompt_pending;
        // A permission or a question names what it needs in its own yellow
        // title: the left slot stays empty while one is open.
        let asking = self.clearance.is_some() || self.ask_options.is_some();
        let working = (busy && !asking).then(|| {
            self.turn_timer
                .borrow()
                .elapsed(Instant::now())
                .unwrap_or_default()
        });
        let effort = model
            .as_ref()
            .filter(|m| m.reasoning)
            .map(|_| session.thinking_level().as_str().to_string())
            .filter(|level| level != "off");
        let cost = hoocode_code_agent_session::stats::sum_assistant_usage(
            session.session_manager().entries(),
        )
        .cost;
        // Persistent rows: extra usage while the model bills through Anthropic
        // subscription auth (the setting turns it off), skip permissions
        // while the flag is on. Both yield to a live notice and to asking.
        let extra_usage = !asking
            && model
                .as_ref()
                .is_some_and(|m| session.uses_anthropic_subscription_auth(m));
        let skip_permissions = !asking && self.footer_data.get_skip_permissions();
        StatusLineData {
            working,
            notice: if asking {
                None
            } else {
                self.notifications.borrow().status_notice()
            },
            scroll: None,
            search: None,
            last_turn: if asking || busy {
                None
            } else {
                self.last_turn.clone()
            },
            signed_out,
            extra_usage,
            skip_permissions,
            plan_mode: self.footer_data.get_active_mode() == "plan",
            session: Some(session.display_name()).filter(|name| !name.is_empty()),
            model: model.map(|m| m.id.clone()),
            effort,
            context_percent: session.get_context_usage().and_then(|u| u.percent),
            cost: Some(cost),
        }
    }

    /// Refreshes the status line's data before the frame; a change repaints.
    pub(super) fn refresh_status_line(&mut self) {
        let busy = self.session.is_streaming() || self.session.is_retrying() || self.prompt_pending;
        self.notifications.borrow_mut().set_status_slot_free(!busy);
        let data = self.status_line_data();
        let mut current = self.status_data.borrow_mut();
        if *current != data {
            *current = data;
            self.dirty.set(true);
        }
    }
}
