//! The mode and grill prompts are built from the template files, not literals.

use std::path::{Path, PathBuf};

use hoocode_code_modes::plan::{build_grill_message, GrillTarget, PlanSections};
use hoocode_code_modes::prompts::{default_mode_prompt, grill_prompt};
use hoocode_code_modes::{build_mode_system_prompt, DEFAULT_MODE_PROMPTS};

fn templates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("templates")
}

fn template(parts: &[&str]) -> String {
    let path = parts.iter().fold(templates(), |p, part| p.join(part));
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn serves_the_built_in_mode_defaults_from_templates() {
    for mode in ["ask", "build", "debug", "plan"] {
        assert_eq!(
            default_mode_prompt(mode),
            Some(template(&["modes", mode, "system.md"]).as_str()),
            "{mode}"
        );
    }
}

#[test]
fn is_what_the_mode_resolver_falls_back_to() {
    assert_eq!(
        build_mode_system_prompt("build", Path::new("/nonexistent"), &[]).as_deref(),
        default_mode_prompt("build")
    );
}

#[test]
fn keeps_the_plan_mode_substitution_token() {
    assert!(default_mode_prompt("plan")
        .unwrap()
        .contains("{{PLAN_PATH}}"));
}

#[test]
fn embeds_the_grill_templates_verbatim() {
    for name in ["grill-me", "grill-plan", "grill-bridge"] {
        assert_eq!(
            grill_prompt(name),
            template(&["prompts", &format!("{name}.md")])
        );
    }
}

#[test]
fn builds_each_grill_target_from_those_files_and_nothing_else() {
    let sections = PlanSections {
        goal: Some("Ship it.".into()),
        raw: "Ship it.".into(),
        ..Default::default()
    };
    let me = template(&["prompts", "grill-me.md"]);
    let critique = template(&["prompts", "grill-plan.md"]);
    let bridge = template(&["prompts", "grill-bridge.md"]);
    let (me, critique, bridge) = (me.trim(), critique.trim(), bridge.trim());
    let goal = "\n\n---\n\n**Goal**\nShip it.";
    assert_eq!(
        build_grill_message(&sections, GrillTarget::Me),
        format!("{me}{goal}")
    );
    assert_eq!(
        build_grill_message(&sections, GrillTarget::Plan),
        format!("{critique}{goal}")
    );
    assert_eq!(
        build_grill_message(&sections, GrillTarget::Both),
        format!("{me}\n\n{bridge}\n\n{critique}{goal}")
    );
}

#[test]
fn carries_every_mode_template() {
    let mut modes: Vec<&str> = DEFAULT_MODE_PROMPTS.iter().map(|(m, _)| *m).collect();
    modes.sort();
    assert_eq!(modes, ["ask", "build", "debug", "plan"]);
}
