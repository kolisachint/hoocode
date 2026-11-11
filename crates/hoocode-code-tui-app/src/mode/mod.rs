//! The interactive mode (`interactive-mode.ts`): the `Mode` struct, its setup (`new`) and the
//! event loop (`run`). The rest of the mode is split by concern into the sibling modules.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoocode_ai_types::{ImageContent, StopReason};
use hoocode_code_agent_session::stats::AssistantUsageTotals;
use hoocode_code_agent_session::{
    AgentSession, AgentSessionEvent, AgentSessionRuntime, NavigateTreeResult, StreamingBehavior,
};
use hoocode_code_auth::AuthStorage;
use hoocode_code_media::clipboard_image::ClipboardImage;
use hoocode_code_media::rich_clipboard::CopyFlavour;
use hoocode_code_paths::{env_override, VERSION};
use hoocode_code_permissions::ClearanceChoice;
use hoocode_code_settings::EditorBorder;
use hoocode_code_subagents::agent_log::{agent_log, set_terminal_owned_by_tui};
use hoocode_code_subagents::live::LiveSink;
use hoocode_code_task_store::task_store;
use hoocode_code_tool_bash::BashResult;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_selectors::model_selector::ModelSelectorComponent;
use hoocode_code_tui_selectors::oauth_selector::LoginMode;
use hoocode_code_tui_selectors::scoped_models_selector::ScopedModelsSelectorComponent;
use hoocode_code_tui_selectors::session_selector::SessionSelectorComponent;
use hoocode_code_tui_selectors::settings_selector::{SettingsChange, SettingsSelectorComponent};
use hoocode_code_tui_selectors::tree_selector::TreeSelectorComponent;
use hoocode_code_tui_selectors::user_message_selector::UserMessageSelectorComponent;
use hoocode_code_tui_theme::tui::WARNING_GLYPH;
use hoocode_code_tui_theme::{
    current_theme_name, get_editor_theme, init_theme, is_auto_theme, on_theme_change,
    resolve_auto_theme, set_registered_themes, set_theme, theme, AUTO_THEME_POLL_INTERVAL,
};
use hoocode_code_tui_widgets::background_notice::BackgroundNoticeComponent;
use hoocode_code_tui_widgets::bash_execution::BashExecutionComponent;
use hoocode_code_tui_widgets::controls::ControlsComponent;
use hoocode_code_tui_widgets::skill_block::SkillBlockComponent;
use hoocode_code_tui_widgets::task_panel::TaskPanelComponent;
use hoocode_code_tui_widgets::tool_chain::ChainState;
use hoocode_code_tui_widgets::tool_chain::ToolChainComponent;
use hoocode_code_tui_widgets::UserMessageComponent;
use hoocode_tui_components::{Editor, EditorHost, EditorOptions, FrameBorderStyle, Loader, Spacer};
use hoocode_tui_render::{
    Component, ComponentHandle, Container, FlexSpacer, Slot, Tui, TuiEvent, WakeHandle,
};

use crate::prompt_row::PromptRow;
use crate::start_header::{StartFacts, StartHeader};
use crate::status_line::{StatusLineData, StatusLineView, TurnResult};
use crate::transient_lines::TransientLines;
use hoocode_tui_terminal::Terminal;

use crate::changelog::changelog_for_display;
use crate::chime::{ChimeDebounce, TurnTimer, BELL};
use crate::clearance_queue::ClearanceQueue;
use crate::dialog_bridge::{set_dialog_sink, DialogRequest};
use crate::footer_data::FooterDataProvider;
use crate::input_frame::set_input_frame_border;
use crate::login_controller::LoginUpdate;
use crate::notification_panel::{NotificationKind, NotificationPanel};
use crate::perf::Perf;
use crate::record_row::RecordRows;
use crate::resource_display::{format_display_path, ResourceListing};
use crate::scroll_view::{
    child_base, hit_chain, install_scroll_status, install_scroll_view, BlockToggle, TranscriptFocus,
};
use crate::startup_progress;
use crate::tips::{
    render_tip, tip_ttl, TipRotation, TipRotationOptions, TipsController, TipsControllerOptions,
};
use hoocode_tui_render::BodyClick;

mod attach;
mod auth;
mod bash;
mod chrome;
mod clipboard;
mod commands;
mod dialogs;
mod events;
mod history_search;
#[cfg(test)]
mod history_search_tests;
mod input;
mod job_tail;
mod keymap_notices;
mod leader;
#[cfg(test)]
mod leader_tests;
mod loop_command;
mod mcp;
mod models;
mod plugin;
#[cfg(test)]
mod prompt_flight_tests;
mod prompt_queue;
mod retry;
mod session_op;
mod session_ops;
mod settings;
mod shutdown;
#[cfg(test)]
mod steer_tests;
mod subagents;
mod task_panel;
#[cfg(test)]
mod test_support;
mod transcript;
mod tree;

use self::task_panel::{superseded_quit, EndShift, QuitJobs};
use self::{auth::*, chrome::*, commands::*, dialogs::*, input::*, models::*, transcript::*};

pub use self::events::plan_settle_outcome;
pub use self::input::{at_file_finder, command_path_argument};
pub use self::models::claim_anthropic_subscription_warning;

/// How the mode is started.
pub struct InteractiveOptions {
    pub session: AgentSession,
    /// The owner of `session` that replaces it (`/resume`, alt+h). Without
    /// one the session cannot be switched.
    pub session_runtime: Option<AgentSessionRuntime>,
    /// Where agent turns run.
    pub runtime: tokio::runtime::Handle,
    /// The resource listing for a session, read when it is drawn.
    pub listing: Box<dyn Fn(&AgentSession) -> ResourceListing>,
    /// The credential store `/login` and `/logout` write (the session's own).
    pub auth_storage: Arc<AuthStorage>,
    pub version: String,
    /// Force the verbose startup banner.
    pub verbose: bool,
    pub initial_message: Option<String>,
    /// Sent with `initial_message` (the `@file` images).
    pub initial_images: Vec<ImageContent>,
    pub initial_messages: Vec<String>,
    /// Shown as a notice (the only place the remedy is named).
    pub model_fallback_message: Option<String>,
    /// Startup warnings (a setting value that was ignored), shown as warnings
    /// in the transcript; stderr is behind the alternate screen.
    pub startup_warnings: Vec<String>,
    pub terminal: Option<Box<dyn Terminal>>,
    /// `--perf-log`: one JSON line of the UI's counters a second is appended here.
    pub perf_log: Option<std::path::PathBuf>,
    /// The startup version check's result (`hoocode-code-update`), polled by the loop.
    pub update_notice: Option<std::sync::mpsc::Receiver<hoocode_code_update::UpdateAvailable>>,
}

use session_op::{SessionOpDone, SessionOutcome};

const DEFAULT_HIDDEN_THINKING_LABEL: &str = "Thinking...";

/// Finished tool blocks kept live; older ones are frozen.
const LIVE_TOOL_WINDOW: usize = 50;

/// Minimum gap between re-renders of the streaming message.
const STREAM_RENDER_THROTTLE: Duration = Duration::from_millis(100);

/// Most keys taken from the input channel in one loop pass. The rest wait for the
/// next pass, so AppEvents and ticks run between batches (concurrency.md, Phase 2).
const MAX_INPUT_PER_ITERATION: usize = 64;

/// Least time between the end of a pass and a repaint that only an app event
/// asked for (about 60 frames a second while text streams).
const MIN_WAKE_FRAME_GAP: Duration = Duration::from_millis(16);

/// The longest the loop sleeps with nothing due. Every wake that matters has its
/// own deadline in `next_wakeup`. The rest are checked once per pass (extension
/// requests, the update notice, the memory and stall flags) and can be this late.
const IDLE_WAKE_CAP: Duration = Duration::from_secs(1);

/// Most AppEvents taken from the channel in one loop pass. Leftovers wake the
/// loop again, so keys and ticks run between batches.
const MAX_APP_EVENTS_PER_ITERATION: usize = 256;

/// A streamed update of the assistant message.
fn is_message_update(event: &AppEvent) -> bool {
    matches!(
        event,
        AppEvent::Session(session_event, _)
            if matches!(
                **session_event,
                AgentSessionEvent::Agent(hoocode_agent_types::AgentEvent::MessageUpdate { .. })
            )
    )
}

/// The UI loop has dropped its receiver.
struct LoopGone;

/// Sends [`AppEvent`]s to the UI loop and wakes it, so an event is handled at
/// once and not at the next tick. Wakes are coalesced (see `WakeHandle`).
#[derive(Clone)]
struct AppSender {
    tx: Sender<AppEvent>,
    wake: WakeHandle,
}

impl AppSender {
    /// Queues the event, then wakes the loop. The event goes first, so a loop
    /// woken by it always finds it.
    fn send(&self, event: AppEvent) -> Result<(), LoopGone> {
        self.tx.send(event).map_err(|_| LoopGone)?;
        self.wake.wake();
        Ok(())
    }

