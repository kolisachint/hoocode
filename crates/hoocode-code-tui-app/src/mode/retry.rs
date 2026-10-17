//! Auto-retry in the transcript (U0.1, C18).
//!
//! The session retries a transient failure itself (`AutoRetryStart` /
//! `AutoRetryEnd`). The failed attempts still reach the chat as assistant
//! messages with an error, so the transcript:
//!
//! - keeps the latest failed attempt's row and takes it back out when the
//!   next event says it was retried (`AutoRetryStart`, `AutoRetryEnd`);
//! - shows one notice in the notification band while it retries
//!   (`retrying · attempt 2 of 6 · 529 upstream overloaded`);
//! - ends a streak that gave up in one failed Model block:
//!
//! ```text
//!  ⊗ Model  529 upstream overloaded
//!    gave up after 6 attempts in 6.2s · send again to retry
//! ```
//!
//! `events.rs` calls these from the session events. The methods are on
//! `Transcript` so they can be tested on a bare chat.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hoocode_code_agent_session::format::format_duration_secs;
use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::AssistantMessageComponent;
use hoocode_tui_components::{Spacer, Text};
use hoocode_tui_render::Container;

use super::*;
use crate::footer::sanitize_status_text;

/// The error text the session gives a streak it cancelled (`abort_retry`).
pub(crate) const RETRY_CANCELLED: &str = "Retry cancelled";

/// One retry streak, from its first `AutoRetryStart` to its end.
pub(super) struct RetryStreak {
    started: Instant,
    last_error: String,
}

/// The notification band's line while the session retries.
pub(crate) fn retry_notice_text(attempt: u64, max_attempts: u64, error: &str) -> String {
    format!(
        "retrying · attempt {attempt} of {max_attempts} · {}",
        sanitize_status_text(error)
    )
}

/// The body under the failed Model block: how many attempts, how long.
pub(crate) fn model_error_body(attempts: u64, elapsed: Duration, cancelled: bool) -> String {
    let verb = if cancelled { "stopped" } else { "gave up" };
    let plural = if attempts == 1 { "" } else { "s" };
    format!(
        "{verb} after {attempts} attempt{plural} in {} · send again to retry",
        format_duration_secs(elapsed.as_secs_f64())
    )
}

impl Transcript {
    /// Remembers the row of a failed attempt, so the next retry event can
    /// take it out of the chat.
    pub(super) fn keep_attempt_error(&mut self, row: Rc<RefCell<AssistantMessageComponent>>) {
        self.attempt_error = Some(row);
    }

    /// Takes the last failed attempt's row out of the chat.
    pub(super) fn drop_attempt_error(&mut self) {
        if let Some(row) = self.attempt_error.take() {
            self.chat.borrow_mut().remove_child(&as_component(&row));
            self.assistant_components.retain(|c| !Rc::ptr_eq(c, &row));
        }
    }

    /// `AutoRetryStart`: the failed attempt leaves the chat; the streak
    /// remembers its start and the error it retries.
    pub(super) fn start_retry(&mut self, error: &str) {
        self.drop_attempt_error();
        let streak = self.retry_streak.get_or_insert_with(|| RetryStreak {
            started: Instant::now(),
            last_error: String::new(),
        });
        streak.last_error = sanitize_status_text(error);
    }

    /// `AutoRetryEnd`: the failed attempt leaves the chat. When the streak
    /// gave up or was cancelled, one failed Model block says so.
    pub(super) fn end_retry(&mut self, success: bool, attempts: u64, final_error: Option<&str>) {
        self.drop_attempt_error();
        let streak = self.retry_streak.take();
        if success {
            return;
        }
        let cancelled = final_error == Some(RETRY_CANCELLED);
        let title = match (&streak, final_error) {
            (Some(streak), _) if !streak.last_error.is_empty() => streak.last_error.clone(),
            (_, Some(error)) if !cancelled => sanitize_status_text(error),
            _ => "Error".to_string(),
        };
        let elapsed = streak.map_or(Duration::ZERO, |s| s.started.elapsed());
        self.add_model_error(&title, &model_error_body(attempts, elapsed, cancelled));
    }

