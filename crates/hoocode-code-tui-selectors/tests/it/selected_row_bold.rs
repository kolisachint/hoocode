//! The selected row's name is bold in every pane (UI-32): the Settings,
//! Sessions and Fork panes, in colour and in monochrome (bold is a modifier,
//! so it stays with the colour off). Each test reads the golden's style
//! section and checks that the name's cells on the selected row are bold,
//! and that an unselected row's name is not.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use chrono::{DateTime, Utc};
use hoocode_code_session::SessionInfo;
use hoocode_code_tools::light::PromptSurface;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_selectors::session_selector::{
    LoadSink, SessionSelectorComponent, SessionSelectorOptions,
};
use hoocode_code_tui_selectors::settings_selector::{
    SettingsChange, SettingsConfig, SettingsSelectorComponent, ToolToggleInfo,
};
use hoocode_code_tui_selectors::user_message_selector::{
    UserMessageItem, UserMessageSelectorComponent,
};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::Component;

use crate::support::{agent_dir, lock, look, Look};

/// The row numbers and bold columns of a golden's style section: `rN cA-B …
/// bold` lines, returned as (row, column) pairs that are bold.
fn bold_cells(rendered: &str) -> HashSet<(usize, usize)> {
    let mut out = HashSet::new();
    let Some((_, styles)) = rendered.split_once("--- styles ---") else {
        return out;
    };
    for line in styles.lines() {
        let mut tokens = line.split_whitespace();
        let Some(row) = tokens.next().and_then(|t| t.strip_prefix('r')) else {
            continue;
        };
        let Ok(row) = row.parse::<usize>() else {
            continue;
        };
        let Some(cols) = tokens.next().and_then(|t| t.strip_prefix('c')) else {
            continue;
        };
        if !line.split_whitespace().any(|t| t == "bold") {
            continue;
        }
        let (start, end) = match cols.split_once('-') {
            Some((a, b)) => (a.parse::<usize>(), b.parse::<usize>()),
            None => (cols.parse::<usize>(), cols.parse::<usize>()),
        };
        if let (Ok(start), Ok(end)) = (start, end) {
            for col in start..=end {
                out.insert((row, col));
            }
        }
    }
    out
}

/// Whether every cell of `name` on the first text line containing `name`
/// is bold. Returns `None` when the name is not drawn.
fn name_bold(rendered: &str, name: &str) -> Option<bool> {
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    let (row, line) = text
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(name))?;
    let start = line.find(name)?;
    let start = line[..start].chars().count();
    let bold = bold_cells(rendered);
    Some((start..start + name.chars().count()).all(|col| bold.contains(&(row, col))))
}

/// The text line of the selected row (the one with `›`) that carries `name`.
fn selected_line_has(rendered: &str, name: &str) -> bool {
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    text.lines()
        .any(|line| line.contains('›') && line.contains(name))
}

/// Renders `component` in each look and checks the selected name is bold and
/// the unselected name is not.
fn check_pane(name: &str, make: impl Fn() -> Box<dyn Component>, selected: &str, unselected: &str) {
    for look_ in [Look::Dark, Look::Mono] {
        let _g = lock();
        look(look_);
        let mut component = make();
        let rendered = render_golden(component.as_mut(), 100);
        assert!(
            selected_line_has(&rendered, selected),
            "{name} {look_:?}: selected row missing in\n{rendered}"
        );
        assert_eq!(
            name_bold(&rendered, selected),
            Some(true),
            "{name} {look_:?}: selected name is not bold in\n{rendered}"
        );
        assert_eq!(
            name_bold(&rendered, unselected),
            Some(false),
            "{name} {look_:?}: unselected name is bold in\n{rendered}"
        );
    }
}

fn surface() -> PromptSurface {
    PromptSurface {
        system_prompt_tokens: 1430,
        tool_schema_tokens: 2710,
        total_tokens: 4140,
        tools: vec![("Read".into(), 162), ("Shell".into(), 128)],
    }
}

fn session(id: &str, name: Option<&str>, first_message: &str) -> SessionInfo {
    SessionInfo {
        path: PathBuf::from(format!("/tmp/{id}.jsonl")),
        id: id.to_string(),
        cwd: String::new(),
        name: name.map(str::to_string),
        color: None,
        branch: None,
        parent_session_path: None,
        created: DateTime::<Utc>::UNIX_EPOCH,
        modified: DateTime::<Utc>::from_timestamp(4_102_444_800, 0).unwrap_or_default(),
        message_count: 5,
        model: None,
        all_messages_text: first_message.to_string(),
        subagent: None,
        first_message: first_message.to_string(),
    }
}

#[test]
fn sessions_pane_selected_name_is_bold() {
    check_pane(
        "sessions",
        || {
            let sessions = vec![
                session("a", Some("wrap width"), "why does wrap_text ignore width"),
                session("b", None, "add a width check to wrap_text in src/wrap.rs"),
            ];
            let mut s = SessionSelectorComponent::new(
                Box::new(move |sink: LoadSink| sink.finish(Ok(sessions.clone()))),
                Box::new(|sink: LoadSink| sink.finish(Ok(Vec::new()))),
                Box::new(|_| {}),
                Box::new(|| {}),
                SessionSelectorOptions {
                    keybindings: Some(Rc::new(
                        AppKeybindingsManager::create(Some(agent_dir())).into_manager(),
                    )),
                    ..Default::default()
                },
                None,
            );
            s.poll();
            Box::new(s)
        },
        "wrap width",
        "add a width check",
    );
}

#[test]
fn settings_pane_selected_name_is_bold() {
    check_pane(
        "settings",
        || {
            let changes: Rc<RefCell<Vec<SettingsChange>>> = Rc::default();
            let sink = changes.clone();
            let pane = SettingsSelectorComponent::new(
                SettingsConfig {
                    tools: vec![ToolToggleInfo {
                        name: "Read".into(),
                        enabled: true,
                        tokens: Some(162),
                    }],
                    current_theme: "dark".into(),
                    available_themes: vec!["dark".into(), "light".into()],
                    measure_token_surface: Some(Rc::new(surface)),
                    tips_enabled: false,
                    ..SettingsConfig::default()
                },
                move |change| sink.borrow_mut().push(change),
            );
            Box::new(pane)
        },
        "Tools",
        "Tool output",
    );
}

#[test]
fn fork_pane_selected_name_is_bold() {
    check_pane(
        "fork",
        || {
            let item = |id: &str, text: &str| UserMessageItem {
                id: id.into(),
                text: text.into(),
                timestamp: None,
            };
            let items = vec![
                item("m1", "what does the wrap crate do?"),
                item("m2", "add a width check to wrap_text in src/wrap.rs"),
                item("m3", "lorem ipsum dolor sit amet"),
            ];
            Box::new(UserMessageSelectorComponent::new(items, Some("m2")))
        },
        "add a width check",
        "lorem ipsum",
    );
}
