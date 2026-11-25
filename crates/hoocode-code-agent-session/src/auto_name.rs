//! Session naming (RT-39): a cheap model names a session from its
//! conversation. It runs after the first agent run and after each successful
//! compaction (interactive), or once at the end of a headless run. It never
//! replaces a name the user chose, and any failure leaves the session as it was.

use std::sync::atomic::{AtomicBool, Ordering};

use hoocode_agent_types::AgentMessage;
use hoocode_ai_types::{
    AssistantMessage, Content, Context, Message, Model, SimpleStreamOptions, StopReason,
    UserMessage,
};
use hoocode_code_models::ResolvedScoped;
use hoocode_code_session::NameSource;

use crate::session::{AgentSession, AgentSessionEvent};

const NAMING_SYSTEM_PROMPT: &str = "You name coding sessions. Reply with only a name of 2 to 6 words that says what the session is for. If the current name still fits the goal, reply KEEP.";
/// Characters of the conversation's tail sent to the naming model.
const EXCERPT_TAIL_CHARS: usize = 3000;
/// Characters of the first user message kept when the excerpt is cut.
const EXCERPT_HEAD_CHARS: usize = 3000;
const MAX_NAME_CHARS: usize = 60;
const NAMING_MAX_TOKENS: u64 = 32;
/// Bounds a hung request, so the in-flight guard is released.
const NAMING_TIMEOUT_MS: u64 = 60_000;

/// Where the naming pass runs (set per session by the mode that creates it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoNameMode {
    /// Never names (the default; rpc, subagent children, `HOOCODE_DETACHED`).
    #[default]
    Off,
    /// TUI and app-server: after the first run, and after each compaction.
    Interactive,
    /// Print and json: once, by the caller, at the end of the run.
    Headless,
}

/// Per-session naming state: the mode, whether the first-run trigger is still
/// armed, and the single-call guard.
pub(crate) struct AutoNameState {
    pub(crate) mode: AutoNameMode,
    first_run_pending: AtomicBool,
    in_flight: AtomicBool,
}

impl AutoNameState {
    pub(crate) fn new(mode: AutoNameMode) -> Self {
        Self {
            mode,
            first_run_pending: AtomicBool::new(true),
            in_flight: AtomicBool::new(false),
        }
    }

    /// Claims the first-run trigger; true only for the first agent run.
    pub(crate) fn take_first_run(&self) -> bool {
        self.first_run_pending.swap(false, Ordering::SeqCst)
    }

    /// Takes the single in-flight slot; false when a naming call is running.
    fn begin(&self) -> bool {
        !self.in_flight.swap(true, Ordering::SeqCst)
    }

    fn end(&self) {
        self.in_flight.store(false, Ordering::SeqCst);
    }
}

/// The scoped model with the lowest input + output cost among those on the
/// same provider as `current`; `current` when none is.
pub fn pick_naming_model(current: &Model, scoped: &[ResolvedScoped]) -> Model {
    scoped
        .iter()
        .map(|s| &s.model)
        .filter(|m| m.provider == current.provider)
        .min_by(|a, b| (a.cost.input + a.cost.output).total_cmp(&(b.cost.input + b.cost.output)))
        .cloned()
        .unwrap_or_else(|| current.clone())
}

/// The name in a naming reply, or `None` when there is none to apply: an
/// empty reply, `KEEP`, or the current name. Capped at 60 characters.
pub fn parse_name_reply(reply: &str, current: Option<&str>) -> Option<String> {
    let line = reply.lines().map(str::trim).find(|l| !l.is_empty())?;
    let name = strip_wrapping(line);
    if name.is_empty() || name.eq_ignore_ascii_case("KEEP") || current == Some(name) {
        return None;
    }
    let capped: String = name.chars().take(MAX_NAME_CHARS).collect();
    let capped = capped.trim();
    (!capped.is_empty()).then(|| capped.to_string())
}

/// Removes surrounding quotes or backticks and a trailing full stop, until stable.
fn strip_wrapping(line: &str) -> &str {
    let mut s = line;
    loop {
        let next = s
            .trim_matches(|c: char| matches!(c, '"' | '\'' | '`'))
            .trim_end_matches('.')
            .trim();
        if next == s {
            return s;
        }
        s = next;
    }
}