    /// The failed Model block: a spacer, the `⊗ Model` header in the error
    /// colour with its message, and the muted body under the header.
    fn add_model_error(&mut self, title: &str, body: &str) {
        let t = theme();
        let mut block = Container::new();
        block.add_child(as_component(&Rc::new(RefCell::new(Text::new(
            format!(
                "{} {}  {}",
                t.fg("error", "⊗"),
                t.bold(&t.fg("toolTitle", "Model")),
                t.fg("error", title)
            ),
            1,
            0,
        )))));
        block.add_child(as_component(&Rc::new(RefCell::new(Text::new(
            t.fg("muted", &format!("  {body}")),
            1,
            0,
        )))));
        self.chat
            .borrow_mut()
            .add_child(as_component(&Rc::new(RefCell::new(Spacer::new(1)))));
        self.chat
            .borrow_mut()
            .add_child(as_component(&Rc::new(RefCell::new(block))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_ai_types::{AssistantMessage, StopReason};
    use hoocode_code_tui_widgets::ThinkingDisplay;
    use hoocode_tui_util::strip_vt_control_characters;

    /// The chat as plain text, 100 columns.
    fn chat_text(chat: &Rc<RefCell<Container>>) -> String {
        let mut lines = Vec::new();
        for child in chat.borrow().children.iter() {
            lines.extend(child.borrow_mut().render(100));
        }
        strip_vt_control_characters(&lines.join("\n"))
    }

    /// One failed attempt as the mode builds it: a row in the chat, kept as
    /// the attempt's error row.
    fn failed_attempt(t: &mut Transcript, message: &str) {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let assistant = AssistantMessage {
            stop_reason: StopReason::Error,
            error_message: Some(message.into()),
            ..Default::default()
        };
        let row = Rc::new(RefCell::new(AssistantMessageComponent::new(
            Some(&assistant),
            ThinkingDisplay::Full,
        )));
        t.chat.borrow_mut().add_child(as_component(&row));
        t.assistant_components.push(row.clone());
        t.keep_attempt_error(row);
    }

    fn fresh() -> Transcript {
        Transcript::new(Rc::new(RefCell::new(Container::new())))
    }

    #[test]
    fn a_streak_that_gives_up_leaves_one_model_block_and_no_error_rows() {
        let mut t = fresh();
        for attempt in 1..=6 {
            failed_attempt(&mut t, "529 upstream overloaded");
            if attempt <= 5 {
                t.start_retry("529 upstream overloaded");
            } else {
                t.end_retry(false, 6, Some("529 upstream overloaded"));
            }
        }
        let text = chat_text(&t.chat);
        assert_eq!(text.matches("⊗ Model").count(), 1, "{text}");
        assert_eq!(text.matches("529 upstream overloaded").count(), 1, "{text}");
        assert!(!text.contains("Error: 529"), "{text}");
        assert!(text.contains("gave up after 6 attempts in"), "{text}");
        assert!(text.contains("· send again to retry"), "{text}");
        assert!(t.attempt_error.is_none());
        assert!(t.assistant_components.is_empty());
    }

    #[test]
    fn a_retry_that_succeeds_leaves_no_error_rows_and_no_model_block() {
        let mut t = fresh();
        failed_attempt(&mut t, "529 upstream overloaded");
        t.start_retry("529 upstream overloaded");
        failed_attempt(&mut t, "529 upstream overloaded");
        t.start_retry("529 upstream overloaded");
        // The answer that follows is an ordinary message; the streak ends.
        t.end_retry(true, 2, None);
        let text = chat_text(&t.chat);
        assert!(!text.contains("Model"), "{text}");
        assert!(!text.contains("529"), "{text}");
    }

    #[test]
    fn a_cancelled_streak_says_stopped_not_gave_up() {
        let mut t = fresh();
        failed_attempt(&mut t, "503 service unavailable");
        t.start_retry("503 service unavailable");
        t.end_retry(false, 1, Some(RETRY_CANCELLED));
        let text = chat_text(&t.chat);
        assert_eq!(text.matches("⊗ Model").count(), 1, "{text}");
        assert!(text.contains("503 service unavailable"), "{text}");
        assert!(text.contains("stopped after 1 attempt in"), "{text}");
    }

    #[test]
    fn the_notice_names_the_attempt_and_the_error_on_one_line() {
        assert_eq!(
            retry_notice_text(2, 6, "529 upstream\noverloaded"),
            "retrying · attempt 2 of 6 · 529 upstream overloaded"
        );
    }

    #[test]
    fn the_body_counts_attempts_and_reads_the_elapsed_time() {
        assert_eq!(
            model_error_body(6, Duration::from_millis(6_200), false),
            "gave up after 6 attempts in 6.2s · send again to retry"
        );
        assert_eq!(
            model_error_body(1, Duration::from_millis(900), true),
            "stopped after 1 attempt in 0.9s · send again to retry"
        );
    }
}
