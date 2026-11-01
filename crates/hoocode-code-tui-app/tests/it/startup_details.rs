//! The startup header and the resource listing's details open at the full
//! stop and fold at peek and radar. `verbose` keeps them open. The dial flips
//! them in place (`Expandable::set_expanded`), so a live move and a fresh
//! listing (`/reload`, a session switch) show the same thing.

use std::cell::Cell;
use std::rc::Rc;

use hoocode_code_settings::ToolOutputView;
use hoocode_code_tui_app::expandable_text::Expandable;
use hoocode_code_tui_app::mode::startup_header;
use hoocode_code_tui_app::resource_display::{
    show_loaded_resources_with_details, startup_details_open, ListedItem, ResourceListing,
};
use hoocode_tui_render::{Component, ComponentHandle};

use crate::support::{lock, plain};

const ONBOARDING: &str = "look up its docs";

fn item(name: &str, path: &str) -> ListedItem {
    ListedItem {
        name: name.into(),
        path: path.into(),
        source_info: None,
    }
}

fn listing(expanded: bool, verbose: bool) -> ResourceListing {
    ResourceListing {
        cwd: "/w".into(),
        columns: Some(100),
        skills: vec![item("alpha", "/w/.agents/skills/alpha/SKILL.md")],
        templates: vec![item("fix", "/w/.hoo/prompts/fix.md")],
        agents: vec![("explore".into(), "Explores the codebase quickly".into())],
        expanded,
        verbose,
        ..Default::default()
    }
}

