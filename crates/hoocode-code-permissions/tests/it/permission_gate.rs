//! Port of hoocode `test/permission-gate-mutation-path.test.ts` (v0.5.89), plus
//! the hard-enforcement rules of `permission-gate.ts`.
//!
//! The project config is `.hoocode/hoo-config.json`; patterns keep hoocode's
//! `.hoocode/...` text since they are plain strings here.

use hoocode_code_permissions::{describe_tool, evaluate, ApprovalChannel, Verdict};

const UI: ApprovalChannel = ApprovalChannel::Ui;
const PRINT: ApprovalChannel = ApprovalChannel::Headless { fail_closed: false };
const RPC: ApprovalChannel = ApprovalChannel::Headless { fail_closed: true };
use serde_json::{json, Value};
use std::path::Path;

/// `ctx.ui.select` that records the prompt and answers "No (block)".
fn run(config: Value, cwd: &Path, tool: &str, input: Value) -> (Verdict, Option<String>) {
    let config = config.as_object().unwrap().clone();
    let verdict = evaluate(&config, cwd, tool, &input, UI);
    let prompt =
        (verdict == Verdict::Prompt).then(|| format!("Allow: {}", describe_tool(tool, &input)));
    (verdict, prompt)
}

fn cwd() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn names_the_file_in_the_approval_prompt_instead_of_unknown() {
    let d = cwd();
    let config = json!({"active_mode": "build", "modes": {"build": {}}});
    let prompts: Vec<String> = ["Edit", "Write"]
        .iter()
        .map(|t| {
            run(config.clone(), d.path(), t, json!({"path": "src/app.ts"}))
                .1
                .unwrap()
        })
        .collect();
    assert_eq!(
        prompts,
        ["Allow: Edit src/app.ts", "Allow: Write src/app.ts"]
    );
}

#[test]
fn still_names_the_file_when_a_model_sends_file_path() {
    let d = cwd();
    let (_, prompt) = run(
        json!({"active_mode": "build", "modes": {"build": {}}}),
        d.path(),
        "Edit",
        json!({"file_path": "src/app.ts"}),
    );
    assert_eq!(prompt.as_deref(), Some("Allow: Edit src/app.ts"));
}

fn docs() -> Value {
    json!({"active_mode": "docs", "modes": {"docs": {"allowed_write_paths": ["docs/*"], "auto_allow": ["Edit", "Write"]}}})
}

fn plan() -> Value {
    json!({"active_mode": "plan", "modes": {"plan": {"allowed_write_paths": [".hoocode/plans/*"], "auto_allow": ["Write"]}}})
}

#[test]
fn lets_allowed_write_paths_permit_a_matching_write() {
    let d = cwd();
    assert_eq!(
        run(docs(), d.path(), "Edit", json!({"path": "docs/guide.md"})).0,
        Verdict::Allow
    );
}

#[test]
fn still_blocks_a_write_outside_allowed_write_paths_and_says_which_file() {
    let d = cwd();
    match run(docs(), d.path(), "Edit", json!({"path": "src/app.ts"})).0 {
        Verdict::Block(reason) => assert!(reason.contains("src/app.ts")),
        v => panic!("{v:?}"),
    }
}

#[test]
fn accepts_a_windows_style_relative_path_against_a_forward_slash_pattern() {
    let d = cwd();
    assert_eq!(
        run(
            plan(),
            d.path(),
            "Write",
            json!({"path": ".hoocode\\plans\\s1.md"})
        )
        .0,
        Verdict::Allow
    );
}

#[test]
fn accepts_an_absolute_path_inside_an_allowed_relative_pattern() {
    let d = cwd();
    let path = d.path().join(".hoocode").join("plans").join("s1.md");
    assert_eq!(
        run(
            plan(),
            d.path(),
            "Write",
            json!({"path": path.to_string_lossy()})
        )
        .0,
        Verdict::Allow
    );
}

#[test]
fn still_blocks_an_absolute_path_that_escapes_the_project_root() {
    let d = cwd();
    assert!(matches!(
        run(plan(), d.path(), "Write", json!({"path": "/etc/passwd"})).0,
        Verdict::Block(_)
    ));
}

#[test]
fn does_not_let_a_patterns_dots_match_arbitrary_characters() {
    let d = cwd();
    assert!(matches!(
        run(
            plan(),
            d.path(),
            "Write",
            json!({"path": "Xhoocode/plans/evil.md"})
        )
        .0,
        Verdict::Block(_)
    ));
}

#[test]
fn blocks_a_mutation_whose_path_cannot_be_identified() {
    let d = cwd();
    assert!(matches!(
        run(docs(), d.path(), "Edit", json!({})).0,
        Verdict::Block(_)
    ));
}

// Hard enforcement (always, with or without a UI).

