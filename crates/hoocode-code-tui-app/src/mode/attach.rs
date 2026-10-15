//! The attached view (T3.3, T3.4): one subagent run's transcript on the whole
//! chat area, read-only, with the session's own transcript parked.
//!
//! The chat container is shared. Two halves of a view exist: the one on screen
//! and the parked one. `Attached::main_in` says which half is on screen, and
//! `flip_view` swaps them (the `Transcript` value and the chat container's
//! children). The invariant: outside a short window, the session is on screen.
//! The windows are a child event (`apply_attached_event`), a key to the screen
//! (`accept_terminal_event`) and the render at the end of the loop. So the
//! session's handlers run unchanged, against the transcript they always wrote.
//!
//! The child's events are its stdout lines (`AppEvent::Subagent`). They are
//! not `AgentEvent`s: a `message_update` is a text delta, so the child handler
//! below reads the few shapes it needs and calls the same tool-block code.
//!
//! Disk work (finding the run, reading its transcript) runs on the tools pool
//! and reports back as `AppEvent::AttachLoaded` / `AgentRunsListed`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use hoocode_agent_types::{AgentEvent, AgentMessage, AgentToolResult};
use hoocode_ai_types::{AssistantMessage, Content};
use hoocode_code_session::{
    build_session_info, list_subagent_sessions, load_entries_from_file, FileEntry, SessionInfo,
    SessionManager, SubagentState, SUBAGENT_RUN_ENTRY,
};
use hoocode_code_subagents::live::run_for_tool_call;
use hoocode_code_subagents::pool::run_is_live;
use hoocode_code_tui_widgets::attach_view::{AttachHeader, AttachPrompt};
use hoocode_code_tui_widgets::AssistantMessageComponent;
use hoocode_runtime::run_blocking;
use serde_json::Value;

use super::dialogs::Answer;
use super::*;

/// The description shown for a run: its first line, cut short.
const DESCRIPTION_CHARS: usize = 120;

/// The attached view's state. It lives from attach until detach.
pub(super) struct Attached {
    run_id: String,
    /// The run's type and description, when the caller knew them (an Agent
    /// block's arguments, a store row); else they come from the transcript's record.
    shown: Option<(String, String)>,
    header: Rc<RefCell<AttachHeader>>,
    prompt: Rc<RefCell<AttachPrompt>>,
    /// False until the history is drawn. Child events until then wait in `queued`.
    loaded: bool,
    queued: Vec<Value>,
    /// The text the child has streamed for its current message.
    child_text: String,
    /// True while the session's transcript is in `Mode::transcript`.
    main_in: bool,
    /// The half not on screen: the child's while `main_in`, else the session's.
    parked: Transcript,
    parked_children: Vec<ComponentHandle>,
    /// The session's scroll position, put back on detach.
    main_scroll: Option<i64>,
}

/// A run's transcript and listing row, read off the UI thread.
pub(super) struct LoadedRun {
    agent_type: String,
    description: String,
    state: String,
    messages: Vec<AgentMessage>,
}

/// One row of `/agents`: a run of this session, and its picker label.
pub(super) struct RunRow {
    pub(super) run_id: String,
    pub(super) agent_type: String,
    pub(super) description: String,
    label: String,
}

/// Swaps the shown transcript and its chat children with the parked half.
fn swap_halves(
    shown: &mut Transcript,
    parked: &mut Transcript,
    parked_children: &mut Vec<ComponentHandle>,
) {
    std::mem::swap(shown, parked);
    std::mem::swap(&mut shown.chat.borrow_mut().children, parked_children);
}

/// The run's state as the header and `/agents` word it.
fn state_word(state: &SubagentState) -> String {
    match state {
        SubagentState::Running => "running".into(),
        SubagentState::Finished(status) => status.clone(),
        SubagentState::Interrupted => "interrupted".into(),
    }
}

/// A run's description: the first line of its task, cut short.
fn describe(first_message: &str) -> String {
    first_message
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(DESCRIPTION_CHARS)
        .collect()
}

/// The subagent runs of `parent`, as session rows (each with its `subagent`).
fn session_runs(parent: &Path) -> Result<Vec<SessionInfo>, String> {
    let info = build_session_info(parent).ok_or("This session has no saved transcript.")?;
    Ok(list_subagent_sessions(
        std::slice::from_ref(&info),
        &|task_id, cwd| run_is_live(Path::new(cwd), task_id),
    ))
}