fn assistant_text(blocks: &[Content]) -> String {
    blocks
        .iter()
        .filter_map(|c| match c {
            Content::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

fn last_chars(text: &str, n: usize) -> String {
    let count = text.chars().count();
    if count <= n {
        text.to_string()
    } else {
        text.chars().skip(count - n).collect()
    }
}

/// The first user message plus the tail of the user and assistant text
/// (text parts only). `None` when there is no text to name from.
fn conversation_excerpt(messages: &[AgentMessage]) -> Option<String> {
    let turns: Vec<(bool, String)> = messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::User(u) => {
                Some((true, crate::stats::extract_user_message_text(&u.content)))
            }
            AgentMessage::Assistant(a) => Some((false, assistant_text(&a.content))),
            _ => None,
        })
        .filter(|(_, text)| !text.trim().is_empty())
        .collect();
    let first = turns.iter().find(|(user, _)| *user)?;
    let line = |(user, text): &(bool, String)| {
        let role = if *user { "User" } else { "Assistant" };
        format!("{role}: {text}")
    };
    let transcript = turns.iter().map(line).collect::<Vec<_>>().join("\n\n");
    if transcript.chars().count() <= EXCERPT_TAIL_CHARS {
        return Some(transcript);
    }
    let head = first_chars(&line(first), EXCERPT_HEAD_CHARS);
    Some(format!(
        "{head}\n\n...\n\n{}",
        last_chars(&transcript, EXCERPT_TAIL_CHARS)
    ))
}

impl AgentSession {
    /// One naming pass: asks the naming model for a name and records it. A
    /// no-op when the mode is `Off`, a call is already in flight, or the
    /// user named the session. Failures change nothing and report nothing.
    pub async fn auto_name_now(&self) {
        if self.auto_name_state().mode == AutoNameMode::Off {
            return;
        }
        if !self.auto_name_state().begin() {
            return;
        }
        if let Some(name) = self.propose_session_name().await {
            self.record_auto_name(&name);
        }
        self.auto_name_state().end();
    }

    /// Interactive trigger: names the session off the turn path.
    pub(crate) fn schedule_auto_name(&self) {
        if self.auto_name_state().mode != AutoNameMode::Interactive {
            return;
        }
        let session = self.clone();
        self.spawn(async move { session.auto_name_now().await });
    }

    /// Interactive trigger for the first agent run only.
    pub(crate) fn schedule_auto_name_after_first_run(&self) {
        if self.auto_name_state().take_first_run() {
            self.schedule_auto_name();
        }
    }

    async fn propose_session_name(&self) -> Option<String> {
        let current_model = self.model()?;
        let (current_name, excerpt) = {
            let manager = self.session_manager();
            if manager.session_name_source() == Some(NameSource::User) {
                return None;
            }
            let excerpt = conversation_excerpt(&manager.build_context().messages)?;
            (manager.session_name(), excerpt)
        };
        let scoped = self.resolved_scoped_models();
        let model = pick_naming_model(&current_model, &scoped);
        let (api_key, headers) = self.required_request_auth(&model).ok()?;
        let user_text = format!(
            "Current name: {}\n\nConversation:\n{excerpt}",
            current_name.as_deref().unwrap_or("none")
        );
        let context = Context::new(
            NAMING_SYSTEM_PROMPT.to_string(),
            vec![Message::User(UserMessage {
                content: vec![Content::text(user_text)].into(),
                timestamp: hoocode_ai_types::now_ms(),
            })],
            vec![],
        );
        let options = SimpleStreamOptions {
            max_tokens: Some(NAMING_MAX_TOKENS),
            api_key: Some(api_key),
            headers,
            session_id: Some(self.session_id()),
            timeout_ms: Some(NAMING_TIMEOUT_MS),
            ..Default::default()
        };
        let reply: AssistantMessage = hoocode_ai_registry::complete_simple(model, context, options)
            .await
            .ok()?;
        if matches!(reply.stop_reason, StopReason::Error | StopReason::Aborted) {
            return None;
        }
        parse_name_reply(&assistant_text(&reply.content), current_name.as_deref())
    }

    fn record_auto_name(&self, name: &str) {
        let stored = {
            let mut manager = self.session_manager();
            // The user may have named the session while the call was in flight.
            if manager.session_name_source() == Some(NameSource::User) {
                return;
            }
            manager.append_auto_session_name(name);
            manager.session_name()
        };
        self.emit(AgentSessionEvent::SessionInfoChanged { name: stored });
    }
}