    /// Wakes the loop without an event (events were left in the channel).
    fn wake(&self) {
        self.wake.wake();
    }
}

/// Makes sure the spawned prompt task always reports `PromptDone`, so
/// `prompt_pending` cannot stay set: if the task panics or is dropped before it
/// sends, the drop sends an error result (a closed channel is ignored).
struct PromptDoneGuard {
    tx: AppSender,
    sent: bool,
}

impl PromptDoneGuard {
    fn new(tx: AppSender) -> Self {
        Self { tx, sent: false }
    }

    fn send(&mut self, result: Result<(), String>) {
        self.sent = true;
        let _ = self.tx.send(AppEvent::PromptDone(result));
    }
}

impl Drop for PromptDoneGuard {
    fn drop(&mut self) {
        if !self.sent {
            let _ = self.tx.send(AppEvent::PromptDone(Err(
                "The prompt ended without a result.".into(),
            )));
        }
    }
}

/// Events from outside the UI thread.
enum AppEvent {
    /// A session event; `agent_start` carries the usage totals sampled when
    /// it fired (the turn-cost anchor), since the UI thread sees it later.
    Session(
        Box<AgentSessionEvent>,
        Option<(AssistantUsageTotals, Instant)>,
    ),
    PromptDone(Result<(), String>),
    Rerender,
    ThemeChanged,
    /// A question or notice from off the UI thread (the permission gate).
    Dialog(DialogRequest),
    /// A step of a running OAuth login.
    Login(LoginUpdate),
    /// Output from the running `!` command.
    BashChunk(String),
    /// The `!` command ended.
    BashDone(Result<BashResult, String>),
    /// A `/copy` write ended: what was copied, and the flavour that landed.
    CopyDone(String, Result<CopyFlavour, String>),
    /// A `y` copy of a focused block's raw output ended: what was copied,
    /// and its bytes that landed.
    CopyRawDone(String, Result<usize, String>),
    /// The clipboard image read ended.
    PastedImage(Option<ClipboardImage>),
    /// A `/subagent` run ended: its mode, and its summary or error.
    SubagentDone(String, Result<Option<String>, String>),
    /// A live event of a subagent run (`hoocode_code_subagents::live`): its pool
    /// task id, then the child's event (or `{"type": "task_*"}` for a lifecycle step).
    Subagent(String, serde_json::Value),
    /// The attached run's transcript was read (or failed), with its run id.
    AttachLoaded(String, Result<attach::LoadedRun, String>),
    /// `/agents`'s runs were listed (or failed).
    AgentRunsListed(Result<Vec<attach::RunRow>, String>),
    /// The `/agents` picker answered: the run to attach to, if one was chosen.
    AttachPicked(Option<attach::RunRow>),
    /// A session operation run off the UI thread ended (`session_op.rs`).
    SessionOp(Box<session_op::SessionOpDone>),
    /// Queued messages sent after a compaction failed; they go back.
    CompactionQueueFailed(Vec<(String, StreamingBehavior)>, String),
    /// A message typed while streaming could not be queued.
    QueueError(String),
    /// A status line from off the UI thread (a subagent cancel's outcome).
    Status(String),
    /// The `/import` confirm, or the confirm for a stored cwd that is gone, answered.
    ImportAnswer {
        input: String,
        cwd_fallback: Option<String>,
        choice: Option<String>,
    },
    /// The confirm for a stored cwd that is gone, on `/resume`, answered.
    ResumeAnswer {
        path: PathBuf,
        fallback: String,
        choice: Option<String>,
    },
    /// "Summarize branch?" answered, for the tree entry `entry_id`.
    TreeSummaryAnswer {
        entry_id: String,
        choice: Option<String>,
    },
    /// The custom summary instructions answered (`None`: cancelled).
    TreeInstructionsAnswer {
        entry_id: String,
        text: Option<String>,
    },
    /// A summarizing tree navigation ended. A panicked one is an `Err`.
    TreeNavigated {
        /// Which navigation this is (`Mode::tree_navigation`).
        navigation: u64,
        entry_id: String,
        outcome: Box<Result<NavigateTreeResult, String>>,
    },
    /// A read of the attached shell job's output (T5.3): its row key, the byte
    /// offset the read started from (`None`: the tail), and the result.
    ShellTail(
        String,
        Option<u64>,
        Result<hoocode_code_tool_bash::jobs::JobRead, String>,
    ),
}

/// `new Date().toISOString()`.
fn iso_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn handle<C: Component + 'static>(c: C) -> Rc<RefCell<C>> {
    Rc::new(RefCell::new(c))
}

fn as_component<C: Component + 'static>(c: &Rc<RefCell<C>>) -> ComponentHandle {
    c.clone()
}

/// Whether full-screen is on. `HOOCODE_FULLSCREEN` reads as a boolean
/// (`0`/`false`/`no`/`off` or `1`/`true`/`yes`/`on`, see `parse_env_bool`)
/// and wins over the `fullscreen` setting for one run. Any other value, or
/// none, takes the setting.
pub fn fullscreen_enabled(env: Option<&str>, setting: bool) -> bool {
    env.and_then(hoocode_code_paths::parse_env_bool)
        .unwrap_or(setting)
}

/// Whether `text` flips the bash-mode (`!`) state recorded in `last`. It records the new state,
/// and reports only a change, so the mode hears about a flip rather than every keystroke.
fn bash_mode_flip(last: &Cell<bool>, text: &str) -> Option<bool> {
    let now = text.trim_start().starts_with('!');
    (last.replace(now) != now).then_some(now)
}

