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

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{AssistantMessage, StopReason};
use hoocode_code_agent_session::format::format_duration_secs;
use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::tool_chain::ToolBlock;
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

/// Whether the errored reply at `i` was retried: the retry's reply follows it.
pub(crate) fn attempt_was_retried(messages: &[AgentMessage], i: usize) -> bool {
    matches!(messages.get(i + 1), Some(AgentMessage::Assistant(_)))
}

/// The failed replies right before message `i`, oldest first.
pub(crate) fn failed_attempts_before(
    messages: &[AgentMessage],
    i: usize,
) -> Vec<&AssistantMessage> {
    let mut failed: Vec<&AssistantMessage> = messages[..i]
        .iter()
        .rev()
        .map_while(|m| match m {
            AgentMessage::Assistant(a) if a.stop_reason == StopReason::Error => Some(a),
            _ => None,
        })
        .collect();
    failed.reverse();
    failed
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
    /// Remembers the row and tool blocks of a failed attempt, so the next
    /// retry event can take them out of the chat.
    pub(super) fn keep_attempt_error(
        &mut self,
        row: Rc<RefCell<AssistantMessageComponent>>,
        tools: Vec<ToolBlock>,
    ) {
        self.attempt_error = Some(row);
        self.attempt_tools = tools;
    }

    /// Takes the last failed attempt's row and tool blocks out of the chat.
    pub(super) fn drop_attempt_error(&mut self) {
        if let Some(row) = self.attempt_error.take() {
            self.chat.borrow_mut().remove_child(&as_component(&row));
            self.assistant_components.retain(|c| !Rc::ptr_eq(c, &row));
        }
        for block in std::mem::take(&mut self.attempt_tools) {
            self.remove_tool_block(&block);
        }
    }

    /// Takes one tool block out of its chain; a chain left empty leaves the chat.
    fn remove_tool_block(&mut self, block: &ToolBlock) {
        for chain in self.chains.clone() {
            if !chain
                .borrow()
                .tool_blocks()
                .iter()
                .any(|b| Rc::ptr_eq(b, block))
            {
                continue;
            }
            chain.borrow_mut().remove(block);
            if chain.borrow().is_empty() {
                self.chat.borrow_mut().remove_child(&as_component(&chain));
                self.chains.retain(|c| !Rc::ptr_eq(c, &chain));
                if self
                    .open_chain
                    .as_ref()
                    .is_some_and(|c| Rc::ptr_eq(c, &chain))
                {
                    self.open_chain = None;
                }
                if self
                    .latest_chain
                    .as_ref()
                    .is_some_and(|c| Rc::ptr_eq(c, &chain))
                {
                    self.latest_chain = None;
                }
            }
        }
        if self
            .latest_block
            .as_ref()
            .is_some_and(|b| Rc::ptr_eq(b, block))
        {
            self.latest_block = None;
        }
    }

    /// The retry streak and its failed attempt are gone: the session they
    /// belong to was switched away from.
    pub(super) fn clear_retry(&mut self) {
        self.drop_attempt_error();
        self.retry_streak = None;
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
    pub(super) fn add_model_error(&mut self, title: &str, body: &str) {
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
    use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
    use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
    use hoocode_code_tui_widgets::tool_output_view::DEFAULT_TOOL_OUTPUT_VIEW;
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

    fn error_row(message: &str) -> Rc<RefCell<AssistantMessageComponent>> {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let assistant = AssistantMessage {
            stop_reason: StopReason::Error,
            error_message: Some(message.into()),
            ..Default::default()
        };
        Rc::new(RefCell::new(AssistantMessageComponent::new(
            Some(&assistant),
            ThinkingDisplay::Full,
        )))
    }

    /// One failed attempt as the mode builds it: a row in the chat, kept as
    /// the attempt's error row.
    fn failed_attempt(t: &mut Transcript, message: &str) {
        let row = error_row(message);
        t.chat.borrow_mut().add_child(as_component(&row));
        t.assistant_components.push(row.clone());
        t.keep_attempt_error(row, Vec::new());
    }

    /// A failed attempt whose call was streamed into a tool block of its own.
    fn failed_attempt_with_tool(t: &mut Transcript, message: &str) -> ToolBlock {
        failed_attempt(t, message);
        let block = Rc::new(RefCell::new(ToolExecutionComponent::new(
            "bash",
            "call-1",
            serde_json::json!({"command": "ls"}),
            ToolExecutionOptions {
                view: DEFAULT_TOOL_OUTPUT_VIEW,
                ..Default::default()
            },
            None,
            "/",
        )));
        let chain = Rc::new(RefCell::new(ToolChainComponent::new(
            DEFAULT_TOOL_OUTPUT_VIEW,
        )));
        chain.borrow_mut().add(block.clone());
        t.chat.borrow_mut().add_child(as_component(&chain));
        t.chains.push(chain.clone());
        t.open_chain = Some(chain);
        if let Some(row) = t.attempt_error.clone() {
            let tools = vec![block.clone()];
            t.keep_attempt_error(row, tools);
        }
        block
    }

    fn fresh() -> Transcript {
        Transcript::new(Rc::new(RefCell::new(Container::new())))
    }

    fn assistant(stop_reason: StopReason, timestamp: i64) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            stop_reason,
            timestamp,
            ..Default::default()
        })
    }

    #[test]
    fn a_streak_that_gives_up_leaves_one_model_block_and_no_error_rows() {
        let mut t = fresh();
        // The original request fails, then each of the six retries fails in
        // turn: every start takes the failed row out, the last failure ends it.
        failed_attempt(&mut t, "529 upstream overloaded");
        for _ in 1..=6 {
            t.start_retry("529 upstream overloaded");
            failed_attempt(&mut t, "529 upstream overloaded");
        }
        t.end_retry(false, 7, Some("529 upstream overloaded"));
        let text = chat_text(&t.chat);
        assert_eq!(text.matches("⊗ Model").count(), 1, "{text}");
        assert_eq!(text.matches("529 upstream overloaded").count(), 1, "{text}");
        assert!(!text.contains("Error: 529"), "{text}");
        assert!(text.contains("gave up after 7 attempts in"), "{text}");
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
    fn a_retried_attempt_takes_its_tool_blocks_out_with_its_row() {
        let mut t = fresh();
        failed_attempt_with_tool(&mut t, "529 upstream overloaded");
        assert_eq!(t.chains.len(), 1);
        t.start_retry("529 upstream overloaded");
        assert!(t.chains.is_empty());
        assert!(t.open_chain.is_none());
        assert!(t.attempt_tools.is_empty());
        assert!(t.chat.borrow().children.is_empty());
    }

    #[test]
    fn a_tool_chain_keeps_the_calls_that_were_not_retried() {
        let mut t = fresh();
        let block = failed_attempt_with_tool(&mut t, "529 upstream overloaded");
        let other = Rc::new(RefCell::new(ToolExecutionComponent::new(
            "read",
            "call-2",
            serde_json::json!({"path": "a"}),
            ToolExecutionOptions::default(),
            None,
            "/",
        )));
        t.chains[0].borrow_mut().add(other.clone());
        t.start_retry("529 upstream overloaded");
        let blocks = t.chains[0].borrow().tool_blocks().len();
        assert_eq!(blocks, 1);
        assert!(!Rc::ptr_eq(&t.chains[0].borrow().tool_blocks()[0], &block));
    }

    #[test]
    fn clearing_the_retry_drops_the_streak_and_its_failed_row() {
        let mut t = fresh();
        failed_attempt(&mut t, "529 upstream overloaded");
        t.start_retry("529 upstream overloaded");
        t.clear_retry();
        assert!(t.retry_streak.is_none());
        assert!(t.attempt_error.is_none());
    }

    #[test]
    fn a_view_reset_keeps_the_retry_streak_in_progress() {
        let mut t = fresh();
        failed_attempt(&mut t, "529 upstream overloaded");
        t.start_retry("529 upstream overloaded");
        t.reset_view();
        assert!(t.retry_streak.is_some());
        assert!(t.attempt_error.is_none());
    }

    #[test]
    fn a_replayed_reply_is_retried_when_a_reply_follows_it() {
        let messages = [
            assistant(StopReason::Error, 1),
            assistant(StopReason::Error, 2),
            assistant(StopReason::Stop, 3),
        ];
        assert!(attempt_was_retried(&messages, 0));
        assert!(attempt_was_retried(&messages, 1));
        assert!(failed_attempts_before(&messages, 0).is_empty());
        assert_eq!(failed_attempts_before(&messages, 1).len(), 1);
    }

    #[test]
    fn a_replayed_final_failure_follows_the_failed_replies_before_it() {
        let messages = [
            assistant(StopReason::Error, 1),
            assistant(StopReason::Error, 2),
            assistant(StopReason::Error, 3),
        ];
        assert!(!attempt_was_retried(&messages, 2));
        let failed = failed_attempts_before(&messages, 2);
        assert_eq!(failed.len(), 2);
        assert_eq!(failed[0].timestamp, 1);
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