#[test]
fn hard_rules_apply_without_a_ui_and_everything_else_runs() {
    let d = cwd();
    let config = json!({"modes": {"build": {
        "denied_tools": ["Write"],
        "denied_bash_commands": ["\\brm\\b"],
        "allowed_bash_commands": ["^git\\s", "^ls"]
    }}});
    let c = config.as_object().unwrap().clone();
    let headless = |tool: &str, input: Value| evaluate(&c, d.path(), tool, &input, PRINT);
    assert_eq!(
        headless("Write", json!({"path": "a"})),
        Verdict::Block("Tool \"Write\" is denied in mode \"build\".".into())
    );
    assert_eq!(
        headless("Shell", json!({"command": "rm -rf x"})),
        Verdict::Block("Bash command matches a denied pattern in mode \"build\": \\brm\\b".into())
    );
    assert_eq!(
        headless("Shell", json!({"command": "cat x"})),
        Verdict::Block(
            "Bash command is not permitted in mode \"build\". Allowed patterns: ^git\\s, ^ls"
                .into()
        )
    );
    assert_eq!(
        headless("Shell", json!({"command": "git status"})),
        Verdict::Allow
    );
    // No UI in print/json: gated tools run without a prompt.
    assert_eq!(headless("Edit", json!({"path": "a"})), Verdict::Allow);
    assert_eq!(
        headless("WebFetch", json!({"url": "https://x"})),
        Verdict::Allow
    );

    let allow = json!({"modes": {"plan": {"enabled_tools": ["Read"]}}, "active_mode": "plan"});
    let c = allow.as_object().unwrap().clone();
    assert_eq!(
        evaluate(&c, d.path(), "Shell", &json!({"command": "ls"}), PRINT),
        Verdict::Block("Tool \"Shell\" is not enabled in mode \"plan\" (enabled: Read).".into())
    );
}

#[test]
fn ungated_tools_are_never_prompted_and_bash_is_described_collapsed() {
    let d = cwd();
    assert_eq!(
        run(json!({}), d.path(), "Read", json!({"path": "a"})).0,
        Verdict::Allow
    );
    let (_, prompt) = run(
        json!({}),
        d.path(),
        "Shell",
        json!({"command": "  ls\n  -la  "}),
    );
    assert_eq!(prompt.as_deref(), Some("Allow: $  ls -la "));
    assert_eq!(
        describe_tool("WebSearch", &json!({"query": "rust"})),
        "WebSearch \"rust\""
    );
    assert_eq!(describe_tool("WebFetch", &json!({})), "WebFetch (unknown)");
}

// Approval channels (reliability 1.1): what a gated call that needs approval
// gets when the UI is there, when it is print/json, and when it is rpc.

fn gated_call(channel: ApprovalChannel, config: Value) -> Verdict {
    let d = cwd();
    let c = config.as_object().unwrap().clone();
    evaluate(&c, d.path(), "Shell", &json!({"command": "ls"}), channel)
}

#[test]
fn rpc_denies_a_gated_call_that_needs_approval_with_a_message() {
    match gated_call(RPC, json!({"active_mode": "build", "modes": {"build": {}}})) {
        Verdict::Block(reason) => {
            assert!(
                reason.starts_with("Tool \"Shell\" needs approval"),
                "{reason}"
            );
            assert!(reason.contains("rpc mode denies it"), "{reason}");
            assert!(reason.contains("auto_allow for mode \"build\""), "{reason}");
        }
        v => panic!("{v:?}"),
    }
}

#[test]
fn rpc_still_runs_an_auto_allowed_or_ungated_call() {
    let auto = json!({"active_mode": "build", "modes": {"build": {"auto_allow": ["Shell"]}}});
    assert_eq!(gated_call(RPC, auto), Verdict::Allow);
    let d = cwd();
    let c = json!({}).as_object().unwrap().clone();
    assert_eq!(
        evaluate(&c, d.path(), "Read", &json!({"path": "a"}), RPC),
        Verdict::Allow
    );
}

#[test]
fn print_allows_a_gated_call_without_asking() {
    assert_eq!(
        gated_call(
            PRINT,
            json!({"active_mode": "build", "modes": {"build": {}}})
        ),
        Verdict::Allow
    );
}

#[test]
fn a_ui_still_prompts_for_a_gated_call() {
    assert_eq!(
        gated_call(UI, json!({"active_mode": "build", "modes": {"build": {}}})),
        Verdict::Prompt
    );
}

#[test]
fn rpc_still_enforces_allowed_write_paths_on_writes() {
    let d = cwd();
    let c = docs().as_object().unwrap().clone();
    match evaluate(&c, d.path(), "Write", &json!({"path": "src/app.ts"}), RPC) {
        Verdict::Block(reason) => assert!(reason.contains("src/app.ts"), "{reason}"),
        v => panic!("{v:?}"),
    }
}
