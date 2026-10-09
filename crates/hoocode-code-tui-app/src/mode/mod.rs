//! The interactive mode (`interactive-mode.ts`): the `Mode` struct, its setup (`new`) and the
//! event loop (`run`). The rest of the mode is split by concern into the sibling modules.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hoocode_ai_types::{ImageContent, StopReason};
use hoocode_code_agent_session::stats::AssistantUsageTotals;
use hoocode_code_agent_session::{
    AgentSession, AgentSessionEvent, AgentSessionRuntime, StreamingBehavior,
};
use hoocode_code_auth::AuthStorage;
use hoocode_code_media::clipboard_image::ClipboardImage;
use hoocode_code_media::rich_clipboard::CopyFlavour;
use hoocode_code_paths::{APP_NAME, VERSION};
use hoocode_code_settings::{ChromeDensity, EditorBorder, ToolOutputView};
use hoocode_code_subagents::agent_log::{agent_log, set_terminal_owned_by_tui};
use hoocode_code_task_store::task_store;
use hoocode_code_tool_bash::BashResult;
use hoocode_code_tui_keybindings::{
    app_key_label, key_hint, key_text, raw_key_hint, AppKeybindingsManager,
};
use hoocode_code_tui_selectors::model_selector::ModelSelectorComponent;
use hoocode_code_tui_selectors::scoped_models_selector::ScopedModelsSelectorComponent;
use hoocode_code_tui_selectors::session_selector::SessionSelectorComponent;
use hoocode_code_tui_selectors::settings_selector::{SettingsChange, SettingsSelectorComponent};
use hoocode_code_tui_selectors::tree_selector::TreeSelectorComponent;
use hoocode_code_tui_selectors::user_message_selector::UserMessageSelectorComponent;
use hoocode_code_tui_theme::{
    get_editor_theme, init_theme, on_theme_change, set_registered_themes, set_theme, theme,
    ThinkingBorderLevel,
};
use hoocode_code_tui_widgets::bash_execution::BashExecutionComponent;
use hoocode_code_tui_widgets::task_panel::{TaskPanelComponent, TaskPanelDensity};
use hoocode_code_tui_widgets::tool_chain_summary::ChainState;
use hoocode_code_tui_widgets::tool_output_view::MAX_TOOL_OUTPUT_VIEW;
use hoocode_code_tui_widgets::UserMessageComponent;
use hoocode_tui_components::{Editor, EditorHost, EditorOptions, FrameBorderStyle, Loader, Spacer};
use hoocode_tui_render::{Component, ComponentHandle, Container, FlexSpacer, Slot, Tui, TuiEvent};
use hoocode_tui_terminal::Terminal;

use crate::changelog::changelog_for_display;
use crate::chrome_layout::{
    ChromeLayoutController, ChromeSurfaces, FooterLayout, TasksLayout, SMALL_TERMINAL_ROWS,
};
use crate::dialog_bridge::{set_dialog_sink, DialogRequest};
use crate::expandable_text::ExpandableText;
use crate::footer::{FooterComponent, FooterDensity};
use crate::footer_data::FooterDataProvider;
use crate::input_frame::set_input_frame_border;
use crate::login_controller::LoginUpdate;
use crate::notification_panel::{NotificationKind, NotificationPanel};
use crate::perf::Perf;
use crate::record_row::RecordRows;
use crate::resource_display::{format_display_path, ResourceListing};
use crate::scroll_view::install_scroll_view;
use crate::startup_progress;
use crate::tips::{
    render_tip, tip_ttl, TipRotation, TipRotationOptions, TipsController, TipsControllerOptions,
};
use crate::wordmark::{build_compact_wordmark, CompactWordmarkOptions};

mod auth;
mod bash;
mod chrome;
mod clipboard;
mod commands;
mod dialogs;
mod events;
mod input;
mod mcp;
mod models;
mod prompt_queue;
mod session_ops;
mod settings;
mod subagents;
mod transcript;
mod tree;

use self::{
    auth::*, chrome::*, commands::*, dialogs::*, input::*, models::*, transcript::*, tree::*,
};

