//! A theme that did not load shows under `[Theme conflicts]` in the resource
//! listing, with what the screen kept; with no warning, no such section.

use crate::support::{lock, plain};
use hoocode_code_resources::diagnostics::ResourceDiagnostic;
use hoocode_code_tui_app::resource_display::{show_loaded_resources, ResourceListing};

fn rendered(listing: &ResourceListing) -> String {
    let lines: Vec<String> = show_loaded_resources(listing, false, true)
        .into_iter()
        .flat_map(|h| h.borrow_mut().render(100))
        .collect();
    plain(&lines.join("\n"))
}

#[test]
fn a_theme_warning_shows_under_theme_conflicts() {
    let _g = lock(Some("dark"));
    let listing = ResourceListing {
        quiet_startup: true,
        theme_diagnostics: vec![ResourceDiagnostic::warning(
            "Theme \"mine\" did not load: Invalid theme \"mine\". Keeping the last good theme.",
            None,
        )],
        ..Default::default()
    };
    let text = rendered(&listing);
    assert!(text.contains("[Theme conflicts]"), "{text}");
    assert!(text.contains("Theme \"mine\" did not load"), "{text}");
    assert!(text.contains("Keeping the last good theme"), "{text}");
}

#[test]
fn no_theme_warning_shows_no_theme_section() {
    let _g = lock(Some("dark"));
    let listing = ResourceListing {
        quiet_startup: true,
        ..Default::default()
    };
    assert!(!rendered(&listing).contains("[Theme conflicts]"));
}