struct Mode {
    /// The chat transcript and the components that track it (`transcript.rs`).
    transcript: Transcript,
    /// The full-screen body holding the chat, when the screen has one: the
    /// pinned view's anchor is laid out through it (`scroll_view.rs`).
    scroll_body: Option<Rc<RefCell<Container>>>,
    /// The attached subagent view, while one is shown (`attach.rs`).
    attached: Option<attach::Attached>,
    /// The attached view's header row, above the chat (empty when not attached).
    attach_slot: Rc<RefCell<Container>>,
    session: AgentSession,
    runtime: tokio::runtime::Handle,
    tui: Tui,
    verbose: bool,
    size: Rc<Cell<(u16, u16)>>,
    dirty: Rc<Cell<bool>>,
    perf: Perf,
    /// The top rows: the start screen's two lines, then one blank row.
    header: Rc<RefCell<StartHeader>>,
    /// What the start screen's second line counts (folder, skills, agents).
    start_facts: Rc<RefCell<StartFacts>>,
    /// The startup header's container: the header is in it, except while the
    /// attached view is up (`attach.rs` `set_start_header`).
    header_container: Rc<RefCell<Container>>,
    status: Rc<RefCell<Container>>,
    loader: Option<Rc<RefCell<Loader>>>,
    stream_render_at: Option<Instant>,
    stream_render_pending: bool,
    scheduler_tick_at: Instant,
    /// A prompt was spawned and its `PromptDone` has not arrived. `session.is_streaming()`
    /// flips only later (template expansion, the MCP sync and a compaction check come
    /// first), so without this a second prompt started in that gap would race the first
    /// and lose with "Agent is already processing a prompt". Set in `start_prompt`, cleared
    /// first thing at `PromptDone`; see `agent_busy`.
    prompt_pending: bool,
    /// The pending prompt is the background-job wake (not typed, not a `/loop` turn).
    wake_in_flight: bool,
    /// The last job wake ended in an error before any model call: it is not retried
    /// until the notices are gone (a new completion or a model call took them).
    job_wake_failed: bool,
    /// Messages queued behind the pending prompt with `session.follow_up` while it was
    /// still starting: if it ends without a run they are returned to the editor.
    gap_queued: bool,
    /// Scheduled prompts that fired and wait for the agent to be free, oldest first.
    /// One is sent per idle pass (`send_next_scheduled`).
    scheduled_backlog: VecDeque<String>,
    /// The `/loop auto` run in progress, if any.
    auto_loop: Option<loop_command::LoopRun>,
    /// The scheduled prompts `/loop` created in this session (`/loop stop` removes them).
    loop_tasks: Vec<loop_command::LoopTask>,
    /// Scheduled prompts fired and not yet finished: their Schedule rows in the task panel.
    scheduled_runs: hoocode_code_scheduler::Runs,
    turn_cost_anchor: Option<(AssistantUsageTotals, Instant)>,
    /// How the last settled turn ended, for the status line's left slot
    /// until the next prompt is sent.
    last_turn: Option<TurnResult>,
    turn_stop_reason: Option<StopReason>,
    /// A `/plugin` change made while a turn ran: applied when the turn ends (`PromptDone`).
    plugin_reload_pending: bool,
    /// Notices of `/plugin` changes, shown after the reload that makes them live.
    plugin_notices: Vec<String>,
    /// When the current prompt was submitted (set at submit, taken at `PromptDone`).
    turn_timer: Rc<RefCell<TurnTimer>>,
    chime: ChimeDebounce,
    /// Reports state to Herdr when hoocode runs in a Herdr pane (set in `run`).
    herdr: Option<crate::herdr::Reporter>,
    /// Reports OSC 7501 program status to this terminal's stdout (set in
    /// `run`; `None` on a dumb terminal). Runs everywhere, not just Herdr.
    osc: Option<crate::herdr::OscReporter>,
    /// How many selectors asked for by `DialogRequest::Select` (the permission
    /// gate's questions) wait for their answer.
    dialog_select: Rc<Cell<u32>>,
    update_notice: Option<std::sync::mpsc::Receiver<hoocode_code_update::UpdateAvailable>>,
    hide_thinking_block: bool,
    chain_closed_for_current_message: bool,
    dial_reverse_taught: HashSet<&'static str>,
    last_tool_tick: Instant,
    /// When the OS appearance is next re-checked while the settings theme is
    /// `"auto"` (a `next_wakeup` deadline, so the loop wakes for it).
    auto_theme_next_check: Instant,
    show_images: bool,
    image_width_cells: u32,
    code_block_indent: String,
    editor: Rc<RefCell<CustomEditor>>,
    /// The prompt row around the editor. Dialogs and the attached view
    /// replace it in the editor's slot; the prompt itself stays focused.
    prompt_row: Rc<RefCell<PromptRow>>,
    /// The prompt's bash-mode marker (`!`), kept with `is_bash_mode`.
    bash_marker: Rc<Cell<bool>>,
    /// The last retry gave up and no new prompt has gone out since: an empty
    /// enter sends the last prompt again (M8e1).
    comms_lost: Rc<Cell<bool>>,
    /// The status line's data, refreshed each loop pass before the frame.
    status_data: Rc<RefCell<StatusLineData>>,
    editor_container: Rc<RefCell<Container>>,
    /// The picker in the editor's slot (`dialogs.rs`).
    picker: Option<ActivePicker>,
    actions: Rc<RefCell<Vec<Action>>>,
    notifications: Rc<RefCell<NotificationPanel>>,
    tips: TipsController,
    /// The transient rows above the prompt (warning, retry, statuses, progress).
    transient: Rc<RefCell<TransientLines>>,
    /// The leader key at the prompt (Ctrl+X, then a key): see `leader.rs`.
    leader: leader::LeaderState,
    /// Problems in `keybindings.json` (errors, then cautions), listed by `/hotkeys`.
    keybinding_problems: Vec<String>,
    /// The open Ctrl+R history search, if any (`history_search.rs`).
    history_search: Option<history_search::HistorySearch>,
    /// Mirrors `history_search.is_some()` for `CustomEditor`, which routes keys.
    history_search_active: Rc<Cell<bool>>,
    /// Messages are queued: `CustomEditor` lets Up take the last one back.
    queue_recallable: Rc<Cell<bool>>,
    /// The transcript's scroll view: the leader opens its search (`leader.rs`).
    scroll_view: Rc<RefCell<crate::scroll_view::ScrollView>>,
    /// Header clicks queued by the TUI, drained after each event (single
    /// view: a click toggles its block and pins the view first).
    pending_body_clicks: Rc<RefCell<Vec<BodyClick>>>,
    /// U21 single view (P2): the pinned view's focused tool block, shared
    /// with the scroll view (see `mode/transcript/focus.rs`).
    focus: Rc<RefCell<TranscriptFocus>>,
    /// The warning now (shedding or UI stall), if any: shown above the prompt.
    runtime_notice: Option<String>,
    shedding_shown: bool,
    footer_data: FooterDataProvider,
    last_sigint: Option<Instant>,
    tx: AppSender,
    rx: Receiver<AppEvent>,
    exit_requested: bool,
    /// A user-initiated quit (`/quit`, ctrl+c twice, ctrl+d): it shows the
    /// end-of-shift card instead of leaving at once.
    user_quit: bool,
    /// Where the quit prompt for running shell jobs stands (T5.3).
    quit_jobs: QuitJobs,
    /// Where the end-of-shift card stands (M6).
    end_shift: EndShift,
    listing: Box<dyn Fn(&AgentSession) -> ResourceListing>,
    session_runtime: Option<AgentSessionRuntime>,
    subscription: Option<hoocode_code_agent_session::SessionSubscription>,
    auth_storage: Arc<AuthStorage>,
    login: Option<LoginStep>,
    ask_options: Option<OpenAskOptions>,
    clearance: Option<OpenClearance>,
    /// The clearance requests waiting behind the card that is up.
    clearance_queue: ClearanceQueue,
    end_shift_card: Option<OpenEndShift>,
    end_shift_jobs_card: Option<OpenEndShiftJobs>,
    settings_changes: Rc<RefCell<Vec<SettingsChange>>>,
    last_escape: Option<Instant>,
    /// The summarizing tree navigation in flight, by its number (Escape cancels it).
    tree_navigation: Option<u64>,
    /// The number the last summarizing tree navigation was started with.
    tree_navigation_seq: u64,
    editor_dialog: Option<OpenEditorDialog>,
    pending_messages: Rc<RefCell<Container>>,
    /// The queued steering and follow-up messages, as `(steering, follow-up)`.
    /// Written by `update_pending_messages_display`, read by the prompt row,
    /// which draws them as dim rows above the input. Shared so the row's
    /// closure can read it without borrowing the mode.
    queue_snapshot: Rc<RefCell<(Vec<String>, Vec<String>)>>,
    is_bash_mode: bool,
    bash_component: Option<Rc<RefCell<BashExecutionComponent>>>,
    pending_bash_components: Vec<Rc<RefCell<BashExecutionComponent>>>,
    compaction_loader: Option<Rc<RefCell<Loader>>>,
    compaction_queue: Vec<(String, StreamingBehavior)>,
    task_panel: Rc<RefCell<TaskPanelComponent>>,
    /// Whether Up on the prompt's first line may hand the keyboard to the task
    /// panel (shown, with rows, not pinned). Kept current by
    /// `refresh_panel_takes_up`, read by `CustomEditor`.
    panel_takes_up: Rc<Cell<bool>>,
    task_store_subscription: Option<hoocode_code_task_store::Subscription<'static>>,
    previous_cwd: Rc<RefCell<Option<PathBuf>>>,
    restarted_input: Option<Receiver<TuiEvent>>,
    /// The session operation running off the UI thread, if any (`session_op.rs`).
    session_op: Option<&'static str>,
    /// The mode dial's line, held until the reload its `/mode` asked for ends.
    mode_dial_after: Option<(&'static str, String)>,
}

/// Installs the panic hook on the UI thread (`docs/design/concurrency.md` section 4,
/// Panics): a UI-thread panic restores the terminal and flushes the session.
fn install_panic_hook() {
    hoocode_tui_terminal::panic_hook::install(
        std::thread::current().id(),
        hoocode_tui_terminal::panic_hook::Effects {
            restore: Box::new(hoocode_tui_terminal::interrupt::restore_terminal),
            flush: Box::new(|| {
                hoocode_tui_terminal::panic_hook::flush_session(
                    hoocode_tui_terminal::panic_hook::PANIC_FLUSH_DEADLINE,
                )
            }),
            log: Box::new(|report: &str| append_panic_log(report)),
        },
    );
}

/// Appends stderr text that an interactive session withheld from the terminal.
fn append_stderr_to_debug_log(text: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(hoocode_code_paths::debug_log_path())
    {
        let _ = file.write_all(text.as_bytes());
    }
}

/// Appends a panic report to the debug log. Unlike agent log lines, it is written
/// whether or not `DEBUG_AGENT_LOG` is set: a crash should always leave a record.
fn append_panic_log(report: &str) {
    use std::io::Write;
    let path = hoocode_code_paths::debug_log_path();
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "[{now}] {report}");
    }
}

/// What a run starts with: the initial message, its images, the queued
/// messages, the model fallback notice and the startup warnings.
type InitialInput = (
    Option<String>,
    Vec<ImageContent>,
    Vec<String>,
    Option<String>,
    Vec<String>,
);

/// `InteractiveMode.run`: until the user exits.
pub fn run_interactive(options: InteractiveOptions) -> Result<(), String> {
    let initial = (
        options.initial_message.clone(),
        options.initial_images.clone(),
        options.initial_messages.clone(),
        options.model_fallback_message.clone(),
        options.startup_warnings.clone(),
    );
    let mode = Mode::new(options);
    mode.run(initial)
}

