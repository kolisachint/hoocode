//! The Task half of the pin's `coding-agent/test/shipped-prose.test.ts`
//! ("the Task delegation prompt has one source"), against the template files,
//! plus a byte check of the crate's copies against the pin when it is present.

use std::path::{Path, PathBuf};

use cortexcode_code_subagents::tools::build_task_main_prompt;

fn templates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/prompts")
}

fn template(name: &str) -> String {
    std::fs::read_to_string(templates().join(name)).unwrap()
}

/// A cwd with no project agents: the built-in explore/plan agents are
/// background agents, so this takes the with-background branch.
fn cwd() -> PathBuf {
    std::env::temp_dir().join(format!("shipped-prose-{}", std::process::id()))
}

#[test]
fn is_assembled_from_templates_prompts_and_nothing_else() {
    let background = template("task-background-agents.md");
    let expected = template("task-main.md")
        .replacen("{{BACKGROUND_GUIDANCE}}", background.trim(), 1)
        .trim()
        .to_string();
    assert_eq!(build_task_main_prompt(&cwd()), expected);
}

#[test]
fn keeps_the_substitution_token_the_background_variants_fill() {
    assert!(template("task-main.md").contains("{{BACKGROUND_GUIDANCE}}"));
    assert!(!template("task-background-agents.md").is_empty());
    assert!(!template("task-background-none.md").is_empty());
}

#[test]
fn leaves_no_substitution_token_in_the_rendered_prompt() {
    assert!(!build_task_main_prompt(&cwd()).contains("{{"));
}

/// Deliberate divergences from the pinned hoocode templates, as
/// `(file, pinned text, our text)`.
///
/// The subagent tools are ours to name: `Task` collided with TodoWrite items in
/// the task store and read as a to-do rather than a background run, so the
/// wire names are `Dispatch` and `DispatchStatus` (`Task` and `TaskOutput` stay
/// registered as deprecated aliases for a release). A prompt that told the
/// model to "call Task" while the tool was called `Dispatch` would be worse
/// than the divergence, so the templates move with the names.
///
/// Everything else in these files must still match the pin byte for byte: a
/// divergence nobody wrote down here is a bug, and this list is where it has to
/// be declared.
const DECLARED_DIVERGENCES: &[(&str, &str, &str)] = &[
    ("task-main.md", "**Task** tool", "**Dispatch** tool"),
    ("task-main.md", "call Task with", "call Dispatch with"),
    (
        "task-background-agents.md",
        "`TaskOutput",
        "`DispatchStatus",
    ),
    ("task-background-none.md", "`TaskOutput", "`DispatchStatus"),
];

#[test]
fn template_copies_match_the_pin_except_where_we_say_so() {
    let pin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/hoocode-pin/packages/coding-agent/templates/prompts");
    if !pin.is_dir() {
        eprintln!("pinned hoocode not built: skipping the copy check");
        return;
    }
    for name in [
        "task-main.md",
        "task-background-agents.md",
        "task-background-none.md",
    ] {
        let theirs = std::fs::read_to_string(pin.join(name)).unwrap();
        // Normalise ours back to the pinned wording: after that, the two files
        // must be byte-identical, so a divergence nobody declared fails here.
        let mut ours = template(name);
        for (_file, pinned_text, our_text) in
            DECLARED_DIVERGENCES.iter().filter(|(f, _, _)| *f == name)
        {
            assert!(
                theirs.contains(pinned_text),
                "{name} no longer contains the pinned text {pinned_text:?}; the divergence entry is stale"
            );
            assert!(
                ours.contains(our_text),
                "{name} no longer contains our text {our_text:?}; the divergence entry is stale"
            );
            ours = ours.replace(our_text, pinned_text);
        }
        assert_eq!(
            ours, theirs,
            "{name} differs from the pin beyond its declared divergences"
        );
    }
}
