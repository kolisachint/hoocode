//! Component goldens: the `/settings` pane as the SYSTEMS console and its
//! theme submenu as the HULL console, at 100 and 80 columns. Hints use
//! literal keys (no `alt`), so the bytes hold on every platform. Accept a
//! deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-selectors systems_console`.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_code_tools::light::PromptSurface;
use hoocode_code_tui_selectors::settings_selector::{
    submenu_list, DoneSlot, SettingsChange, SettingsConfig, SettingsSelectorComponent,
    ToolToggleInfo,
};
use hoocode_tui_components::SubmenuOutcome;
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{Component, ComponentHandle};

use crate::support::lock;

fn surface() -> PromptSurface {
    PromptSurface {
        system_prompt_tokens: 1430,
        tool_schema_tokens: 2710,
        total_tokens: 4140,
        tools: vec![("Read".into(), 162), ("Shell".into(), 128)],
    }
}

fn themes() -> Vec<String> {
    [
        "dark",
        "light",
        "solarized-dark",
        "solarized-light",
        "colorsafe-dark",
        "colorsafe-light",
        "vox-cutout-dark",
        "vox-cutout-light",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn pane(changes: Rc<RefCell<Vec<SettingsChange>>>) -> SettingsSelectorComponent {
    let sink = changes.clone();
    SettingsSelectorComponent::new(
        SettingsConfig {
            tools: vec![ToolToggleInfo {
                name: "Read".into(),
                enabled: true,
                tokens: Some(162),
            }],
            current_theme: "dark".into(),
            available_themes: themes(),
            measure_token_surface: Some(Rc::new(surface)),
            tips_enabled: false,
            ..SettingsConfig::default()
        },
        move |change| sink.borrow_mut().push(change),
    )
}

/// Raw input for moving down, the app's arrow sequence.
const DOWN: &str = "\x1b[B";

fn check_systems(name: &str, width: u16) {
    let _g = lock();
    let mut pane = pane(Rc::default());
    let rendered = render_golden(&mut pane, width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        text.contains("SYSTEMS · /settings  4,140 tok/turn"),
        "{name}: missing title and count in\n{text}"
    );
    assert!(
        text.lines().any(|l| l.contains('›') && l.contains("Tools")),
        "{name}: missing selected row in\n{text}"
    );
    assert!(
        text.lines().any(|l| l.contains('▸')),
        "{name}: missing query row in\n{text}"
    );
    assert!(
        text.contains("Per-turn surface: 4,140 tokens"),
        "{name}: missing surface row in\n{text}"
    );
    assert!(
        text.contains("enter open"),
        "{name}: missing hint foot in\n{text}"
    );
    assert_golden!(name, rendered);
}

#[test]
fn systems_100() {
    check_systems("systems_100", 100);
}

#[test]
fn systems_80() {
    check_systems("systems_80", 80);
}

/// Typing narrows the pane to the matching rows.
#[test]
fn systems_search_narrows_the_rows() {
    let _g = lock();
    let mut pane = pane(Rc::default());
    pane.settings_list().borrow_mut().apply_filter("theme");
    let rendered = render_golden(&mut pane, 100);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(text.contains("Interface"), "missing category in\n{text}");
    assert!(!text.contains("Tools"), "unfiltered row in\n{text}");
}

/// Open the Interface category, then its Theme submenu: the HULL console
/// handle and its outcome slot.
fn open_hull(pane: &mut SettingsSelectorComponent) -> (ComponentHandle, DoneSlot) {
    pane.settings_list().borrow_mut().apply_filter("Interface");
    pane.handle_input("\r");
    let handle = pane
        .settings_list()
        .borrow()
        .open_submenu()
        .expect("the Interface submenu");
    let cat = submenu_list(&handle).expect("a settings-list submenu");
    let slot: DoneSlot = Rc::default();
    let hull = {
        let cat = cat.borrow();
        let item = cat.items().iter().find(|i| i.id == "theme").expect("theme");
        (item.submenu.as_ref().expect("factory"))(&item.current_value.clone(), slot.clone())
    };
    (hull, slot)
}

fn check_hull(name: &str, width: u16) {
    let _g = lock();
    let mut pane = pane(Rc::default());
    let (hull, _) = open_hull(&mut pane);
    let rendered = render_golden(&mut *hull.borrow_mut(), width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    assert!(
        text.contains("HULL · /theme  8 themes"),
        "{name}: missing title and count in\n{text}"
    );
    assert!(
        text.lines()
            .any(|l| l.contains('›') && l.contains("dark") && l.contains("● in use")),
        "{name}: missing selected theme in\n{text}"
    );
    assert!(
        text.contains("enter keep"),
        "{name}: missing hint foot in\n{text}"
    );
    assert_golden!(name, rendered);
}

#[test]
fn hull_100() {
    check_hull("hull_100", 100);
}

#[test]
fn hull_80() {
    check_hull("hull_80", 80);
}

/// Moving the cursor previews the theme at once.
#[test]
fn hull_move_previews_live() {
    let _g = lock();
    let changes: Rc<RefCell<Vec<SettingsChange>>> = Rc::default();
    let mut pane = pane(changes.clone());
    let (hull, _) = open_hull(&mut pane);
    hull.borrow_mut().handle_input(DOWN);
    assert_eq!(
        *changes.borrow(),
        vec![SettingsChange::ThemePreview("light".to_string())]
    );
}

/// Enter keeps the previewed theme, through the result slot.
#[test]
fn hull_enter_keeps() {
    let _g = lock();
    let changes: Rc<RefCell<Vec<SettingsChange>>> = Rc::default();
    let mut pane = pane(changes.clone());
    let (hull, slot) = open_hull(&mut pane);
    hull.borrow_mut().handle_input(DOWN);
    hull.borrow_mut().handle_input("\r");
    assert_eq!(
        *changes.borrow(),
        vec![
            SettingsChange::ThemePreview("light".to_string()),
            SettingsChange::Theme("light".to_string()),
        ]
    );
    assert_eq!(
        *slot.borrow(),
        Some(SubmenuOutcome::Selected("light".to_string()))
    );
}

/// Esc reverts to the theme the list opened on.
#[test]
fn hull_esc_reverts() {
    let _g = lock();
    let changes: Rc<RefCell<Vec<SettingsChange>>> = Rc::default();
    let mut pane = pane(changes.clone());
    let (hull, _) = open_hull(&mut pane);
    hull.borrow_mut().handle_input(DOWN);
    hull.borrow_mut().handle_input("\x1b");
    assert_eq!(
        *changes.borrow(),
        vec![
            SettingsChange::ThemePreview("light".to_string()),
            SettingsChange::ThemePreview("dark".to_string()),
        ]
    );
}