pub use self::events::plan_settle_outcome;
pub use self::input::{at_file_finder, command_path_argument};
pub use self::models::{
    claim_anthropic_subscription_warning, ANTHROPIC_SUBSCRIPTION_AUTH_BODY,
    ANTHROPIC_SUBSCRIPTION_AUTH_TITLE,
};

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
    /// Whether a provider's stored credential is an OAuth token.
    pub is_oauth: Box<dyn Fn(&str) -> bool>,
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
    pub terminal: Option<Box<dyn Terminal>>,
    /// `--perf-log`: one JSON line of the UI's counters a second is appended here.
    pub perf_log: Option<std::path::PathBuf>,
}

const DEFAULT_WORKING_MESSAGE: &str = "Working...";

const DEFAULT_HIDDEN_THINKING_LABEL: &str = "Thinking...";

/// Finished tool blocks kept live; older ones are frozen.
const LIVE_TOOL_WINDOW: usize = 50;

/// Minimum gap between re-renders of the streaming message.
const STREAM_RENDER_THROTTLE: Duration = Duration::from_millis(100);

/// Most keys taken from the input channel in one loop pass. The rest wait for the
/// next pass, so AppEvents and ticks run between batches (concurrency.md, Phase 2).
const MAX_INPUT_PER_ITERATION: usize = 64;

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
    /// The clipboard image read ended.
    PastedImage(Option<ClipboardImage>),
    /// A `/subagent` run ended: its mode, and its summary or error.
    SubagentDone(String, Result<Option<String>, String>),
    /// Queued messages sent after a compaction failed; they go back.
    CompactionQueueFailed(Vec<(String, StreamingBehavior)>, String),
    /// A message typed while streaming could not be queued.
    QueueError(String),
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

fn thinking_border_level(level: &str) -> ThinkingBorderLevel {
    match level {
        "minimal" => ThinkingBorderLevel::Minimal,
        "low" => ThinkingBorderLevel::Low,
        "medium" => ThinkingBorderLevel::Medium,
        "high" => ThinkingBorderLevel::High,
        "xhigh" => ThinkingBorderLevel::Xhigh,
        _ => ThinkingBorderLevel::Off,
    }
}

/// The banner's logo (the compact wordmark, or the name on a narrow screen).
fn logo(columns: u16, version: &str, cwd: &str) -> String {
    let t = theme();
    if columns < 40 {
        return t.bold(&t.fg("accent", APP_NAME)) + &t.fg("dim", &format!(" v{version}"));
    }
    let accent = |s: &str| theme().fg("accent", s);
    let glyph = |s: &str| theme().fg("text", s);
    let dim = |s: &str| theme().fg("dim", s);
    let muted = |s: &str| theme().fg("muted", s);
    let cursor = |s: &str| {
        let t = theme();
        t.blink(&t.fg("accent", s))
    };
    let note = || theme().fg("dim", &format!("  {} more", key_text("app.tools.expand")));
    build_compact_wordmark(&CompactWordmarkOptions {
        app_name: APP_NAME,
        version,
        cwd,
        tagline: None,
        accent: &accent,
        glyph: Some(&glyph),
        dim: &dim,
        muted: &muted,
        cursor: Some(&cursor),
        note: Some(&note),
    })
}

