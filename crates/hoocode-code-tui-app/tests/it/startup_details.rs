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
        100,
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
        100,
        "0.2.22",
        "~/w",
        Rc::new(Cell::new(false)),
        startup_details_open(false, ToolOutputView::Full),
    );
    assert!(plain(&header.render(100).join("\n")).contains(ONBOARDING));
}
