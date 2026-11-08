//! A theme that did not load shows as one dim row in the resource listing
//! (`N conflicts, first wins  /reload`); the message is one toggle away.
//! With no warning, no such row.

use crate::support::{lock, plain};
use hoocode_code_resources::diagnostics::ResourceDiagnostic;
use hoocode_code_tui_app::expandable_text::Expandable;
use hoocode_code_tui_app::resource_display::{show_loaded_resources_with_details, ResourceListing};

fn theme_listing() -> ResourceListing {
    ResourceListing {
        quiet_startup: true,
        theme_diagnostics: vec![ResourceDiagnostic::warning(
            "Theme \"mine\" did not load: Invalid theme \"mine\". Keeping the last good theme.",
            None,
        )],
        ..Default::default()
    }
}

/// The transcript's text for `components`, colour codes stripped.
fn text_of(components: &[hoocode_tui_render::ComponentHandle]) -> String {
    let lines: Vec<String> = components
        .iter()
        .flat_map(|c| c.borrow_mut().render(100))
        .collect();
    plain(&lines.join("\n"))
}

#[test]
fn a_theme_warning_is_one_row_with_details_behind_the_toggle() {
    let _g = lock(Some("dark"));
    // Quiet startup: the row, no details block down here (`/reload`
    // re-renders the listing with the details behind it).
    let quiet = show_loaded_resources_with_details(&theme_listing(), false, true);
    let text = text_of(&quiet.components);
    assert!(text.contains("1 conflict, first wins"), "{text}");
    assert!(text.contains("/reload"), "{text}");
    assert!(!text.contains("[Theme conflicts]"), "{text}");

    // With the listing shown: the row plus a toggle holding the message.
    let listing = ResourceListing {
        quiet_startup: false,
        ..theme_listing()
    };
    let loaded = show_loaded_resources_with_details(&listing, false, true);
    let folded = text_of(&loaded.components);
    assert!(folded.contains("1 conflict, first wins"), "{folded}");
    assert!(!folded.contains("did not load"), "{folded}");
    let details = loaded.details.expect("conflicts keep a details toggle");
    details.borrow_mut().set_expanded(true);
    let open = text_of(&loaded.components);
    assert!(open.contains("Theme \"mine\" did not load"), "{open}");
    assert!(open.contains("Keeping the last good theme"), "{open}");
}

#[test]
fn no_theme_warning_shows_no_conflict_row() {
    let _g = lock(Some("dark"));
    let listing = ResourceListing {
        quiet_startup: true,
        ..Default::default()
    };
    let loaded = show_loaded_resources_with_details(&listing, false, true);
    assert!(loaded.components.is_empty());
}