/// Each run's description as its parent's `subagent_run` record has it, by task id.
fn record_descriptions(parent: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for entry in load_entries_from_file(parent) {
        let FileEntry::Custom {
            custom_type,
            data: Some(data),
            ..
        } = entry
        else {
            continue;
        };
        if custom_type != SUBAGENT_RUN_ENTRY {
            continue;
        }
        let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_string);
        if let (Some(task), Some(description)) = (text("taskId"), text("description")) {
            out.insert(task, describe(&description));
        }
    }
    out
}

/// What the attached view needs from disk: the run's row and its messages.
///
/// A run that is live but has no transcript file yet (it has not written its
/// first message) is still attachable: its view starts empty and its events
/// fill it. `shown` is what the caller knew of the run, used for such a run.
fn read_run(
    parent: &Path,
    cwd: &Path,
    run_id: &str,
    shown: Option<(String, String)>,
) -> Result<LoadedRun, String> {
    let listed = session_runs(parent)?.into_iter().find_map(|run| {
        let subagent = run.subagent?;
        (subagent.task_id == run_id).then_some((run.path, run.first_message, subagent))
    });
    let Some((path, first_message, subagent)) = listed else {
        if !run_is_live(cwd, run_id) {
            return Err("That subagent run is not in this session.".into());
        }
        let (agent_type, description) = shown.unwrap_or_else(|| ("subagent".into(), String::new()));
        return Ok(LoadedRun {
            agent_type,
            description,
            state: "running".into(),
            messages: Vec::new(),
        });
    };
    let description = record_descriptions(parent)
        .remove(run_id)
        .unwrap_or_else(|| describe(&first_message));
    let messages = SessionManager::open(path, None, None)
        .build_context()
        .messages;
    Ok(LoadedRun {
        agent_type: subagent.agent_type,
        description,
        state: state_word(&subagent.state),
        messages,
    })
}

/// `/agents`: this session's runs, newest first.
fn list_runs(parent: &Path) -> Result<Vec<RunRow>, String> {
    let mut runs = session_runs(parent)?;
    runs.sort_by_key(|r| std::cmp::Reverse(r.modified));
    let descriptions = record_descriptions(parent);
    Ok(runs
        .into_iter()
        .filter_map(|r| {
            let subagent = r.subagent?;
            let description = descriptions
                .get(&subagent.task_id)
                .cloned()
                .unwrap_or_else(|| describe(&r.first_message));
            let label = format!(
                "{} · {} · {} · {}",
                subagent.agent_type,
                state_word(&subagent.state),
                description,
                subagent.task_id
            );
            Some(RunRow {
                run_id: subagent.task_id,
                agent_type: subagent.agent_type,
                description,
                label,
            })
        })
        .collect())
}

impl Mode {
    /// Whether the attached view shows `run_id`.
    pub(super) fn attached_to(&self, run_id: &str) -> bool {
        self.attached.as_ref().is_some_and(|a| a.run_id == run_id)
    }

    /// The component in the prompt's place: the attached view's hint while
    /// attached, else the prompt.
    pub(super) fn prompt_component(&self) -> ComponentHandle {
        match &self.attached {
            Some(attached) => as_component(&attached.prompt),
            None => as_component(&self.editor),
        }
    }

