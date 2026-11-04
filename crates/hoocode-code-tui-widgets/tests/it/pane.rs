//! Component goldens for the pane primitives (declutter spec v2.4, v2.9):
//! a plain list, the filter (input) row, the hint row, the split list |
//! detail at 100 columns, the same pane narrow (detail stacked), the warn
//! chip, and the monochrome borders. Accept a deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-code-tui-widgets pane`.

use hoocode_code_tui_widgets::pane::{
    capped_hint_line, hint_line, input_line, Chip, NarrowDetail, Pane, PaneStyle,
};
use hoocode_tui_ratatui::{Line, Modifier, Span};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component};

use crate::support::lock;

/// A pane drawn as a component.
struct Drawn(Pane);

impl Component for Drawn {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.0.render(width as usize)
    }
}

fn dim(st: &PaneStyle, text: &str) -> Span<'static> {
    Span::styled(text.to_string(), st.dim_style())
}

fn text(st: &PaneStyle, text: &str) -> Span<'static> {
    Span::styled(text.to_string(), st.text_style())
}

/// The Models pane of the mockups: two rows, the first selected.
fn models(st: PaneStyle) -> Pane {
    let mut pane = Pane::new("Models");
    pane.command = "/model".to_string();
    pane.facts = Line::from(vec![dim(&st, "showing all · 2 signed in")]);
    pane.rows = vec![
        Line::from(vec![text(&st, "mock-large   "), text(&st, "free")]),
        Line::from(vec![text(&st, "mock-fast    "), dim(&st, "free")]),
    ];
    pane.selected = Some(0);
    pane.style = Some(st);
    pane
}

fn models_detail(st: &PaneStyle) -> Vec<Line<'static>> {
    let row = |key: &str, value: &str| Line::from(vec![dim(st, key), text(st, value)]);
    vec![
        Line::from(vec![
            Span::styled(
                "mock-large".to_string(),
                st.text_style().add_modifier(Modifier::BOLD),
            ),
            dim(st, "  in use"),
        ]),
        row("provider  ", "mock"),
        row("context   ", "128k"),
        row("price     ", "free"),
        row("thinking  ", "supported"),
    ]
}

fn models_hints() -> Line<'static> {
    capped_hint_line(
        &[
            ("↑↓", "move"),
            ("enter", "pick"),
            ("tab", "all / scoped"),
            ("esc", "close"),
        ],
        false,
    )
}

fn check(name: &str, pane: Pane, width: u16) {
    let rendered = render_golden(&mut Drawn(pane), width);
    assert_golden!(name, rendered);
}

/// Title chip, the selected row's marker and band, no input or hints.
#[test]
fn pane_plain_list() {
    let _g = lock();
    check("pane_plain_list", models(PaneStyle::dark()), 80);
}

/// The filter row at the bottom: the dim placeholder, then typed text.
#[test]
fn pane_filter_row() {
    let _g = lock();
    let mut pane = models(PaneStyle::dark());
    pane.input = Some(input_line("", "filter", false));
    check("pane_filter_row", pane.clone(), 80);
    pane.input = Some(input_line("fast", "filter", true));
    check("pane_filter_row_typed", pane, 80);
}

/// The hint row last: at most four items, then `f1 more`.
#[test]
fn pane_hints() {
    let _g = lock();
    let mut pane = models(PaneStyle::dark());
    pane.input = Some(input_line("", "filter", false));
    pane.hint = capped_hint_line(
        &[
            ("↑↓", "move"),
            ("enter", "open"),
            ("tab", "scope"),
            ("esc", "close"),
            ("ctrl+d", "delete"),
        ],
        false,
    );
    check("pane_hints", pane, 80);
}

/// At 100 columns: list | blank gutter | detail in the stronger shade.
#[test]
fn pane_split_100() {
    let _g = lock();
    let st = PaneStyle::dark();
    let mut pane = models(st);
    pane.detail = models_detail(&st);
    pane.input = Some(input_line("", "filter", false));
    pane.hint = models_hints();
    check("pane_split_100", pane, 100);
}

/// Below 100 columns the detail stacks under the list (or hides).
#[test]
fn pane_narrow() {
    let _g = lock();
    let st = PaneStyle::dark();
    let mut pane = models(st);
    pane.detail = models_detail(&st);
    pane.input = Some(input_line("", "filter", false));
    pane.hint = models_hints();
    check("pane_narrow", pane.clone(), 60);
    pane.narrow_detail = NarrowDetail::Hidden;
    check("pane_narrow_hidden", pane, 60);
}

/// A blocking pane: the yellow chip with dark chip text, rows with no
/// selection.
#[test]
fn pane_warn_chip() {
    let _g = lock();
    let st = PaneStyle::dark();
    let mut pane = Pane::new("Allow Shell?");
    pane.chip = Chip::Warn;
    pane.rows = vec![
        Line::from(vec![
            dim(&st, "$ "),
            text(&st, "printf 'alpha\\nbeta\\ngamma\\n' | sort -r"),
        ]),
        Line::from(vec![dim(&st, "access: pipes 2 programs")]),
        Line::from(vec![dim(&st, "1  "), text(&st, "yes")]),
        Line::from(vec![
            dim(&st, "2  "),
            text(&st, "yes, always for printf this session"),
        ]),
        Line::from(vec![dim(&st, "3  "), text(&st, "no, tell it why")]),
    ];
    pane.selected = Some(2);
    pane.hint = hint_line(&[("1–3", "pick"), ("e", "edit first")]);
    pane.style = Some(st);
    check("pane_warn_chip", pane, 100);
}

/// Monochrome: light borders, the title in the top border, ` › ` + bold,
/// a `│` between list and detail, and no rule above the filter row.
#[test]
fn pane_mono() {
    let _g = lock();
    hoocode_code_tui_theme::chalk::set_enabled(false);
    let st = PaneStyle::mono();
    let mut pane = models(st);
    pane.detail = models_detail(&st);
    pane.input = Some(input_line("", "filter", false));
    pane.hint = models_hints();
    let split = render_golden(&mut Drawn(pane.clone()), 100);
    let narrow = render_golden(&mut Drawn(pane), 60);
    hoocode_code_tui_theme::chalk::set_enabled(true);
    let text = split.split("--- styles ---").next().unwrap_or_default();
    assert!(text.starts_with("┌─ Models ─ /model"), "{text}");
    assert!(!text.contains('├'), "no rule above the filter row:\n{text}");
    let first = text.lines().nth(1).unwrap_or_default();
    assert_eq!(
        first.matches('│').count(),
        3,
        "a │ between list and detail:\n{text}"
    );
    assert_golden!("pane_mono_split_100", split);
    assert_golden!("pane_mono_narrow", narrow);
}

/// Every colour cell carries a background and a foreground.
#[test]
fn pane_paints_fg_with_every_bg() {
    let _g = lock();
    let st = PaneStyle::dark();
    let mut pane = models(st);
    pane.detail = models_detail(&st);
    pane.input = Some(input_line("", "filter", false));
    pane.hint = models_hints();
    let rendered = render_golden(&mut Drawn(pane), 100);
    let styles = rendered.split("--- styles ---").nth(1).unwrap_or_default();
    for run in styles.lines().filter(|l| !l.is_empty()) {
        assert!(
            run.contains("bg=") && run.contains("fg="),
            "a run without bg or fg: {run}"
        );
    }
}
