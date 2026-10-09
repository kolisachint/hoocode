//! Port of the `showLoadedResources` cases of the pin's
//! `test/interactive-mode-status.test.ts`, against `show_loaded_resources`.
//! The file's `showRecord` cases are in `record_row.rs`'s tests, its
//! `jumpToFullView` / `setToolsExpanded` cases drive the real interactive
//! mode (`hoocode-code-cli` runtime tests). Its canvas case (real canvas
//! discovery) and the extension UI context cases (`setTheme`,
//! `addAutocompleteProvider`, autocomplete wrappers) belong to phase 12.

use crate::support::lock;
use hoocode_code_resources::context_files::{ContextFile, ContextFileSize};
use hoocode_code_resources::diagnostics::{DiagnosticType, ResourceDiagnostic};
use hoocode_code_tui_app::resource_display::*;

/// Every child rendered at 220, SGR stripped, trailing spaces gone, trimmed.
fn normalized(listing: &ResourceListing, force: bool, diagnostics_when_quiet: bool) -> String {
    show_loaded_resources(listing, force, diagnostics_when_quiet)
        .into_iter()
        .flat_map(|h| h.borrow_mut().render(220))
        .map(|l| {
            hoocode_tui_util::strip_vt_control_characters(&l)
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn render(listing: &ResourceListing) -> String {
    normalized(listing, false, false)
}

fn base(cwd: &str) -> ResourceListing {
    ResourceListing {
        cwd: cwd.into(),
        columns: Some(100),
        ..Default::default()
    }
}

fn skill(name: &str, path: &str) -> ListedItem {
    ListedItem {
        name: name.into(),
        path: path.into(),
        source_info: None,
    }
}

fn context(path: &str) -> ContextFile {
    ContextFile {
        path: path.into(),
        content: String::new(),
        tokens: None,
        size: None,
    }
}

#[test]
fn shows_the_counted_summary_no_names_by_default() {
    let _g = lock(Some("dark"));
    let out = render(&ResourceListing {
        skills: vec![skill("commit", "/tmp/skill/SKILL.md")],
        ..base("/tmp/project")
    });
    assert!(out.contains("1 skill"), "{out}");
    assert!(!out.contains("commit"));
    assert!(!out.contains("details"));
}

#[test]
fn shows_the_detailed_listing_when_expanded() {
    let _g = lock(Some("dark"));
    let out = render(&ResourceListing {
        expanded: true,
        skills: vec![skill("commit", "/tmp/skill/SKILL.md")],
        ..base("/tmp/project")
    });
    assert!(out.contains("[Skills]"));
    assert!(out.contains("commit"));
}

#[test]
fn shows_the_detailed_listing_on_verbose_startup_even_when_collapsed() {
    let _g = lock(Some("dark"));
    let out = render(&ResourceListing {
        quiet_startup: true,
        verbose: true,
        skills: vec![skill("commit", "/tmp/skill/SKILL.md")],
        ..base("/tmp/project")
    });
    assert!(out.contains("[Skills]"), "{out}");
    assert!(out.contains("commit"));
}

#[test]
fn shows_context_paths_relative_to_cwd_while_preserving_full_external_paths() {
    let _g = lock(Some("dark"));
    let home = std::env::var("HOME").unwrap();
    let cwd = format!("{home}/Development/hoocode");
    for expanded in [false, true] {
        let out = render(&ResourceListing {
            expanded,
            context_files: vec![
                context(&format!("{home}/.hoocode/agent/AGENTS.md")),
                context(&format!("{cwd}/AGENTS.md")),
            ],
            ..base(&cwd)
        });
        assert!(out.contains("context"), "{out}");
        assert!(
            out.contains("~/.hoocode/agent/AGENTS.md, AGENTS.md"),
            "{out}"
        );
        assert!(!out.contains(&format!("{cwd}/AGENTS.md")));
    }
}

#[test]
fn does_not_show_the_listing_on_quiet_startup_during_reload() {
    let _g = lock(Some("dark"));
    let listing = ResourceListing {
        quiet_startup: true,
        skills: vec![skill("commit", "/tmp/skill/SKILL.md")],
        ..base("/tmp/project")
    };
    assert!(show_loaded_resources(&listing, false, true).is_empty());
}

#[test]
fn still_shows_diagnostics_on_quiet_startup_when_requested() {
    let _g = lock(Some("dark"));
    let out = normalized(
        &ResourceListing {
            quiet_startup: true,
            skills: vec![skill("commit", "/tmp/skill/SKILL.md")],
            skill_diagnostics: vec![ResourceDiagnostic {
                kind: DiagnosticType::Warning,
                message: "duplicate skill name".into(),
                path: None,
                collision: None,
            }],
            ..base("/tmp/project")
        },
        false,
        true,
    );
    assert!(out.contains("[Skill conflicts]"), "{out}");
    assert!(!out.contains("[Skills]"));
}

#[test]
fn annotates_an_oversized_context_file_on_the_context_row() {
    let _g = lock(Some("dark"));
    let out = render(&ResourceListing {
        context_files: vec![ContextFile {
            path: "/tmp/AGENTS.md".into(),
            content: "large".into(),
            tokens: Some(3624),
            size: Some(ContextFileSize::Large),
        }],
        ..base("/tmp/project")
    });
    assert!(out.contains("~3.6k tokens"), "{out}");
    assert!(out.contains("consider trimming"));
}

#[test]
fn shows_context_file_read_failures_as_warnings() {
    let _g = lock(Some("dark"));
    let out = render(&ResourceListing {
        context_warnings: vec!["Could not read /tmp/AGENTS.md: EACCES".into()],
        ..base("/tmp/project")
    });
    assert!(out.contains("Could not read"), "{out}");
}

#[test]
fn summary_shows_counts_and_context_names_sections_stay_collapsed() {
    let _g = lock(Some("dark"));
    let out = render(&ResourceListing {
        context_files: vec![context("/tmp/AGENTS.md")],
        skills: vec![skill("commit", "/tmp/skill/SKILL.md")],
        ..base("/tmp/project")
    });
    assert!(out.contains("1 skill"), "{out}");
    assert!(out.contains("AGENTS.md"));
    assert!(!out.contains("[Context]"));
    assert!(!out.contains("[Skills]"));
}

#[test]
fn expanded_details_carry_every_section_regardless_of_resource_count() {
    let _g = lock(Some("dark"));
    let out = render(&ResourceListing {
        expanded: true,
        skills: ["alpha", "beta", "gamma", "delta", "epsilon"]
            .iter()
            .map(|n| skill(n, &format!("/tmp/skill/{n}/SKILL.md")))
            .collect(),
        context_files: vec![context("/tmp/AGENTS.md")],
        ..base("/tmp/project")
    });
    assert!(out.contains("5 skills"), "{out}");
    assert!(out.contains("[Skills]"));
}
