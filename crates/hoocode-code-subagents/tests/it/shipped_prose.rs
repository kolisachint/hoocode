//! The Task delegation prompt is the shipped template file, not literals.

use std::path::{Path, PathBuf};

use hoocode_code_subagents::tools::build_task_main_prompt;

fn template_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/prompts/task-main.md")
}

#[test]
fn is_the_template_file_and_nothing_else() {
    let template = std::fs::read_to_string(template_path()).unwrap();
    assert_eq!(build_task_main_prompt(), template.trim());
}

#[test]
fn leaves_no_substitution_token_in_the_rendered_prompt() {
    assert!(!build_task_main_prompt().contains("{{"));
}

#[test]
fn carries_the_parent_guidance_the_brief_and_the_worktree_rules() {
    let prompt = build_task_main_prompt();
    assert!(prompt.contains("Results arrive as messages at the next turn boundary"));
    assert!(prompt.contains("Call AgentOutput with wait:true only when"));
    assert!(prompt.contains("cancel: true"));
    assert!(prompt.contains("Write every `prompt` as a brief"));
    assert!(prompt.contains("~/.hoocode/worktrees/<repo>/<label>"));
    assert!(prompt.contains("agent/<label>"));
    assert!(prompt.contains("never pushes"));
}