impl Mode {
    fn new(options: InteractiveOptions) -> Self {
        let session = options.session;
        let settings = session.settings();
        let (
            show_hardware_cursor,
            clear_on_shrink,
            theme_name,
            editor_border,
            editor_padding_x,
            autocomplete_max_visible,
            hide_thinking_block,
            code_block_indent,
            show_images,
            image_width_cells,
        ) = (
            settings.show_hardware_cursor(),
            settings.clear_on_shrink(),
            settings.theme(),
            settings.editor_border(),
            settings.editor_padding_x() as usize,
            settings.autocomplete_max_visible() as usize,
            settings.hide_thinking_block(),
            settings.code_block_indent(),
            settings.show_images(),
            settings.image_width_cells() as u32,
        );
        // HOOCODE_FULLSCREEN=0 wins over the `fullscreen` setting.
        let fullscreen =
            fullscreen_enabled(env_override("FULLSCREEN").as_deref(), settings.fullscreen());
        drop(settings);
        let terminal = options
            .terminal
            .unwrap_or_else(|| Box::new(hoocode_tui_terminal::ProcessTerminal::new()));
        let size = Rc::new(Cell::new((terminal.columns(), terminal.rows())));
        let perf = Perf::new(options.perf_log.as_deref());
        let mut tui = Tui::new(terminal, Some(show_hardware_cursor));
        tui.set_clear_on_shrink(clear_on_shrink);
        tui.set_fullscreen(fullscreen);
        // The scroll and search bars live in the status line's left slot.
        tui.set_scroll_status_in_bottom(true);
        // A plain click on a link (OSC 8 or a bare URL) opens it in the
        // browser; `open_url` takes only http, https and mailto.
        tui.on_hyperlink = Some(Box::new(crate::login_controller::open_url));
        // A click on a tool header toggles its block. The TUI only queues
        // the row: the mode below drains the queue while the last frame's
        // layout still matches the painted rows the click mapped to.
        let pending_body_clicks: Rc<RefCell<Vec<BodyClick>>> = Rc::default();
        let queued_clicks = pending_body_clicks.clone();
        tui.on_body_click = Some(Box::new(move |click| {
            queued_clicks.borrow_mut().push(click);
        }));

        let keybindings = AppKeybindingsManager::create(None);
        keybindings.install();
        let leader = leader::LeaderState::new(&keybindings);
        let keybinding_problems = keymap_notices::problem_lines(&keybindings);

        // The bundled themes' reference copies, after an upgrade. A failure is
        // a diagnostics warning, never a crash.
        hoocode_code_tui_theme::refresh_theme_defaults(VERSION);
        // Themes: the settings' theme (retired names resolve), watched.
        set_registered_themes(Vec::new());
        init_theme(theme_name.as_deref(), true);

        let dirty = Rc::new(Cell::new(true));
        let actions = Rc::new(RefCell::new(Vec::new()));
        let border = match editor_border {
            EditorBorder::Box => FrameBorderStyle::Box,
            EditorBorder::Rule => FrameBorderStyle::Rule,
        };
        set_input_frame_border(border);
        let rows = size.clone();
        let render_flag = dirty.clone();
        let mut editor = Editor::new(
            EditorHost {
                rows: Box::new(move || rows.get().1),
                request_render: Box::new(move || render_flag.set(true)),
            },
            get_editor_theme(),
            EditorOptions {
                padding_x: Some(editor_padding_x),
                autocomplete_max_visible: Some(autocomplete_max_visible),
                // The prompt row draws the marker and the surface: the
                // editor renders only its text content, with no border.
                // NOTE: `None` would mean the editor default (`Rule`);
                // `Some(None-style)` is the borderless editor.
                border: Some(FrameBorderStyle::None),
            },
        );
        let sink = actions.clone();
        editor.on_submit = Some(Box::new(move |text: &str| {
            sink.borrow_mut().push(Action::Submit(text.to_string()))
        }));
        let sink = actions.clone();
        let bash_mode = Cell::new(false);
        editor.on_change = Some(Box::new(move |text: &str| {
            if let Some(now) = bash_mode_flip(&bash_mode, text) {
                sink.borrow_mut().push(Action::EditorChanged(now));
            }
        }));
        let sink = actions.clone();
        editor.on_autocomplete_visibility_change = Some(Box::new(move |visible| {
            sink.borrow_mut()
                .push(Action::AutocompleteVisibility(visible))
        }));
        let panel_takes_up = Rc::new(Cell::new(false));
        let history_search_active = Rc::new(Cell::new(false));
        let queue_recallable = Rc::new(Cell::new(false));
        let editor = handle(CustomEditor {
            editor,
            actions: actions.clone(),
            panel_takes_up: panel_takes_up.clone(),
            search_active: history_search_active.clone(),
            queue_recallable: queue_recallable.clone(),
        });

        let footer_data = FooterDataProvider::new(session.cwd());
        footer_data
            .set_subagent_enabled(session.get_active_tool_names().iter().any(|t| t == "Agent"));
        // Skip permissions is known from the first frame (the first
        // `apply_runtime_settings` only runs on a reload or switch).
        footer_data.set_skip_permissions(hoocode_code_modes::config::skip_permissions(
            &hoocode_code_modes::config::read_merged_config(session.cwd()),
        ));
        let transient = handle(TransientLines::new(footer_data.clone()));
        // The prompt row: ` ›` on the surface, the queued messages above it.
        let turn_timer = Rc::new(RefCell::new(TurnTimer::default()));
        // The queue snapshot the prompt row reads: `update_pending_messages_display`
        // writes it, the row draws it above the input.
        let queue_snapshot: Rc<RefCell<(Vec<String>, Vec<String>)>> = Rc::default();
        let queue_read = queue_snapshot.clone();
        let comms_lost = Rc::new(Cell::new(false));
        let bash_marker = Rc::new(Cell::new(false));
        let rows_editor = editor.clone();
        let menu_editor = editor.clone();
        // The prompt row draws the completion menu above the prompt.
        editor.borrow_mut().editor.set_inline_completion(false);
        let prompt_row = handle(PromptRow::new(
            as_component(&editor),
            Rc::new(move || rows_editor.try_borrow().map_or(1, |e| e.editor.text_rows())),
            Rc::new(move |max| {
                menu_editor
                    .try_borrow_mut()
                    .ok()
                    .and_then(|mut e| e.editor.render_completion(max))
            }),
            Rc::new(move || queue_read.borrow().clone()),
            bash_marker.clone(),
        ));
        // Single view: the task panel always draws its rows (no outline).
        let task_panel = handle(TaskPanelComponent::new());
        let tasks_slot = handle(Slot::new(as_component(&task_panel)));

        let render_flag = dirty.clone();
        let budget = size.clone();
        let notifications = handle(NotificationPanel::new(
            move || render_flag.set(true),
            Some(Box::new(move || (budget.get().1 / 3) as usize)),
        ));

        // Tips: one small thing about hoocode, on the band, at a moment that was
        // being spent anyway. `band_is_free` keeps it the lowest-priority thing
        // on screen: a tip is dropped rather than queued behind anything.
        let tips = {
            let (s1, s2, s3, s4, s5) = (
                session.clone(),
                session.clone(),
                session.clone(),
                session.clone(),
                session.clone(),
            );
            let (band, show) = (notifications.clone(), notifications.clone());
            TipsController::new(TipsControllerOptions::new(
                Box::new(move || s1.settings().tips_enabled()),
                Box::new(move || band.borrow().showing().is_none()),
                Box::new(move |tip| {
                    let rendered = render_tip(tip);
                    let body: Vec<&str> = rendered.body.iter().map(String::as_str).collect();
                    show.borrow_mut().notify(
                        NotificationKind::Info,
                        &rendered.title,
                        &body,
                        rendered.note.as_deref(),
                        Some(tip_ttl(body.len())),
                        Some("tip"),
                    );
                }),
                TipRotation::new(
                    TipRotationOptions {
                        seen: Box::new(move || s2.settings().seen_tips()),
                        mark_seen: Box::new(move |id| s3.settings().mark_tip_seen(id)),
                        star_nudge_count: Box::new(move || s4.settings().star_nudge_count()),
                        mark_star_nudge: Box::new(move || s5.settings().record_star_nudge()),
                    },
                    None,
                ),
            ))
        };

        // The top rows: the start screen's two lines, then one blank row.
        let cwd = format_display_path(&session.cwd().to_string_lossy());
        let start_facts = Rc::new(RefCell::new(StartFacts {
            cwd,
            skills: 0,
            agents: 0,
        }));
        let chat = handle(Container::new());
        let turns_chat = chat.clone();
        let header = handle(StartHeader::new(
            &options.version,
            start_facts.clone(),
            Rc::new(move || {
                turns_chat
                    .try_borrow()
                    .map_or(true, |chat| chat.children().iter().any(is_turn_row))
            }),
        ));
        let header_container = handle(Container::new());
        header_container
            .borrow_mut()
            .add_child(as_component(&header));
        let body = handle(Container::new());
        let screen_fill = handle(FlexSpacer::new());
        let widget_above = handle(Container::new());
        // Inline only: one blank row between the chat and the rows that hang
        // off the bottom. Full-screen's bottom is exactly the spec's (the
        // prompt and the status line), so an empty container renders zero
        // rows there.
        if !fullscreen {
            widget_above
                .borrow_mut()
                .add_child(as_component(&handle(Spacer::new(1))));
        }
        let editor_container = handle(Container::new());
        editor_container
            .borrow_mut()
            .add_child(as_component(&prompt_row));
        let status_data: Rc<RefCell<StatusLineData>> = Rc::default();
        let status_line = handle(StatusLineView::new(status_data.clone()));
        let attach_slot = handle(Container::new());
        let pending_messages = handle(Container::new());
        let status = handle(Container::new());

        // The app tree, top to bottom: the start header (or the blank top
        // row), the attach slot and the transcript; then the bottom: pending
        // rows, the status area, the task panel, notices, the transient rows,
        // the prompt (or the dialog in its place) and the status line.
        // Full-screen lays the transcript out in a body it scrolls itself;
        // inline pushes it down with a fill.
        tui.add_child(as_component(&header_container));
        if fullscreen {
            tui.add_child(as_component(&attach_slot));
            body.borrow_mut().add_child(as_component(&chat));
            tui.add_child(as_component(&body));
            tui.set_fullscreen_body(Some(as_component(&body)));
        } else {
            tui.add_child(as_component(&screen_fill));
            tui.set_flex_spacer(Some(screen_fill.clone()));
            tui.add_child(as_component(&attach_slot));
            tui.add_child(as_component(&chat));
        }
        tui.add_child(as_component(&pending_messages));
        tui.add_child(as_component(&status));
        tui.add_child(as_component(&widget_above));
        tui.add_child(tasks_slot.clone());
        tui.add_child(as_component(&notifications));
        tui.add_child(as_component(&transient));
        tui.add_child(as_component(&editor_container));
        tui.add_child(as_component(&handle(Container::new()))); // widgets below
        tui.add_child(as_component(&status_line));
        tui.set_focus(Some(as_component(&prompt_row)));
        // Scrolling the transcript: the prompt's pager keys, and the keys of
        // the pinned view (`scroll_view.rs`).
        let prompt_text = editor.clone();
        let busy_session = session.clone();
        let scroll_view = install_scroll_view(
            &mut tui,
            as_component(&prompt_row),
            chat.clone(),
            fullscreen.then(|| body.clone()),
            Rc::new(is_turn_row),
            Rc::new(move || {
                prompt_text
                    .try_borrow()
                    .map_or(true, |e| e.editor.get_text().is_empty())
            }),
            Rc::new(move || busy_session.is_streaming() || busy_session.is_retrying()),
        );
        // U21 single view (P2): the pinned view's transcript focus, shared
        // with the scroll view (`mode/transcript/focus.rs`).
        let focus = Rc::new(RefCell::new(TranscriptFocus::new()));
        scroll_view.borrow_mut().set_focus_state(focus.clone());
        install_scroll_status(&mut tui, status_data.clone(), focus.clone());

        let (tx, rx) = mpsc::channel();
        let tx = AppSender {
            tx,
            wake: tui.wake_handle(),
        };
        Self {
            session,
            runtime: options.runtime,
            tui,
            verbose: options.verbose,
            size,
            dirty,
            perf,
            header,
            start_facts,
            header_container,
            scroll_body: fullscreen.then(|| body.clone()),
            transcript: Transcript::new(chat),
            attached: None,
            attach_slot,
            status,
            loader: None,
            stream_render_at: None,
            stream_render_pending: false,
            scheduler_tick_at: Instant::now() + hoocode_code_scheduler::TICK_INTERVAL,
            prompt_pending: false,
            wake_in_flight: false,
            job_wake_failed: false,
            gap_queued: false,
            scheduled_backlog: VecDeque::new(),
            auto_loop: None,
            loop_tasks: Vec::new(),
            scheduled_runs: Default::default(),
            turn_cost_anchor: None,
            last_turn: None,
            turn_stop_reason: None,
            plugin_reload_pending: false,
            plugin_notices: Vec::new(),
            turn_timer,
            chime: ChimeDebounce::default(),
            herdr: None,
            osc: None,
            dialog_select: Rc::new(Cell::new(0)),
            update_notice: options.update_notice,
            hide_thinking_block,
            chain_closed_for_current_message: false,
            dial_reverse_taught: HashSet::new(),
            last_tool_tick: Instant::now(),
            auto_theme_next_check: Instant::now() + AUTO_THEME_POLL_INTERVAL,
            show_images,
            image_width_cells,
            code_block_indent,
            editor,
            prompt_row,
            bash_marker,
            comms_lost,
            status_data,
            editor_container,
            actions,
            notifications,
            tips,
            transient,
            leader,
            scroll_view,
            pending_body_clicks,
            focus,
            runtime_notice: None,
            shedding_shown: false,
            footer_data,
            last_sigint: None,
            tx,
            rx,
            exit_requested: false,
            quit_jobs: QuitJobs::Unasked,
            end_shift: EndShift::Unasked,
            user_quit: false,
            listing: options.listing,
            session_runtime: options.session_runtime,
            subscription: None,
            auth_storage: options.auth_storage,
            login: None,
            picker: None,
            ask_options: None,
            clearance: None,
            clearance_queue: ClearanceQueue::default(),
            end_shift_card: None,
            end_shift_jobs_card: None,
            settings_changes: Rc::default(),
            last_escape: None,
            tree_navigation: None,
            tree_navigation_seq: 0,
            editor_dialog: None,
            pending_messages,
            queue_snapshot,
            is_bash_mode: false,
            bash_component: None,
            pending_bash_components: Vec::new(),
            compaction_loader: None,
            compaction_queue: Vec::new(),
            task_panel,
            panel_takes_up,
            history_search: None,
            keybinding_problems,
            history_search_active,
            queue_recallable,
            task_store_subscription: None,
            previous_cwd: Rc::new(RefCell::new(None)),
            restarted_input: None,
            session_op: None,
            mode_dial_after: None,
        }
    }