    /// Runs `job` on the tools pool and posts what `done` makes of its result.
    fn off_ui_thread<T, F, E>(&self, job: F, done: E)
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
        E: FnOnce(T) -> AppEvent + Send + 'static,
    {
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            if let Ok(value) = run_blocking(job).await {
                let _ = tx.send(done(value));
            }
        });
    }

    /// `app.subagent.attach`: attach to the run of the newest Agent block that
    /// has one. The block's arguments name the run's type and description.
    pub(super) fn attach_newest_agent_block(&mut self) {
        let run = self.tool_blocks().iter().rev().find_map(|block| {
            let block = block.borrow();
            if block.tool_name() != "Agent" {
                return None;
            }
            let run = run_for_tool_call(block.tool_call_id())?;
            let args = block.args();
            let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
            let shown = (
                text("subagent_type").to_string(),
                text("description").to_string(),
            );
            Some((run, shown))
        });
        match run {
            Some((run_id, shown)) => self.attach_run(run_id, Some(shown)),
            None => self.show_status("No Agent block with a subagent run to attach to."),
        }
    }

    /// Attaches to `run_id`: the header and the hint go up, the transcript is
    /// read on the tools pool, and the run's events are routed here until
    /// detach. Attaching to another run detaches first. `shown` is the run's
    /// (type, description) when the caller knows them.
    pub(super) fn attach_run(&mut self, run_id: String, shown: Option<(String, String)>) {
        if self.attached_to(&run_id) {
            return;
        }
        self.detach();
        let Some(parent) = self.parent_session_file() else {
            self.show_status("This session has no saved transcript, so it has no subagent runs.");
            return;
        };
        let (agent_type, description) = match &shown {
            Some((agent_type, description)) => (agent_type.clone(), description.clone()),
            None => ("subagent".to_string(), "loading…".to_string()),
        };
        let job_shown = shown.clone();
        let header = Rc::new(RefCell::new(AttachHeader::new(
            &agent_type,
            &description,
            "loading",
        )));
        let prompt = Rc::new(RefCell::new(AttachPrompt::new()));
        let main_scroll = self.tui.get_scroll_position().map(|(top, _, _)| top);
        // The run's view starts at its tail, as a new session does.
        self.tui.scroll_to_live();
        {
            let mut slot = self.attach_slot.borrow_mut();
            slot.clear();
            slot.add_child(as_component(&header));
        }
        {
            let mut slot = self.editor_container.borrow_mut();
            slot.clear();
            slot.add_child(as_component(&prompt));
        }
        self.tui.set_focus(Some(as_component(&prompt)));
        self.attached = Some(Attached {
            run_id: run_id.clone(),
            shown,
            header,
            prompt,
            loaded: false,
            queued: Vec::new(),
            child_text: String::new(),
            main_in: true,
            parked: Transcript::new(self.transcript.chat.clone()),
            parked_children: Vec::new(),
            main_scroll,
        });
        self.dirty.set(true);
        let job_id = run_id.clone();
        let cwd = self.session.cwd().to_path_buf();
        self.off_ui_thread(
            move || read_run(&parent, &cwd, &job_id, job_shown),
            move |result| AppEvent::AttachLoaded(run_id, result),
        );
    }

    /// The run's transcript arrived (or failed). The history is drawn, then the
    /// events that came while it loaded.
    pub(super) fn attach_loaded(&mut self, run_id: String, result: Result<LoadedRun, String>) {
        if !self.attached_to(&run_id) {
            return;
        }
        // A second read (the run settled) replaces the view with the whole transcript.
        let reread = self.attached.as_ref().is_some_and(|a| a.loaded);
        let run = match result {
            Ok(run) => run,
            // A failed re-read keeps the view as it is: the run was attached.
            Err(_) if reread => return,
            Err(message) => {
                self.detach();
                self.show_error(&message);
                return;
            }
        };
        self.show_attached();
        if reread {
            self.reset_transcript_view();
        }
        if let Some(attached) = self.attached.as_mut() {
            // What the caller knew wins; the transcript's record fills the rest.
            let (agent_type, description) = attached
                .shown
                .clone()
                .unwrap_or((run.agent_type.clone(), run.description.clone()));
            *attached.header.borrow_mut() =
                AttachHeader::new(&agent_type, &description, &run.state);
            attached.loaded = true;
        }
        self.render_session_context(&run.messages, false);
        let queued = self
            .attached
            .as_mut()
            .map(|a| std::mem::take(&mut a.queued))
            .unwrap_or_default();
        for event in queued {
            self.apply_child_event(event);
        }
        self.show_main();
        self.dirty.set(true);
    }

    /// Back to the session: its transcript and scroll position come back, the
    /// run's view is dropped, and the prompt returns. A no-op when not attached.
    pub(super) fn detach(&mut self) {
        let Some(mut attached) = self.attached.take() else {
            return;
        };
        if !attached.main_in {
            swap_halves(
                &mut self.transcript,
                &mut attached.parked,
                &mut attached.parked_children,
            );
        }
        self.tui.restore_scroll_top(attached.main_scroll);
        self.attach_slot.borrow_mut().clear();
        self.restore_editor();
        self.dirty.set(true);
    }

    /// Leaves the view when its hint took Esc. Called once per input pass.
    pub(super) fn poll_attached_prompt(&mut self) {
        let leave = self
            .attached
            .as_ref()
            .is_some_and(|a| a.prompt.borrow_mut().take_detach());
        if leave {
            self.detach();
        }
    }

    /// Puts the session's transcript on screen (the default).
    pub(super) fn show_main(&mut self) {
        if self.attached.as_ref().is_some_and(|a| !a.main_in) {
            self.flip_view();
        }
    }

    /// Puts the run's transcript on screen, for a child event or a render.
    pub(super) fn show_attached(&mut self) {
        if self.attached.as_ref().is_some_and(|a| a.main_in) {
            self.flip_view();
        }
    }

    fn flip_view(&mut self) {
        let Some(attached) = self.attached.as_mut() else {
            return;
        };
        swap_halves(
            &mut self.transcript,
            &mut attached.parked,
            &mut attached.parked_children,
        );
        attached.main_in = !attached.main_in;
    }

    /// A live event of the attached run (called with the session on screen).
    /// It is shown now, or queued until the history has been drawn.
    pub(super) fn apply_attached_event(&mut self, event: Value) {
        let Some(attached) = self.attached.as_mut() else {
            return;
        };
        if !attached.loaded {
            attached.queued.push(event);
            return;
        }
        self.show_attached();
        self.apply_child_event(event);
        self.show_main();
    }

    /// One child event, with the run's view on screen.
    fn apply_child_event(&mut self, event: Value) {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match kind.as_str() {
            "message_update" => self.child_text_delta(&event),
            "message_end" => self.child_message_end(&event),
            "tool_execution_start" => self.child_tool_start(&event),
            "tool_execution_end" => self.child_tool_end(&event),
            "task_done" => self.child_settled("complete"),
            "task_failed" => self.child_settled("failed"),
            "task_cancelled" => self.child_settled("cancelled"),
            "task_timeout" => self.child_settled("timeout"),
            _ => {}
        }
        self.dirty.set(true);
    }

    /// A text delta of the child's message: the streaming block grows.
    fn child_text_delta(&mut self, event: &Value) {
        let delta = event
            .pointer("/assistantMessageEvent/delta")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if delta.is_empty() {
            return;
        }
        let Some(attached) = self.attached.as_mut() else {
            return;
        };
        attached.child_text.push_str(delta);
        let assistant = AssistantMessage {
            content: vec![Content::text(attached.child_text.clone())],
            ..Default::default()
        };
        let component = match self.transcript.streaming.clone() {
            Some(component) => component,
            None => {
                if self.opens_new_chain(&assistant) {
                    self.close_open_chain(ChainState::Done);
                }
                let component = handle(AssistantMessageComponent::with_theme(
                    None,
                    self.thinking_display(),
                    self.markdown_theme(),
                    DEFAULT_HIDDEN_THINKING_LABEL,
                ));
                self.add_to_chat(as_component(&component));
                self.transcript.assistant_components.push(component.clone());
                self.transcript.streaming = Some(component.clone());
                component
            }
        };
        component.borrow_mut().update_content(&assistant, false);
        self.transcript.streaming_message = Some(Rc::new(assistant));
    }

    /// The child's message ended: its streamed block takes the full message.
    /// Its run-level effects (the turn's stop reason, auto-loop) are the
    /// session's, so they are not applied here.
    fn child_message_end(&mut self, event: &Value) {
        if let Some(attached) = self.attached.as_mut() {
            attached.child_text.clear();
        }
        let message = event
            .get("message")
            .cloned()
            .and_then(|m| serde_json::from_value::<AgentMessage>(m).ok());
        let assistant = match message {
            Some(AgentMessage::Assistant(assistant)) => Some(assistant),
            _ => None,
        };
        if let Some(component) = self.transcript.streaming.take() {
            match &assistant {
                Some(assistant) => {
                    component.borrow_mut().update_content(assistant, false);
                    for block in self.transcript.pending_tools.values() {
                        block.borrow_mut().set_args_complete();
                    }
                }
                None => self
                    .transcript
                    .chat
                    .borrow_mut()
                    .remove_child(&as_component(&component)),
            }
        }
        self.transcript.streaming_message = None;
    }

    fn child_tool_start(&mut self, event: &Value) {
        self.handle_agent_event(AgentEvent::ToolExecutionStart {
            tool_call_id: str_field(event, "toolCallId"),
            tool_name: str_field(event, "toolName"),
            args: event.get("args").cloned().unwrap_or(Value::Null),
        });
    }

    fn child_tool_end(&mut self, event: &Value) {
        let result = event.get("result");
        let content = result
            .and_then(|r| r.get("content"))
            .cloned()
            .and_then(|c| serde_json::from_value::<Vec<Content>>(c).ok())
            .unwrap_or_default();
        let details = result
            .and_then(|r| r.get("details"))
            .cloned()
            .unwrap_or(Value::Null);
        self.handle_agent_event(AgentEvent::ToolExecutionEnd {
            tool_call_id: str_field(event, "toolCallId"),
            tool_name: str_field(event, "toolName"),
            result: AgentToolResult {
                content,
                details,
                terminate: false,
            },
            is_error: event
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }

    /// The run settled: its open block is finished and the header shows how.
    fn child_settled(&mut self, state: &str) {
        if let Some(component) = self.transcript.streaming.take() {
            self.transcript
                .chat
                .borrow_mut()
                .remove_child(&as_component(&component));
        }
        self.transcript.streaming_message = None;
        self.transcript.pending_tools.clear();
        self.close_open_chain(if state == "complete" {
            ChainState::Done
        } else {
            ChainState::Interrupted
        });
        if let Some(attached) = self.attached.as_mut() {
            attached.child_text.clear();
            attached.header.borrow_mut().set_state(state);
        }
        self.reread_attached_history();
    }

    /// Reads the attached run's transcript again: a run settles after its
    /// transcript is complete, and the view shows all of it, not what a
    /// live attach happened to see.
    fn reread_attached_history(&mut self) {
        let Some(attached) = self.attached.as_ref() else {
            return;
        };
        let run_id = attached.run_id.clone();
        let shown = attached.shown.clone();
        let Some(parent) = self.parent_session_file() else {
            return;
        };
        let cwd = self.session.cwd().to_path_buf();
        let job_id = run_id.clone();
        self.off_ui_thread(
            move || read_run(&parent, &cwd, &job_id, shown),
            move |result| AppEvent::AttachLoaded(run_id, result),
        );
    }

    /// `/agents`: lists this session's runs on the tools pool, then picks one.
    pub(super) fn open_agent_runs(&mut self) {
        let Some(parent) = self.parent_session_file() else {
            self.show_status("This session has no saved transcript, so it has no subagent runs.");
            return;
        };
        self.off_ui_thread(move || list_runs(&parent), AppEvent::AgentRunsListed);
    }

    /// The `/agents` list arrived: a picker of its rows.
    pub(super) fn agent_runs_listed(&mut self, result: Result<Vec<RunRow>, String>) {
        let rows = match result {
            Ok(rows) => rows,
            Err(message) => {
                self.show_error(&message);
                return;
            }
        };
        if rows.is_empty() {
            self.show_status("No subagent runs in this session.");
            return;
        }
        let labels: Vec<String> = rows.iter().map(|r| r.label.clone()).collect();
        let tx = self.tx.clone();
        let answer: Answer = Box::new(move |choice: Option<String>| {
            let row = choice.and_then(|label| rows.into_iter().find(|r| r.label == label));
            let _ = tx.send(AppEvent::AttachPicked(row));
        });
        self.show_selector("Subagent runs", labels, answer);
    }
}

fn str_field(event: &Value, key: &str) -> String {
    event
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use hoocode_ai_types::{Message, StopReason, UserMessage};
    use hoocode_code_session::{
        append_custom_entry_to_file, flush_session_writes, subagents_dir, SessionManager,
        SESSION_FLUSH_DEADLINE, SUBAGENT_RUN_ENTRY,
    };
    use hoocode_tui_components::Text;

    use super::*;

    fn user(text: &str) -> AgentMessage {
        AgentMessage::from_message(Message::User(UserMessage {
            content: vec![Content::text(text)].into(),
            timestamp: 0,
        }))
    }

    fn assistant(text: &str) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            content: vec![Content::text(text)],
            stop_reason: StopReason::Stop,
            ..Default::default()
        })
    }

    /// The chat container's children, each as its rendered text.
    fn labels(chat: &Rc<RefCell<Container>>) -> Vec<String> {
        chat.borrow_mut()
            .children
            .iter()
            .map(|c| c.borrow_mut().render(40).join("").trim_end().to_string())
            .collect()
    }

    fn text_child(text: &str) -> ComponentHandle {
        as_component(&handle(Text::new(text, 0, 0)))
    }

    #[test]
    fn each_half_keeps_its_own_children_across_swaps() {
        let chat = handle(Container::new());
        chat.borrow_mut().add_child(text_child("session one"));
        // The session is on screen; the child's half is empty and parked.
        let mut shown = Transcript::new(chat.clone());
        let mut parked = Transcript::new(chat.clone());
        let mut parked_children = Vec::new();
        // The child's view comes on screen: the session's children are parked.
        swap_halves(&mut shown, &mut parked, &mut parked_children);
        assert!(labels(&chat).is_empty());
        chat.borrow_mut().add_child(text_child("child answer"));
        // The session comes back as it was; the child's view is parked.
        swap_halves(&mut shown, &mut parked, &mut parked_children);
        assert_eq!(labels(&chat), vec!["session one".to_string()]);
        // The child's view again, with its own content still in it.
        swap_halves(&mut shown, &mut parked, &mut parked_children);
        assert_eq!(labels(&chat), vec!["child answer".to_string()]);
        assert!(Rc::ptr_eq(&shown.chat, &chat));
    }

    #[test]
    fn describe_takes_the_first_line_and_cuts_it_short() {
        assert_eq!(describe("  Find the footer \nmore"), "Find the footer");
        assert_eq!(
            describe(&"x".repeat(300)).chars().count(),
            DESCRIPTION_CHARS
        );
        assert_eq!(describe(""), "");
    }

    #[test]
    fn a_run_state_is_named_for_the_header_and_the_list() {
        assert_eq!(state_word(&SubagentState::Running), "running");
        assert_eq!(state_word(&SubagentState::Interrupted), "interrupted");
        assert_eq!(
            state_word(&SubagentState::Finished("complete".into())),
            "complete"
        );
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "attach-{tag}-{}-{}",
            std::process::id(),
            hoocode_code_session::create_session_id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// A parent session with one finished run: its transcript and its record.
    fn parent_with_run(dir: &Path) -> std::path::PathBuf {
        let mut parent = SessionManager::create("/work", Some(dir.into()));
        let parent_path = parent.session_file().expect("persisted").to_path_buf();
        // The header reaches the file with the first assistant message.
        parent.append_message(user("question"));
        parent.append_message(assistant("Asked."));
        parent
            .flush(SESSION_FLUSH_DEADLINE)
            .expect("writes reached the OS");
        let child_path = subagents_dir(&parent_path).join("task-1.jsonl");
        let mut child = SessionManager::open(child_path, None, None);
        child.set_parent_session(parent_path.to_string_lossy().into_owned());
        child.append_message(user("Compute a product"));
        child.append_message(assistant("42"));
        child
            .flush(SESSION_FLUSH_DEADLINE)
            .expect("writes reached the OS");
        append_custom_entry_to_file(
            &parent_path,
            SUBAGENT_RUN_ENTRY,
            serde_json::json!({
                "taskId": "task-1",
                "agentType": "general-purpose",
                "description": "Compute a product",
                "childFile": "task-1.jsonl",
                "outcome": "complete",
            }),
        )
        .expect("record appended");
        let _ = flush_session_writes(SESSION_FLUSH_DEADLINE);
        parent_path
    }

    #[test]
    fn agents_lists_the_session_runs_and_attach_loads_one() {
        let dir = temp_dir("runs");
        let parent = parent_with_run(&dir);
        let rows = list_runs(&parent).expect("listed");
        assert_eq!(rows.len(), 1, "one run in this session");
        assert_eq!(rows[0].run_id, "task-1");
        assert_eq!(
            rows[0].label,
            "general-purpose · complete · Compute a product · task-1"
        );
        let cwd = Path::new("/work");
        let run = read_run(&parent, cwd, "task-1", None).expect("loaded");
        assert_eq!(run.agent_type, "general-purpose");
        assert_eq!(run.state, "complete");
        assert_eq!(run.description, "Compute a product");
        assert_eq!(run.messages.len(), 2, "the child's task and its answer");
        assert!(read_run(&parent, cwd, "no-such-run", None).is_err());
        // A run with no transcript yet is attachable only while it is live.
        assert!(read_run(&parent, cwd, "not-started", None).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