/// The transcript's text for `components`, colour codes stripped.
fn text_of(components: &[ComponentHandle]) -> String {
    components
        .iter()
        .flat_map(|c| c.borrow_mut().render(100))
        .map(|line| plain(&line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_dial_policy_opens_at_full_and_under_verbose_only() {
    for view in [
        ToolOutputView::Radar,
        ToolOutputView::Peek,
        ToolOutputView::Full,
    ] {
        assert_eq!(
            startup_details_open(false, view),
            view == ToolOutputView::Full,
            "{view:?}"
        );
        assert!(
            startup_details_open(true, view),
            "verbose holds {view:?} open"
        );
    }
}

#[test]
fn the_listing_details_fold_at_peek_and_open_at_full_on_a_live_move() {
    let _g = lock(Some("dark"));
    let loaded = show_loaded_resources_with_details(&listing(false, false), false, true);
    let details = loaded.details.expect("a listing with skills has details");
    let folded = text_of(&loaded.components);
    assert!(!folded.contains("[Skills]"), "{folded}");
    assert!(!folded.contains("/skill:alpha"), "{folded}");
    // The summary line stays: the counts are what the peek shows.
    assert!(folded.contains("skill"), "{folded}");

    // Ctrl+O to full: the details open in place.
    details
        .borrow_mut()
        .set_expanded(startup_details_open(false, ToolOutputView::Full));
    let open = text_of(&loaded.components);
    for needle in [
        "[Skills]",
        "/skill:alpha",
        "[Commands]",
        "/fix",
        "[Agents]",
        "explore",
    ] {
        assert!(open.contains(needle), "{needle} missing at full:\n{open}");
    }

    // Back to peek: they fold again.
    details
        .borrow_mut()
        .set_expanded(startup_details_open(false, ToolOutputView::Peek));
    assert!(!text_of(&loaded.components).contains("[Skills]"));
}

#[test]
fn the_listing_opens_at_startup_when_the_saved_view_is_full() {
    let _g = lock(Some("dark"));
    let loaded = show_loaded_resources_with_details(&listing(true, false), false, true);
    assert!(text_of(&loaded.components).contains("[Skills]"));
}

#[test]
fn verbose_keeps_the_listing_details_open_when_the_dial_moves_to_peek() {
    let _g = lock(Some("dark"));
    let loaded = show_loaded_resources_with_details(&listing(true, true), false, true);
    let details = loaded.details.expect("details");
    details
        .borrow_mut()
        .set_expanded(startup_details_open(true, ToolOutputView::Peek));
    assert!(text_of(&loaded.components).contains("[Skills]"));
}

#[test]
fn a_listing_without_resources_has_no_details_to_open() {
    let _g = lock(Some("dark"));
    let loaded = show_loaded_resources_with_details(&ResourceListing::default(), false, true);
    assert!(loaded.details.is_none());
}

#[test]
fn the_listing_rows_name_each_skill_as_it_is_typed() {
    let _g = lock(Some("dark"));
    let loaded = show_loaded_resources_with_details(&listing(true, false), false, true);
    let text = text_of(&loaded.components);
    // Skills are typed as `/skill:<name>`; commands as `/<name>`; agents by
    // their plain name. Each row names the item, then where it lives.
    assert!(text.contains("/skill:alpha"), "{text}");
    assert!(text.contains("/fix"), "{text}");
    assert!(text.contains("explore"), "{text}");
}

#[test]
fn the_startup_header_folds_at_peek_and_opens_at_full() {
    let _g = lock(Some("dark"));
    let mut header = startup_header(
        Rc::new(Cell::new((100, 24))),
        "0.2.22",
        "~/w",
        Rc::new(Cell::new(false)),
        startup_details_open(false, ToolOutputView::Peek),
    );
    let folded = plain(&header.render(100).join("\n"));
    assert!(!folded.contains(ONBOARDING), "{folded}");
    assert!(!folded.contains("to queue follow-up"), "{folded}");

    header.set_expanded(startup_details_open(false, ToolOutputView::Full));
    let open = plain(&header.render(100).join("\n"));
    assert!(open.contains(ONBOARDING), "{open}");
    assert!(open.contains("to queue follow-up"), "{open}");

    header.set_expanded(startup_details_open(false, ToolOutputView::Peek));
    assert!(!plain(&header.render(100).join("\n")).contains(ONBOARDING));
}

#[test]
fn the_startup_header_opens_at_startup_when_the_saved_view_is_full() {
    let _g = lock(Some("dark"));
    let mut header = startup_header(
        Rc::new(Cell::new((100, 24))),
        "0.2.22",
        "~/w",
        Rc::new(Cell::new(false)),
        startup_details_open(false, ToolOutputView::Full),
    );
    assert!(plain(&header.render(100).join("\n")).contains(ONBOARDING));
}

#[test]
fn the_logo_keeps_three_rows_at_the_fullscreen_body_width() {
    // Fullscreen lays the body out one column narrower than the terminal
    // (`body_width`), so an edge star dot built for the full width wraps
    // onto its own row and tears the owl apart. A 51-wide terminal keeps the
    // dot at column 50; the body renders at 50.
    let _g = lock(Some("dark"));
    let mut header = startup_header(
        Rc::new(Cell::new((51, 24))),
        "0.2.22",
        "~/w",
        Rc::new(Cell::new(false)),
        startup_details_open(false, ToolOutputView::Peek),
    );
    let rows = header.render(50);
    let text = plain(&rows.join("\n"));
    assert_eq!(rows.len(), 3, "{text}");
}

#[test]
fn the_logo_re_lays_itself_out_after_a_resize() {
    // The header reads the live terminal size on each rebuild: narrowing the
    // terminal (a vertical split) and refreshing must drop the dots that no
    // longer fit instead of wrapping them onto their own rows.
    let _g = lock(Some("dark"));
    let size = Rc::new(Cell::new((100u16, 24u16)));
    let mut header = startup_header(
        size.clone(),
        "0.2.22",
        "~/w",
        Rc::new(Cell::new(false)),
        startup_details_open(false, ToolOutputView::Peek),
    );
    assert_eq!(header.render(99).len(), 3);
    size.set((60, 24));
    header.refresh();
    let rows = header.render(59);
    let text = plain(&rows.join("\n"));
    assert_eq!(rows.len(), 3, "{text}");
    assert!(text.contains('\u{2726}'), "{text}");
    assert!(!text.contains('\u{22c6}'), "far dot no longer fits: {text}");
}

#[test]
fn the_logo_skips_a_dot_that_would_land_past_the_edge() {
    // Row 2 carries the cwd: with a long one the row already reaches past
    // the dot's column, so appending the dot would push the row past the
    // body width and wrap it onto its own row. 70 chars of cwd make row 2
    // exactly 79 wide; the body renders at 79 on an 80-wide terminal.
    let _g = lock(Some("dark"));
    let cwd = format!("~/{}", "x".repeat(68));
    assert_eq!(cwd.len(), 70);
    let mut header = startup_header(
        Rc::new(Cell::new((80, 24))),
        "0.2.22",
        &cwd,
        Rc::new(Cell::new(false)),
        startup_details_open(false, ToolOutputView::Peek),
    );
    let rows = header.render(79);
    let text = plain(&rows.join("\n"));
    assert_eq!(rows.len(), 3, "{text}");
}