    /// The next time something is due without input.
    /// One event from the TUI's input channel, handled without painting: the
    /// loop paints once after it has taken the pass's events.
    fn accept_terminal_event(&mut self, event: TuiEvent) {
        // A keystroke that turns out to do nothing is still the user being
        // present: it restarts the tips' idle clock.
        if let TuiEvent::Input(_, arrived) = &event {
            self.tips.on_activity();
            self.perf.key_arrived(*arrived);
        }
        if let TuiEvent::Resize = event {
            let terminal = &self.tui.terminal;
            self.size.set((terminal.columns(), terminal.rows()));
        }
        // Keys reach the screen's components with the view they show on screen.
        self.show_attached();
        // The keyboard owner is settled before this key and again after it, so
        // a key in the same batch as an Up or a Down-exit reaches the right place.
        self.refresh_panel_takes_up();
        self.release_stale_panel_focus();
        // The leader sees a key first: a key it takes never reaches the screen.
        let taken = match &event {
            TuiEvent::Input(data, _) => self.leader_takes_key(data),
            _ => false,
        };
        if !taken {
            self.tui.accept_event(event);
        }
        self.drain_body_clicks();
        // U21 single view (P2): the transcript keys the pinned view queued.
        self.drain_focus_intents();
        self.hand_off_panel_up();
        self.return_panel_focus_if_asked();
        self.show_main();
    }

    /// The chat's base buffer row in the last frame's layout: the body holds
    /// it full-screen, else it is a direct child of the TUI root.
    fn chat_base_row(&self, width: u16) -> Option<usize> {
        let chat_handle: ComponentHandle = self.transcript.chat.clone();
        match &self.scroll_body {
            Some(body) => child_base(&body.borrow(), &chat_handle, width),
            None => {
                let offsets = self.tui.child_row_offsets(width)?;
                let children = self.tui.children();
                if children.len() != offsets.len() {
                    return None;
                }
                children
                    .iter()
                    .position(|c| Rc::ptr_eq(c, &chat_handle))
                    .map(|i| offsets[i])
            }
        }
    }