/// The expanded banner: the keys, grouped as the map is.
fn expanded_instructions() -> String {
    let hint = |id: &str, description: &str| key_hint(id, description);
    let dial = |forward: &str, backward: &str, subject: &str| {
        raw_key_hint(
            // " or", not "/": a slash is itself a key (`/` for commands).
            &format!("{} or {}", app_key_label(forward), app_key_label(backward)),
            &format!("to step {subject}"),
        )
    };
    let group = |title: &str| theme().fg("dim", &format!("\n{title}"));
    [
        group("Compose — the message in your hands"),
        hint("app.editor.external", "for external editor"),
        hint("app.input.voiceTranscribe", "to speak instead of type"),
        hint("app.clipboard.pasteImage", "to paste image"),
        hint("app.message.followUp", "to queue follow-up"),
        hint("app.message.dequeue", "to edit all queued messages"),
        raw_key_hint("drop files", "to attach"),
        raw_key_hint("/", "for commands"),
        raw_key_hint("!", "to run bash"),
        raw_key_hint("!!", "to run bash (no context)"),
        group("Steer — what the agent is before it runs"),
        dial(
            "app.mode.cycleForward",
            "app.mode.cycleBackward",
            "agent mode (ask/plan/build/debug)",
        ),
        dial(
            "app.model.cycleForward",
            "app.model.cycleBackward",
            "model — /model to pick one",
        ),
        dial(
            "app.thinking.cycleForward",
            "app.thinking.cycleBackward",
            "thinking level",
        ),
        group("Read — what you see of what it did"),
        dial(
            "app.view.cycleForward",
            "app.view.cycleBackward",
            "tool output (radar/peek/full)",
        ),
        dial(
            "app.tasks.cycleForward",
            "app.tasks.cycleBackward",
            "task panel view",
        ),
        hint("app.tools.expand", "to jump to full output and back"),
        hint("app.thinking.toggle", "to show or hide thinking"),
        group("Go — sessions and places"),
        hint("app.session.resume", "to resume a session"),
        hint("app.session.changeDirectory", "to change working directory"),
        dial(
            "app.session.color.cycleForward",
            "app.session.color.cycleBackward",
            "session colour",
        ),
        hint("app.settings.open", "for settings"),
        hint("app.hotkeys.open", "for all shortcuts"),
        group("Flow — getting out, getting back"),
        hint("app.interrupt", "to interrupt"),
        hint("app.clear", "to clear"),
        raw_key_hint(&format!("{} twice", key_text("app.clear")), "to exit"),
        hint("app.exit", "to exit (empty)"),
        hint("app.suspend", "to suspend"),
        key_hint("tui.editor.deleteToLineEnd", "to delete to end"),
    ]
    .join("\n")
}

struct Mode {
    /// The chat transcript and the components that track it (`transcript.rs`).
    transcript: Transcript,
    session: AgentSession,
    runtime: tokio::runtime::Handle,
    tui: Tui,
    verbose: bool,
    size: Rc<Cell<(u16, u16)>>,
    dirty: Rc<Cell<bool>>,
    perf: Perf,
    header: Rc<RefCell<ExpandableText>>,
    status: Rc<RefCell<Container>>,
    loader: Option<Rc<RefCell<Loader>>>,
    stream_render_at: Option<Instant>,
    stream_render_pending: bool,
    scheduler_tick_at: Instant,
    turn_cost_anchor: Option<(AssistantUsageTotals, Instant)>,
    turn_stop_reason: Option<StopReason>,
    tool_output_view: ToolOutputView,
    view_before_jump: Option<ToolOutputView>,
    hide_thinking_block: bool,
    chain_closed_for_current_message: bool,
    dial_reverse_taught: HashSet<&'static str>,
    last_tool_tick: Instant,
    show_images: bool,
    image_width_cells: u32,
    code_block_indent: String,
    editor: Rc<RefCell<CustomEditor>>,
    editor_container: Rc<RefCell<Container>>,
    selector: Option<OpenSelector>,
    actions: Rc<RefCell<Vec<Action>>>,
    notifications: Rc<RefCell<NotificationPanel>>,
    tips: TipsController,
    footer: Rc<RefCell<FooterComponent>>,
    runtime_notice: Option<String>,
    shedding_shown: bool,
    footer_data: FooterDataProvider,
    chrome: ChromeLayoutController,
    expanded: bool,
    last_sigint: Option<Instant>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    exit_requested: bool,
    listing: Box<dyn Fn(&AgentSession) -> ResourceListing>,
    session_runtime: Option<AgentSessionRuntime>,
    subscription: Option<hoocode_code_agent_session::SessionSubscription>,
    is_oauth: Rc<dyn Fn(&str) -> bool>,
    auth_storage: Arc<AuthStorage>,
    login: Option<LoginStep>,
    session_selector: Option<Rc<RefCell<SessionSelectorComponent>>>,
    pending_cwd_prompt: Option<(PathBuf, String, mpsc::Receiver<Option<String>>)>,
    ask_options: Option<OpenAskOptions>,
    tree_selector: Option<(Rc<RefCell<TreeSelectorComponent>>, Option<String>)>,
    model_selector: Option<Rc<RefCell<ModelSelectorComponent>>>,
    scoped_models_selector: Option<(Rc<RefCell<ScopedModelsSelectorComponent>>, usize)>,
    anthropic_warning_shown: bool,
    settings_selector: Option<Rc<RefCell<SettingsSelectorComponent>>>,
    settings_changes: Rc<RefCell<Vec<SettingsChange>>>,
    last_escape: Option<Instant>,
    pending_tree_summary: Option<PendingTreeAnswer>,
    pending_tree_instructions: Option<PendingTreeAnswer>,
    tree_navigation: Option<TreeNavigation>,
    editor_dialog: Option<OpenEditorDialog>,
    pending_messages: Rc<RefCell<Container>>,
    is_bash_mode: bool,
    bash_component: Option<Rc<RefCell<BashExecutionComponent>>>,
    pending_bash_components: Vec<Rc<RefCell<BashExecutionComponent>>>,
    compaction_loader: Option<Rc<RefCell<Loader>>>,
    compaction_queue: Vec<(String, StreamingBehavior)>,
    task_panel: Rc<RefCell<TaskPanelComponent>>,
    task_store_subscription: Option<hoocode_code_task_store::Subscription<'static>>,
    fork_selector: Option<Rc<RefCell<UserMessageSelectorComponent>>>,
    previous_cwd: Rc<RefCell<Option<PathBuf>>>,
    pending_import: Option<(String, Option<String>, mpsc::Receiver<Option<String>>)>,
    restarted_input: Option<Receiver<TuiEvent>>,
}