    /// A queued header click toggles its block and pins the view, keeping
    /// the header on the row the click found it on (decision 6). A click on
    /// a body row, or outside every chain, queued nothing.
    fn drain_body_clicks(&mut self) {
        let clicks = std::mem::take(&mut *self.pending_body_clicks.borrow_mut());
        if clicks.is_empty() {
            return;
        }
        for click in clicks {
            let base = self.chat_base_row(click.width);
            let rel = base.and_then(|b| (click.buffer_row as usize).checked_sub(b));
            let hit = rel.and_then(|r| hit_chain(&self.transcript.chat, click.width, r));
            let Some((chain, block)) = hit else { continue };
            let toggle = BlockToggle {
                chain,
                block,
                buffer_row: click.buffer_row,
                context: click.context,
            };
            // The click shares `enter`'s primitive (see `set_block_expanded`):
            // both paths read the block's flag.
            let target = toggle
                .chain
                .borrow_mut()
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<ToolChainComponent>())
                .and_then(|c| c.tool_blocks().get(toggle.block).cloned());
            let Some(target) = target else {
                continue;
            };
            let expanded = !target.borrow().is_expanded();
            self.set_block_expanded(&target, expanded);
            self.move_focus_on_click(&target);
            // Pin first, keeping the header on its row; the toggle opens
            // under it.
            self.tui
                .scroll_to_row(toggle.buffer_row, Some(toggle.context));
            self.dirty.set(true);
        }
    }

    /// Acts on the watchdog once per loop turn. A hard-limit trip aborts the turn
    /// and flushes the session. Shedding changes clear the render caches. The
    /// warning above the prompt follows the stall flag and shedding. The stall flag can only
    /// show once the loop turns again, since a stuck loop cannot draw it.
    fn sync_runtime_health(&mut self) {
        for event in hoocode_runtime::take_memory_events() {
            if let hoocode_runtime::MemoryEvent::HardLimit { rss, limit } = event {
                self.session.agent().abort();
                let flushed = hoocode_runtime::session_io()
                    .flush_blocking(Duration::from_secs(1))
                    .is_ok();
                self.show_error(&format!(
                    "Memory limit reached: {} MB resident, hard limit {} MB. The turn was aborted{}. Raise performance.memoryHardLimitMb or end the session.",
                    rss / hoocode_runtime::MIB,
                    limit / hoocode_runtime::MIB,
                    if flushed { " and the session saved" } else { "" },
                ));
            }
        }
        let shedding = hoocode_runtime::shedding();
        if shedding != self.shedding_shown {
            self.shedding_shown = shedding;
            self.tui.invalidate();
            self.dirty.set(true);
        }
        let notice = if hoocode_runtime::watchdog::ui_stalled() {
            Some(format!(
                "{WARNING_GLYPH} UI stalled: no response for 2 s; keys are queued"
            ))
        } else if shedding {
            Some(format!(
                "{WARNING_GLYPH} Memory above the soft limit: no new subagents; tool calls run one at a time"
            ))
        } else {
            None
        };
        if notice != self.runtime_notice {
            self.transient.borrow_mut().set_notice(notice.clone());
            self.runtime_notice = notice;
            self.dirty.set(true);
        }
    }

    /// `PromptDone`: the prompt in flight is over (its turn, if it ran one, has
    /// ended). Settles the request, then starts the next queued start-up message,
    /// the next `/loop auto` step or the next scheduled prompt.
    fn on_prompt_done(&mut self, result: Result<(), String>, queued: &mut Vec<String>) {
        // The prompt in flight is over, whatever it ended in.
        self.prompt_pending = false;
        // A wake whose notices are still pending lost none: they ride the
        // next model call. Do not re-wake at once — an abort before the first
        // model call ends `Ok` with the notices still due.
        if std::mem::take(&mut self.wake_in_flight)
            && (result.is_err() || self.session.job_wake_pending())
        {
            self.job_wake_failed = true;
        }
        // The request has ended (retries and continuations
        // included): settle it (`settleRequestOnIdle`). The
        // last chain has nothing after it to close it.
        // A run that failed outright ended in error, whatever
        // its last message said.
        let stop = if result.is_err() {
            Some(StopReason::Error)
        } else {
            self.turn_stop_reason
        };
        // The error the turn ended with is the mission's reason. It
        // may come with no error message on the last reply, so the
        // flight data keeps it (`set_mission_failure`).
        if let Err(error) = &result {
            self.transcript.set_mission_failure(Some(error.clone()));
        }
        let outcome = if stop == Some(StopReason::Stop) {
            ChainState::Done
        } else {
            ChainState::Interrupted
        };
        self.close_open_chain(outcome);
        self.show_turn_cost(stop);
        self.settle_dangling_plan_items();
        let run_ok = result.is_ok() && self.turn_stop_reason == Some(StopReason::Stop);
        self.scheduled_runs
            .run_finished(task_store(), run_ok, &chrono::Local::now());
        self.tips.on_turn_end();
        self.finish_turn_chime();
        let failed = result.is_err();
        if let Err(error) = result {
            self.show_error(&error);
        }
        // A `/plugin` change waited for the turn to end: the next turn sees it.
        if std::mem::take(&mut self.plugin_reload_pending) {
            self.reload_plugins();
        }
        // Messages queued behind the prompt while it started, and left over
        // because it ran no turn. A prompt that succeeded without a turn (an
        // extension command, which may send its own follow-up, e.g. `/grill`)
        // leaves them to run now. One that failed before its turn sends them
        // back to the editor, so they are not sent to the same failure.
        if std::mem::take(&mut self.gap_queued) && !self.session.is_streaming() {
            if failed {
                if self.restore_queued_messages_to_editor(false) > 0 {
                    self.show_status("Queued message returned to the prompt");
                }
            } else {
                self.run_gap_queued();
            }
        }
        if let Some(next) = queued.pop() {
            self.prompt(next);
        } else {
            self.auto_loop_turn_done(failed);
        }
        self.send_next_scheduled();
    }

    fn next_wakeup(&self) -> Duration {
        let now = Instant::now();
        let mut wait = IDLE_WAKE_CAP;
        let deadlines = [
            self.notifications.borrow().deadline(),
            self.tips.deadline(),
            self.editor.borrow().editor.autocomplete_deadline(),
        ];
        for deadline in deadlines
            .into_iter()
            .chain(self.leader_wake_times(now))
            .flatten()
        {
            wait = wait.min(deadline.saturating_duration_since(now));
        }
        // Wake when a loader's frame falls due, not on a fixed cadence: the
        // pulse flips on its own clock, so the loop must not sleep past it.
        for loader in [&self.loader, &self.compaction_loader]
            .into_iter()
            .flatten()
        {
            if let Some(due) = loader.borrow().next_deadline() {
                wait = wait.min(due.saturating_duration_since(now));
            }
        }
        // The bash spinner advances in the pass too (its frame is not a loader above).
        if let Some(bash) = &self.bash_component {
            if let Some(due) = bash.borrow().next_deadline() {
                wait = wait.min(due.saturating_duration_since(now));
            }
        }
        if let Some(deadline) = self.picker.as_ref().and_then(|p| p.deadline()) {
            wait = wait.min(deadline.saturating_duration_since(now));
        }
        // The OS-appearance re-check while the settings theme is `"auto"`:
        // a deadline like the rest, so the loop wakes for it.
        wait = wait.min(self.auto_theme_next_check.saturating_duration_since(now));
        if self.stream_render_pending {
            if let Some(at) = self.stream_render_at {
                wait = wait.min((at + STREAM_RENDER_THROTTLE).saturating_duration_since(now));
            }
        }
        // The once-a-second tick in `run` (attached view, task panel, turn clock,
        // ticking tool blocks): wake for it on time, not at the first pass after it.
        if self.tool_tick_running() {
            let due = self.last_tool_tick + Duration::from_secs(1);
            wait = wait.min(due.saturating_duration_since(now));
        }
        wait.max(Duration::from_millis(1))
    }

    /// Whether the once-a-second tick in `run` has anything to redraw.
    fn tool_tick_running(&self) -> bool {
        self.attached.is_some()
            || self.task_panel.borrow().ticking()
            || self.turn_cost_anchor.is_some()
            || self
                .transcript
                .pending_tools
                .values()
                .any(|block| block.borrow().is_ticking())
    }

    /// `pollAutoTheme`: while the settings theme is `"auto"`, re-read the OS
    /// appearance at most every `AUTO_THEME_POLL_INTERVAL` and switch to the
    /// matching side. The settings keep the raw `"auto"` value; `set_theme`
    /// fires the `ThemeChanged` repaint, so a flip lands like any other theme
    /// change. Silent on failure: the next tick retries.
    fn poll_auto_theme(&mut self) {
        let now = Instant::now();
        if now < self.auto_theme_next_check {
            return;
        }
        self.auto_theme_next_check = now + AUTO_THEME_POLL_INTERVAL;
        let setting = self.session.settings().theme();
        let Some(setting) = setting else {
            return;
        };
        if !is_auto_theme(&setting) {
            return;
        }
        let resolved = resolve_auto_theme(&setting);
        if current_theme_name().as_deref() != Some(resolved.as_str()) {
            let _ = set_theme(&resolved, true);
        }
    }

    /// Ctrl+C aborts the turn from the input thread, without waiting for this
    /// loop; an exit by signal answers the open tool calls first. Set again
    /// on a session switch, so they reach the current session.
    fn install_interrupt_hook(&self) {
        let session = self.session.clone();
        hoocode_tui_terminal::interrupt::set_hook(Some(Arc::new(move || {
            session.abort_retry();
            session.agent().abort();
        })));
        let session = self.session.clone();
        hoocode_tui_terminal::interrupt::set_exit_hook(Some(Arc::new(move || {
            let _ = session.try_close_unanswered_tool_calls();
        })));
    }

    fn run(mut self, options_initial: InitialInput) -> Result<(), String> {
        self.tui
            .set_frame_observer(Some(self.perf.frame_observer()));
        // Before the terminal starts, so a panic from here on restores it. Stray
        // stderr text goes to the debug log while the TUI owns the terminal.
        install_panic_hook();
        hoocode_runtime::diagnostics::set_stderr_sink(Some(Box::new(|text: &str| {
            append_stderr_to_debug_log(text);
        })));
        let mut input = self.tui.start();
        self.install_interrupt_hook();
        // Only the interactive mode reports to Herdr: print, RPC and subagent
        // runs never build a reporter. OSC 7501 goes to any terminal.
        self.herdr = crate::herdr::Reporter::from_process_env();
        self.osc = crate::herdr::OscReporter::from_process_env();
        // A stop hoocode did not ask for, ended by `fg`: the loop restores the
        // UI (`tick_external_resume`). Held until the loop ends.
        #[cfg(unix)]
        let _sigcont = {
            let wake = self.tui.wake_handle();
            hoocode_runtime::watch_sigcont("hoocode-sigcont", move || {
                crate::suspend::note_sigcont();
                wake.wake();
            })
            .ok()
        };
        // From here the TUI owns the terminal: the agent's operational log
        // lines (dispatch, warm fallback, lifeguard) must not write to it.
        set_terminal_owned_by_tui(true);
        let tx = self.tx.clone();
        self.task_store_subscription = Some(task_store().subscribe(move || {
            let _ = tx.send(AppEvent::Rerender);
        }));
        self.subscription = Some(self.subscribe());

        // Everything the screen shows about the session.
        self.update_terminal_title();
        let theme_name = self.session.settings().theme();
        if let Some(name) = theme_name {
            // A theme that does not load keeps the current one; the warning is
            // listed under theme diagnostics by `render_resources` below.
            let _ = set_theme(&name, true);
        }
        let changelog = {
            let has_messages = !self.session.messages().is_empty();
            let mut settings = self.session.settings();
            changelog_for_display(has_messages, &mut settings, VERSION)
        };
        self.render_resources();
        self.show_startup_notices(changelog);
        self.show_keymap_startup_notes();
        // Messages after the resource listing, as the pin orders them.
        self.render_initial_messages();
        // First run: no working provider. The Log in pane opens on its own;
        // Esc leaves `⊗ not signed in  /login` in the status line.
        if self.session.model().is_none() {
            self.show_oauth_selector(LoginMode::Login);
        }
        self.setup_autocomplete_provider();
        self.update_available_provider_count();

        let tx = Mutex::new(self.tx.clone());
        set_dialog_sink(Some(Box::new(move |request| {
            tx.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(AppEvent::Dialog(request))
                .is_ok()
        })));
        // Live events of subagent runs (T3.1), throttled to the streaming rate.
        let tx = Mutex::new(self.tx.clone());
        hoocode_code_subagents::instance::set_live_sink(Some(LiveSink {
            throttle: STREAM_RENDER_THROTTLE,
            send: Arc::new(move |task_id, event| {
                let _ = tx
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(AppEvent::Subagent(task_id, event));
            }),
        }));
        // Subagents cannot ask for clearance themselves (they run without a
        // screen): their gated calls come here, to this session's gate and card.
        hoocode_code_subagents::instance::set_clearance_relay(Some(Arc::new(
            crate::dialog_bridge::TuiClearanceRelay::new(self.session.cwd().to_path_buf()),
        )));
        self.ask_mcp_trust();
        let tx = self.tx.clone();
        on_theme_change(move || {
            let _ = tx.send(AppEvent::ThemeChanged);
        });
        let tx = self.tx.clone();
        let progress = startup_progress::subscribe(move || {
            let _ = tx.send(AppEvent::Rerender);
        });
        let tx = self.tx.clone();
        let branch = self.footer_data.on_branch_change(move || {
            let _ = tx.send(AppEvent::Rerender);
        });

        let (initial_message, initial_images, initial_messages, fallback, warnings) =
            options_initial;
        if let Some(message) = fallback {
            self.show_error(&message);
        }
        for warning in &warnings {
            self.show_warning(warning);
        }
        if let Some(message) = self.perf.take_startup_error() {
            self.show_error(&message);
        }
        if let Some(model_error) = self.session.model_registry().error() {
            self.show_error(&format!("models.json error: {model_error}"));
        }
        let has_initial_message = initial_message.is_some();
        let mut queued: Vec<String> = initial_message
            .into_iter()
            .chain(initial_messages)
            .collect();
        queued.reverse();
        if let Some(first) = queued.pop() {
            // The @file images go with the initial message only.
            let images = if has_initial_message {
                initial_images
            } else {
                Vec::new()
            };
            self.prompt_with_images(first, images);
        }

        // Watchdog: limits from settings, the heartbeat and the stall log (to the debug log, not the terminal).
        if let Err(error) = hoocode_runtime::watchdog::start() {
            self.show_error(&format!("watchdog did not start: {error}"));
        }
        hoocode_runtime::watchdog::set_log_sink(Box::new(|line: &str| agent_log(line)));
        {
            let settings = self.session.settings();
            hoocode_runtime::configure_memory_limits(
                settings.performance_memory_soft_limit_mb() * hoocode_runtime::MIB,
                settings.performance_memory_hard_limit_mb() * hoocode_runtime::MIB,
            );
        }

        self.tui.request_render(false);
        self.dirty.set(false);
        let mut last_pass_end = Instant::now();
        loop {
            let wait = self.next_wakeup();
            // The wait may run up to `wait`, plus the frame-gap wait below. Silence
            // inside it is not a stall, for the watchdog or the Ctrl+C emergency check.
            let allowance = wait + MIN_WAKE_FRAME_GAP;
            hoocode_runtime::watchdog::ui_beat_waiting("ui-loop:wait", allowance);
            hoocode_tui_terminal::interrupt::ui_beat_waiting(allowance);
            let mut received = input.recv_timeout(wait);
            // A wake from an app event does not repaint sooner than the frame
            // gap after the last pass. A key ends the wait at once.
            if matches!(received, Ok(TuiEvent::Wake)) {
                if let Some(rest) = MIN_WAKE_FRAME_GAP.checked_sub(last_pass_end.elapsed()) {
                    if let Ok(event) = input.recv_timeout(rest) {
                        self.tui.wake_handle().clear();
                        received = Ok(event);
                    }
                }
            }
            // Phase 0 timing: an iteration starts when its input has arrived,
            // not while the loop sleeps.
            let iteration_started = Instant::now();
            // Heartbeat for the input thread's stall check (Ctrl+C twice = emergency exit).
            // The wait is over, so the beat is back to real time for the watchdog too.
            hoocode_tui_terminal::interrupt::ui_beat();
            hoocode_runtime::watchdog::ui_beat("ui-loop:input");
            match received {
                Ok(event) => {
                    self.accept_terminal_event(event);
                    // Keys already queued go in this pass too, up to a cap, so
                    // AppEvents are not starved. The frame is painted once below.
                    for _ in 1..MAX_INPUT_PER_ITERATION {
                        match input.try_recv() {
                            Ok(event) => self.accept_terminal_event(event),
                            Err(_) => break,
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.poll_attached_prompt();
            self.drain_task_panel_events();
            self.poll_leader();
            loop {
                let pending: Vec<Action> = self.actions.borrow_mut().drain(..).collect();
                if pending.is_empty() {
                    break;
                }
                for action in pending {
                    self.handle_action(action);
                }
            }
            hoocode_runtime::watchdog::ui_beat("ui-loop:input-done");
            self.drain_extension_ui_requests();
            self.tick_scheduler();
            self.tick_job_wake();
            self.poll_update_notice();
            self.advance_auto_loop();
            self.poll_auto_theme();
            self.sync_runtime_health();
            self.tick_external_resume();
            if let Some(restarted) = self.restarted_input.take() {
                input = restarted;
            }
            let mut batch = Vec::new();
            while batch.len() < MAX_APP_EVENTS_PER_ITERATION {
                let Ok(event) = self.rx.try_recv() else {
                    break;
                };
                batch.push(event);
            }
            let taken = batch.len();
            let mut batch = batch.into_iter().peekable();
            while let Some(event) = batch.next() {
                // Each update holds the whole message so far: when the next
                // event is another update, this one is already out of date.
                if is_message_update(&event) && batch.peek().is_some_and(is_message_update) {
                    continue;
                }
                match event {
                    AppEvent::Session(event, anchor) => {
                        if self.turn_cost_anchor.is_none() {
                            self.turn_cost_anchor = anchor;
                        }
                        // While an attached view is on screen, the session's
                        // changes are drawn on the next tick, not now.
                        let dirty = self.dirty.get();
                        self.handle_session_event(*event);
                        if self.attached.is_some() {
                            self.dirty.set(dirty);
                        }
                    }
                    AppEvent::PromptDone(result) => self.on_prompt_done(result, &mut queued),
                    AppEvent::Rerender => self.dirty.set(true),
                    AppEvent::Dialog(DialogRequest::Select {
                        title,
                        options,
                        reply,
                    }) => self.show_gate_selector(&title, options, reply),
                    AppEvent::Dialog(DialogRequest::Notify(message)) => self.show_record(&message),
                    AppEvent::Dialog(DialogRequest::AskOptions {
                        questions,
                        reply,
                        chime,
                    }) => {
                        if chime {
                            self.ring_chime();
                        }
                        self.show_ask_options(questions, reply)
                    }
                    AppEvent::Dialog(DialogRequest::HideAskOptions) => self.hide_ask_options(None),
                    AppEvent::Dialog(DialogRequest::Clearance { id, info, reply }) => {
                        self.show_clearance_card(id, info, reply)
                    }
                    AppEvent::Dialog(DialogRequest::HideClearance(id)) => self.hide_clearance(id),
                    AppEvent::Login(update) => self.handle_login_update(update),
                    AppEvent::BashChunk(chunk) => {
                        if let Some(component) = &self.bash_component {
                            component.borrow_mut().append_output(&chunk);
                            self.dirty.set(true);
                        }
                    }
                    AppEvent::BashDone(result) => self.finish_bash_command(result),
                    AppEvent::CopyDone(subject, result) => self.finish_copy(&subject, result),
                    AppEvent::CopyRawDone(subject, result) => {
                        self.finish_raw_copy(&subject, result)
                    }
                    AppEvent::PastedImage(image) => self.insert_pasted_image(image),
                    AppEvent::SubagentDone(mode, result) => self.finish_subagent(&mode, result),
                    AppEvent::Subagent(task_id, event) => {
                        self.apply_subagent_event(&task_id, event)
                    }
                    AppEvent::AttachLoaded(run_id, result) => self.attach_loaded(run_id, result),
                    AppEvent::ShellTail(key, since, result) => {
                        self.shell_tail_read(key, since, result)
                    }
                    AppEvent::AgentRunsListed(result) => self.agent_runs_listed(result),
                    AppEvent::AttachPicked(row) => {
                        if let Some(row) = row {
                            self.attach_run(row.run_id, Some((row.agent_type, row.description)));
                        }
                    }
                    AppEvent::SessionOp(done) => self.finish_session_op(*done),
                    AppEvent::CompactionQueueFailed(queued, error) => {
                        self.session.clear_queue();
                        self.compaction_queue = queued;
                        self.update_pending_messages_display();
                        self.show_error(&error);
                    }
                    AppEvent::QueueError(error) => self.show_error(&error),
                    AppEvent::Status(message) => self.show_status(&message),
                    AppEvent::ImportAnswer {
                        input,
                        cwd_fallback,
                        choice,
                    } => self.import_answered(input, cwd_fallback, choice),
                    AppEvent::ResumeAnswer {
                        path,
                        fallback,
                        choice,
                    } => self.resume_answered(path, fallback, choice),
                    AppEvent::TreeSummaryAnswer { entry_id, choice } => {
                        self.tree_summary_answered(entry_id, choice)
                    }
                    AppEvent::TreeInstructionsAnswer { entry_id, text } => {
                        self.tree_instructions_answered(entry_id, text)
                    }
                    AppEvent::TreeNavigated {
                        navigation,
                        entry_id,
                        outcome,
                    } => self.tree_navigated(navigation, entry_id, *outcome),
                    AppEvent::ThemeChanged => {
                        self.tui.invalidate();
                        self.dirty.set(true);
                    }
                }
            }
            if taken == MAX_APP_EVENTS_PER_ITERATION {
                // Maybe more waiting: run another pass after this one's frame.
                self.tx.wake();
            }
            if self.notifications.borrow_mut().poll() {
                self.dirty.set(true);
            }
            if self.tips.poll() {
                self.dirty.set(true);
            }
            if self.poll_picker_component() {
                self.dirty.set(true);
            }
            if self.last_tool_tick.elapsed() >= Duration::from_secs(1) {
                self.last_tool_tick = Instant::now();
                // The attached view is drawn once a second; the session's changes wait for it.
                if self.attached.is_some() {
                    self.dirty.set(true);
                }
                if self.task_panel.borrow().ticking() {
                    self.dirty.set(true);
                }
                // A running turn's clock: the outline mission row, the live
                // step times and the status line read it, so they redraw
                // once a second while a turn runs.
                if self.turn_cost_anchor.is_some() {
                    self.dirty.set(true);
                }
                // The attached job's output is followed once a second, off the UI thread.
                self.poll_shell_tail();
                for block in self.transcript.pending_tools.values() {
                    if block.borrow().is_ticking() {
                        block.borrow_mut().invalidate();
                        self.dirty.set(true);
                    }
                }
            }
            self.poll_tree_selector();
            self.poll_fork_selector();
            self.poll_model_selectors();
            self.poll_controls();
            self.poll_login();
            self.poll_settings_selector();
            if self.loader.as_ref().is_some_and(|l| l.borrow_mut().tick()) {
                self.dirty.set(true);
            }
            if self
                .compaction_loader
                .as_ref()
                .is_some_and(|l| l.borrow_mut().tick())
            {
                self.dirty.set(true);
            }
            if self
                .bash_component
                .as_ref()
                .is_some_and(|c| c.borrow_mut().tick())
            {
                self.dirty.set(true);
            }
            if self.stream_render_pending
                && self
                    .stream_render_at
                    .is_none_or(|at| at.elapsed() >= STREAM_RENDER_THROTTLE)
            {
                self.run_streaming_render();
            }
            if self.editor.borrow_mut().editor.poll_autocomplete() {
                self.dirty.set(true);
            }
            self.sync_herdr();
            if self.ready_to_quit() {
                break;
            }
            // Input and resizes this pass, and anything else that changed, render once here.
            self.task_panel
                .borrow_mut()
                .set_screen_rows(self.size.get().1);
            self.show_attached();
            self.refresh_status_line();
            if self.dirty.replace(false) {
                self.tui.request_render(false);
            }
            self.tui.flush_scheduled_render();
            self.show_main();
            self.perf.end_iteration(iteration_started);
            last_pass_end = Instant::now();
        }

        hoocode_runtime::watchdog::ui_stopped();
        // The user quit (not a fatal exit): Herdr drops the agent and its
        // resume command.
        if let Some(herdr) = self.herdr.as_mut() {
            herdr.finish(self.user_quit);
        }
        // OSC 7501 goes to every terminal, not just Herdr: drop the root
        // record so no stale working/blocked badge outlives the program.
        if let Some(osc) = self.osc.as_mut() {
            osc.clear();
        }
        set_dialog_sink(None);
        hoocode_code_subagents::instance::set_live_sink(None);
        hoocode_code_subagents::instance::set_clearance_relay(None);
        if let Some(picker) = self.picker.take() {
            picker.dispose();
        }
        if let Some((_, reply)) = self.ask_options.take() {
            let _ = reply.send(None);
        }
        if let Some((_, reply)) = self.clearance.take() {
            let _ = reply.send(ClearanceChoice::Deny { reason: None });
        }
        self.deny_queued_clearances();
        if let Some((_, answer)) = self.editor_dialog.take() {
            answer(None);
        }
        progress.unsubscribe();
        self.footer_data.off_branch_change(branch);
        self.footer_data.dispose();
        hoocode_code_tui_theme::stop_theme_watcher();
        self.tips.stop();
        self.notifications.borrow_mut().stop();
        if let Some(subscription) = self.task_store_subscription.take() {
            subscription.unsubscribe();
        }
        self.task_panel.borrow_mut().dispose();
        hoocode_tui_terminal::interrupt::set_hook(None);
        hoocode_tui_terminal::interrupt::set_exit_hook(None);
        // Abort, kill tool groups, flush, close MCP, release, then restore the terminal.
        let reports = shutdown::run(&self.session, || {
            set_terminal_owned_by_tui(false);
            hoocode_runtime::diagnostics::set_stderr_sink(None);
            self.tui.stop();
        });
        for report in reports
            .iter()
            .filter(|r| r.outcome != hoocode_runtime::StepOutcome::Done)
        {
            agent_log(&format!(
                "shutdown: {} {:?} after {:?}",
                report.name, report.outcome, report.elapsed
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod bash_mode_flip_tests {
    use super::*;

    #[test]
    fn reports_only_the_flip_into_and_out_of_bash_mode() {
        let last = Cell::new(false);
        assert_eq!(bash_mode_flip(&last, "ls"), None);
        assert_eq!(bash_mode_flip(&last, "!l"), Some(true));
        assert_eq!(bash_mode_flip(&last, "!ls"), None);
        assert_eq!(bash_mode_flip(&last, "  !!ls"), None);
        assert_eq!(bash_mode_flip(&last, ""), Some(false));
    }
}