/// `InteractiveMode.run`: until the user exits.
pub fn run_interactive(options: InteractiveOptions) -> Result<(), String> {
    let initial = (
        options.initial_message.clone(),
        options.initial_images.clone(),
        options.initial_messages.clone(),
        options.model_fallback_message.clone(),
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
            compaction_enabled,
            tool_output_view,
            chrome_density,
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
            settings.compaction_enabled(),
            settings.tool_output_view(),
            settings.chrome_density(),
            settings.hide_thinking_block(),
            settings.code_block_indent(),
            settings.show_images(),
            settings.image_width_cells() as u32,
        );
        drop(settings);
        let terminal = options
            .terminal
            .unwrap_or_else(|| Box::new(hoocode_tui_terminal::ProcessTerminal::new()));
        let size = Rc::new(Cell::new((terminal.columns(), terminal.rows())));
        let perf = Perf::new(options.perf_log.as_deref());
        let mut tui = Tui::new(terminal, Some(show_hardware_cursor));
        tui.set_clear_on_shrink(clear_on_shrink);

        let keybindings = AppKeybindingsManager::create(None);
        keybindings.install();

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
                border: Some(border),
            },
        );
        editor.prompt_prefix = "❯".into();
        let sink = actions.clone();
        editor.on_submit = Some(Box::new(move |text: &str| {
            sink.borrow_mut().push(Action::Submit(text.to_string()))
        }));
        let sink = actions.clone();
        editor.on_change = Some(Box::new(move |text: &str| {
            sink.borrow_mut()
                .push(Action::EditorChanged(text.to_string()))
        }));
        let sink = actions.clone();
        editor.on_autocomplete_visibility_change = Some(Box::new(move |visible| {
            sink.borrow_mut()
                .push(Action::AutocompleteVisibility(visible))
        }));
        let editor = handle(CustomEditor {
            editor,
            actions: actions.clone(),
        });

        let footer_data = FooterDataProvider::new(session.cwd());
        footer_data
            .set_subagent_enabled(session.get_active_tool_names().iter().any(|t| t == "Agent"));
        let is_oauth: Rc<dyn Fn(&str) -> bool> = Rc::from(options.is_oauth);
        let mut footer = FooterComponent::new(
            Box::new(SessionFooter {
                session: session.clone(),
                is_oauth: is_oauth.clone(),
            }),
            footer_data.clone(),
        );
        footer.set_auto_compact_enabled(compaction_enabled);
        footer.set_tool_output_view(tool_output_view);
        let footer = handle(footer);
        let footer_slot = handle(Slot::new(as_component(&footer)));
        let task_panel = handle(TaskPanelComponent::new());
        let tasks_slot = handle(Slot::new(as_component(&task_panel)));
        let panel_for_density = task_panel.clone();
        let footer_for_density = footer.clone();
        let density = chrome_density.unwrap_or(if size.get().1 < SMALL_TERMINAL_ROWS {
            ChromeDensity::Compact
        } else {
            ChromeDensity::Full
        });
        let mut chrome = ChromeLayoutController::new(
            ChromeSurfaces {
                footer_slot: footer_slot.clone(),
                tasks_slot: tasks_slot.clone(),
                set_footer_density: Box::new(move |d| {
                    footer_for_density
                        .borrow_mut()
                        .set_density(if d == FooterLayout::Line {
                            FooterDensity::Line
                        } else {
                            FooterDensity::Full
                        })
                }),
                set_tasks_density: Box::new(move |d| {
                    panel_for_density
                        .borrow_mut()
                        .set_density(if d == TasksLayout::Summary {
                            TaskPanelDensity::Summary
                        } else {
                            TaskPanelDensity::Full
                        })
                }),
            },
            density,
        );
        chrome.apply();

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

        let expanded = options.verbose || tool_output_view == MAX_TOOL_OUTPUT_VIEW;
        let (version, cwd) = (
            options.version.clone(),
            format_display_path(&session.cwd().to_string_lossy()),
        );
        let columns = size.get().0;
        let (v1, c1, v2, c2) = (version.clone(), cwd.clone(), version.clone(), cwd.clone());
        let header = handle(ExpandableText::new(
            move || logo(columns, &v1, &c1),
            move || {
                let onboarding = theme().fg(
                    "dim",
                    &format!(
                        "{APP_NAME} can explain its own features and look up its docs. Ask it how to use or extend {APP_NAME}."
                    ),
                );
                format!(
                    "{}\n{}\n\n{onboarding}",
                    logo(columns, &v2, &c2),
                    expanded_instructions()
                )
            },
            expanded,
            0,
            0,
        ));

        let chat = handle(Container::new());
        let header_container = handle(Container::new());
        header_container
            .borrow_mut()
            .add_child(as_component(&header));
        let screen_fill = handle(FlexSpacer::new());
        let widget_above = handle(Container::new());
        widget_above
            .borrow_mut()
            .add_child(as_component(&handle(Spacer::new(1))));
        let editor_container = handle(Container::new());
        editor_container
            .borrow_mut()
            .add_child(as_component(&editor));

        tui.add_child(as_component(&header_container));
        tui.add_child(as_component(&screen_fill));
        tui.set_flex_spacer(Some(screen_fill.clone()));
        tui.add_child(as_component(&chat));
        let pending_messages = handle(Container::new());
        tui.add_child(as_component(&pending_messages));
        let status = handle(Container::new());
        tui.add_child(as_component(&status));
        tui.add_child(as_component(&widget_above));
        tui.add_child(tasks_slot.clone());
        tui.add_child(as_component(&notifications));
        tui.add_child(as_component(&editor_container));
        tui.add_child(as_component(&handle(Container::new()))); // widgets below
        tui.add_child(footer_slot.clone());
        tui.set_focus(Some(as_component(&editor)));
        // Scrolling the transcript: the prompt's pager keys, and the keys of
        // the pinned view (`scroll_view.rs`).
        install_scroll_view(
            &mut tui,
            as_component(&editor),
            chat.clone(),
            Rc::new(|child: &ComponentHandle| {
                child
                    .borrow()
                    .as_any()
                    .is_some_and(|any| any.is::<UserMessageComponent>())
            }),
        );

        let (tx, rx) = mpsc::channel();
        Self {
            session,
            runtime: options.runtime,
            tui,
            verbose: options.verbose,
            size,
            dirty,
            perf,
            header,
            transcript: Transcript::new(chat),
            status,
            loader: None,
            stream_render_at: None,
            stream_render_pending: false,
            scheduler_tick_at: Instant::now() + hoocode_code_scheduler::TICK_INTERVAL,
            turn_cost_anchor: None,
            turn_stop_reason: None,
            tool_output_view,
            view_before_jump: None,
            hide_thinking_block,
            chain_closed_for_current_message: false,
            dial_reverse_taught: HashSet::new(),
            last_tool_tick: Instant::now(),
            show_images,
            image_width_cells,
            code_block_indent,
            editor,
            editor_container,
            selector: None,
            actions,
            notifications,
            tips,
            footer,
            runtime_notice: None,
            shedding_shown: false,
            footer_data,
            chrome,
            expanded,
            last_sigint: None,
            tx,
            rx,
            exit_requested: false,
            listing: options.listing,
            session_runtime: options.session_runtime,
            subscription: None,
            is_oauth,
            auth_storage: options.auth_storage,
            login: None,
            session_selector: None,
            pending_cwd_prompt: None,
            ask_options: None,
            tree_selector: None,
            model_selector: None,
            scoped_models_selector: None,
            anthropic_warning_shown: false,
            settings_selector: None,
            settings_changes: Rc::default(),
            last_escape: None,
            pending_tree_summary: None,
            pending_tree_instructions: None,
            tree_navigation: None,
            editor_dialog: None,
            pending_messages,
            is_bash_mode: false,
            bash_component: None,
            pending_bash_components: Vec::new(),
            compaction_loader: None,
            compaction_queue: Vec::new(),
            task_panel,
            task_store_subscription: None,
            fork_selector: None,
            previous_cwd: Rc::new(RefCell::new(None)),
            pending_import: None,
            restarted_input: None,
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
        self.tui.accept_event(event);
    }

    /// Acts on the watchdog once per loop turn. A hard-limit trip aborts the turn
    /// and flushes the session. Shedding changes clear the render caches. The
    /// footer warning follows the stall flag and shedding. The stall flag can only
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
            Some("UI stalled: no response for 2 s; keys are queued".to_string())
        } else if shedding {
            Some(
                "Memory above the soft limit: no new subagents; tool calls run one at a time"
                    .to_string(),
            )
        } else {
            None
        };
        if notice != self.runtime_notice {
            self.footer.borrow_mut().set_notice(notice.clone());
            self.runtime_notice = notice;
            self.dirty.set(true);
        }
    }

    fn next_wakeup(&self) -> Duration {
        let now = Instant::now();
        let mut wait = Duration::from_millis(250);
        let deadlines = [
            self.notifications.borrow().deadline(),
            self.tips.deadline(),
            self.editor.borrow().editor.autocomplete_deadline(),
        ];
        for deadline in deadlines.into_iter().flatten() {
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
        if let Some(deadline) = self
            .selector
            .as_ref()
            .and_then(|(s, _)| s.borrow().deadline())
        {
            wait = wait.min(deadline.saturating_duration_since(now));
        }
        if let Some(deadline) = self
            .fork_selector
            .as_ref()
            .and_then(|s| s.borrow().deadline())
        {
            wait = wait.min(deadline.saturating_duration_since(now));
        }
        if let Some(deadline) = self
            .tree_selector
            .as_ref()
            .and_then(|(s, _)| s.borrow().deadline())
        {
            wait = wait.min(deadline.saturating_duration_since(now));
        }
        if self.stream_render_pending {
            if let Some(at) = self.stream_render_at {
                wait = wait.min((at + STREAM_RENDER_THROTTLE).saturating_duration_since(now));
            }
        }
        wait.max(Duration::from_millis(1))
    }

    fn run(
        mut self,
        options_initial: (
            Option<String>,
            Vec<ImageContent>,
            Vec<String>,
            Option<String>,
        ),
    ) -> Result<(), String> {
        self.tui
            .set_frame_observer(Some(self.perf.frame_observer()));
        let mut input = self.tui.start();
        // Ctrl+C aborts the turn from the input thread, without waiting for this loop.
        let interrupt_agent = self.session.agent().clone();
        hoocode_tui_terminal::interrupt::set_hook(Some(Arc::new(move || interrupt_agent.abort())));
        // From here the TUI owns the terminal: the agent's operational log
        // lines (dispatch, warm fallback, lifeguard) must not write to it.
        set_terminal_owned_by_tui(true);
        let tx = self.tx.clone();
        self.task_store_subscription = Some(task_store().subscribe(move || {
            let _ = tx.send(AppEvent::Rerender);
        }));
        self.subscription = Some(self.subscribe());

        // Everything the chrome shows about the session.
        self.update_editor_border_color();
        self.update_session_chip();
        self.update_terminal_title();
        let theme_name = self.session.settings().theme();
        if let Some(name) = theme_name {
            if let Err(error) = set_theme(&name, true) {
                self.show_error(&format!(
                    "Failed to load theme \"{name}\": {error}\nFell back to dark theme."
                ));
            }
        }
        let changelog = {
            let has_messages = !self.session.messages().is_empty();
            let mut settings = self.session.settings();
            changelog_for_display(has_messages, &mut settings, VERSION)
        };
        self.render_resources();
        self.show_startup_notices(changelog);
        // Messages after the resource listing, as the pin orders them.
        self.render_initial_messages();
        self.setup_autocomplete_provider();
        self.update_available_provider_count();
        self.maybe_warn_about_anthropic_subscription_auth(None);

        let tx = Mutex::new(self.tx.clone());
        set_dialog_sink(Some(Box::new(move |request| {
            tx.lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(AppEvent::Dialog(request))
                .is_ok()
        })));
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

        let (initial_message, initial_images, initial_messages, fallback) = options_initial;
        if let Some(message) = fallback {
            self.show_error(&message);
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
        loop {
            hoocode_runtime::watchdog::ui_beat("ui-loop:wait");
            let received = input.recv_timeout(self.next_wakeup());
            // Phase 0 timing: an iteration starts when its input has arrived,
            // not while the loop sleeps.
            let iteration_started = Instant::now();
            // Heartbeat for the input thread's stall check (Ctrl+C twice = emergency exit).
            hoocode_tui_terminal::interrupt::ui_beat();
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
            self.sync_runtime_health();
            if let Some(restarted) = self.restarted_input.take() {
                input = restarted;
            }
            while let Ok(event) = self.rx.try_recv() {
                match event {
                    AppEvent::Session(event, anchor) => {
                        if self.turn_cost_anchor.is_none() {
                            self.turn_cost_anchor = anchor;
                        }
                        self.handle_session_event(*event)
                    }
                    AppEvent::PromptDone(result) => {
                        // The request has ended (retries and continuations
                        // included): settle it (`settleRequestOnIdle`). The
                        // last chain has nothing after it to close it.
                        let outcome = if self.turn_stop_reason == Some(StopReason::Stop) {
                            ChainState::Done
                        } else {
                            ChainState::Interrupted
                        };
                        self.close_open_chain(outcome);
                        self.show_turn_cost();
                        self.settle_dangling_plan_items();
                        self.tips.on_turn_end();
                        if let Err(error) = result {
                            self.show_error(&error);
                        }
                        if let Some(next) = queued.pop() {
                            self.prompt(next);
                        }
                    }
                    AppEvent::Rerender => self.dirty.set(true),
                    AppEvent::Dialog(DialogRequest::Select {
                        title,
                        options,
                        reply,
                    }) => self.show_selector(&title, options, reply),
                    AppEvent::Dialog(DialogRequest::Notify(message)) => self.show_record(&message),
                    AppEvent::Dialog(DialogRequest::AskOptions { questions, reply }) => {
                        self.show_ask_options(questions, reply)
                    }
                    AppEvent::Dialog(DialogRequest::HideAskOptions) => self.hide_ask_options(None),
                    AppEvent::Login(update) => self.handle_login_update(update),
                    AppEvent::BashChunk(chunk) => {
                        if let Some(component) = &self.bash_component {
                            component.borrow_mut().append_output(&chunk);
                            self.dirty.set(true);
                        }
                    }
                    AppEvent::BashDone(result) => self.finish_bash_command(result),
                    AppEvent::CopyDone(subject, result) => self.finish_copy(&subject, result),
                    AppEvent::PastedImage(image) => self.insert_pasted_image(image),
                    AppEvent::SubagentDone(mode, result) => self.finish_subagent(&mode, result),
                    AppEvent::CompactionQueueFailed(queued, error) => {
                        self.session.clear_queue();
                        self.compaction_queue = queued;
                        self.update_pending_messages_display();
                        self.show_error(&error);
                    }
                    AppEvent::QueueError(error) => self.show_error(&error),
                    AppEvent::ThemeChanged => {
                        self.tui.invalidate();
                        self.update_editor_border_color();
                        self.update_session_chip();
                    }
                }
            }
            if self.notifications.borrow_mut().poll() {
                self.dirty.set(true);
            }
            if self.tips.poll() {
                self.dirty.set(true);
            }
            if self
                .selector
                .as_ref()
                .is_some_and(|(s, _)| s.borrow_mut().poll())
            {
                self.dirty.set(true);
            }
            if self.last_tool_tick.elapsed() >= Duration::from_secs(1) {
                self.last_tool_tick = Instant::now();
                if self.task_panel.borrow().ticking() {
                    self.dirty.set(true);
                }
                for block in self.transcript.pending_tools.values() {
                    if block.borrow().is_ticking() {
                        block.borrow_mut().invalidate();
                        self.dirty.set(true);
                    }
                }
            }
            if self
                .session_selector
                .as_ref()
                .is_some_and(|s| s.borrow_mut().poll())
            {
                self.dirty.set(true);
            }
            self.poll_cwd_prompt();
            self.poll_import_confirm();
            self.poll_tree_selector();
            self.poll_fork_selector();
            self.poll_model_selectors();
            self.poll_login();
            self.poll_settings_selector();
            self.poll_tree_summary();
            self.poll_tree_instructions();
            self.poll_tree_navigation();
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
            if self.exit_requested {
                break;
            }
            // Input and resizes this pass, and anything else that changed, render once here.
            if self.dirty.replace(false) {
                self.tui.request_render(false);
            }
            self.tui.flush_scheduled_render();
            self.perf.end_iteration(iteration_started);
        }

        hoocode_runtime::watchdog::ui_stopped();
        set_dialog_sink(None);
        if let Some((_, reply)) = self.selector.take() {
            let _ = reply.send(None);
        }
        if let Some((_, reply)) = self.ask_options.take() {
            let _ = reply.send(None);
        }
        if let Some((_, reply)) = self.editor_dialog.take() {
            let _ = reply.send(None);
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
        self.tui.stop();
        hoocode_tui_terminal::interrupt::set_hook(None);
        set_terminal_owned_by_tui(false);
        let session = self.session.clone();
        if session.is_streaming() {
            self.runtime.block_on(async move { session.abort().await });
        }
        self.session.dispose();
        Ok(())
    }
}

#[cfg(test)]
mod expanded_instructions_tests {
    use super::*;

    #[test]
    fn dial_hints_join_their_keys_with_or_not_a_slash() {
        init_theme(Some("dark"), false);
        let text = hoocode_tui_util::strip_vt_control_characters(&expanded_instructions());
        let line = text
            .lines()
            .find(|l| l.contains("to step thinking level"))
            .expect("thinking dial line");
        let keys = line.split(" to step").next().unwrap_or_default();
        assert!(keys.contains(" or "), "keys joined with or: {line}");
        assert!(!keys.contains('/'), "no slash between keys: {line}");
    }
}
