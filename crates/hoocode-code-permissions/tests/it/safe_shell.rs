//! The built-in read-only Shell allowlist, the shipped mode rules and
//! `skip_permissions`.

use hoocode_code_modes::config::{default_config, with_builtin_defaults, HooConfig};
use hoocode_code_permissions::{evaluate, is_safe_shell_command, ApprovalChannel, Verdict};
use serde_json::{json, Value};

const UI: ApprovalChannel = ApprovalChannel::Ui;

fn obj(v: Value) -> HooConfig {
    v.as_object().unwrap().clone()
}

/// The shipped rules for `mode`, as a user with no file of their own gets them.
fn shipped(mode: &str) -> HooConfig {
    let mut config = with_builtin_defaults(&HooConfig::new());
    config.insert("active_mode".into(), mode.into());
    config
}

fn verdict(config: &HooConfig, tool: &str, input: Value) -> Verdict {
    let dir = tempfile::tempdir().unwrap();
    evaluate(config, dir.path(), tool, &input, UI)
}

fn shell(config: &HooConfig, command: &str) -> Verdict {
    verdict(config, "Shell", json!({ "command": command }))
}

#[test]
fn read_only_commands_are_safe() {
    for command in [
        "ls -la",
        "pwd",
        "cat src/main.rs",
        "head -n 20 a.txt | wc -l",
        "rg -n foo src",
        "grep -r bar . 2>/dev/null",
        "git status",
        "git diff HEAD~1 -- src",
        "git log --oneline -5",
        "git branch --show-current",
        "cd src && ls",
        "sort -r names.txt | head",
        "echo hello",
        "ls 2>&1",
        "ls *.rs",
        "wc -l src/*.rs",
        "rg -g '*.rs' 'fn main$' src",
        "rg foo src/*",
        "git log -p -- 'src/*'",
        "tail -n 5 log.txt",
        "cat ~/notes.txt",
    ] {
        assert!(is_safe_shell_command(command), "{command}");
    }
}

#[test]
fn commands_that_write_or_run_code_are_not_safe() {
    for command in [
        "",
        "rm -rf target",
        "ls; rm x",
        "echo hi > out.txt",
        "echo hi >> out.txt",
        "ls >| out.txt",
        "ls >&out.txt",
        "cat $(which ls)",
        "cat `which ls`",
        "ls\nrm x",
        "sort -o out names.txt",
        "sort -ro out names.txt",
        "sort --compress-program=sh a",
        "rg --pre ./x foo",
        "git push",
        "git -c core.pager=x log",
        "git diff --output=x",
        "git branch -D main",
        "git grep -O foo",
        "./ls",
        "X=1 ls",
        "find . -delete",
        "sed -i s/a/b/ f",
        "cargo build",
        "sudo ls",
        "tree -o out",
        // GNU long-option abbreviations.
        "sort --out=x a",
        "sort --compress=sh a",
        "file --comp -m x",
        "git diff --outp=x",
        "git log --ext",
        // Short-flag clusters in git.
        "git grep -nOvim foo",
        // Programs that run a command.
        "rg --hostname-bin=sh foo",
        "rg --pre=sh foo",
        "rg '--pre'=sh foo",
        "rg --p\\re=sh foo",
        // `tree -R` writes 00Tree.html files.
        "tree -R -H .",
        // Never ends.
        "tail -f log.txt",
        "tail -nF log.txt",
        "tail --follow=name log.txt",
        // Expansions the word checks cannot see.
        "rg $'--pre' sh foo",
        "sort $'-o' x a",
        "rg $PRE foo",
        "echo \"$HOME\"",
        "cat ${HOME}",
        "cat <(ls)",
        "cat x <<EOF",
        // Globs and braces that can expand to a flag (a file named `--pre=x`).
        "rg foo *",
        "rg foo -*",
        "sort ?",
        "git log *",
        "rg x {--pre=sh,a}",
        "tree [-]o",
        // Redirect / list forms.
        "cat <> f",
        "ls &> out",
        "ls & rm x",
        "ls | sh",
        "(rm x)",
        "{ rm x; }",
        "git status; git push",
    ] {
        assert!(!is_safe_shell_command(command), "{command:?}");
    }
}

#[test]
fn safe_shell_runs_without_a_prompt_in_every_mode() {
    for mode in ["ask", "plan", "build", "debug"] {
        assert_eq!(
            shell(&shipped(mode), "git status"),
            Verdict::Allow,
            "{mode}"
        );
        assert_eq!(
            shell(&shipped(mode), "make build"),
            Verdict::Prompt,
            "{mode}"
        );
    }
}

#[test]
fn safe_shell_can_be_turned_off() {
    let mut config = shipped("build");
    config.insert("safe_shell".into(), false.into());
    assert_eq!(shell(&config, "git status"), Verdict::Prompt);
}

#[test]
fn ask_and_debug_deny_writes() {
    for mode in ["ask", "debug"] {
        let config = shipped(mode);
        for tool in ["Write", "Edit"] {
            assert!(
                matches!(
                    verdict(&config, tool, json!({"path": "src/a.rs"})),
                    Verdict::Block(_)
                ),
                "{mode} {tool}"
            );
        }
        assert_eq!(
            verdict(&config, "Read", json!({"path": "a"})),
            Verdict::Allow
        );
    }
}

#[test]
fn plan_writes_only_under_dot_hoocode_without_asking() {
    let config = shipped("plan");
    assert_eq!(
        verdict(&config, "Write", json!({"path": ".hoocode/plans/s.md"})),
        Verdict::Allow
    );
    assert!(matches!(
        verdict(&config, "Write", json!({"path": "src/main.rs"})),
        Verdict::Block(_)
    ));
    assert!(matches!(
        verdict(&config, "Edit", json!({"path": ".hoocode/../src/main.rs"})),
        Verdict::Block(_)
    ));
}

#[test]
fn build_prompts_for_writes() {
    assert_eq!(
        verdict(&shipped("build"), "Edit", json!({"path": "src/a.rs"})),
        Verdict::Prompt
    );
}

#[test]
fn a_user_key_overrides_the_shipped_rule() {
    let config = with_builtin_defaults(&obj(json!({
        "active_mode": "ask",
        "modes": {"ask": {"denied_tools": []}}
    })));
    assert_eq!(
        verdict(&config, "Edit", json!({"path": "src/a.rs"})),
        Verdict::Prompt
    );
}

#[test]
fn skip_permissions_allows_everything_but_keeps_mode_rules() {
    let mut config = shipped("build");
    config.insert("skip_permissions".into(), true.into());
    assert_eq!(shell(&config, "rm -rf target"), Verdict::Allow);
    assert_eq!(
        verdict(&config, "Edit", json!({"path": "src/a.rs"})),
        Verdict::Allow
    );
    let mut ask = shipped("ask");
    ask.insert("skip_permissions".into(), true.into());
    assert!(matches!(
        verdict(&ask, "Write", json!({"path": "src/a.rs"})),
        Verdict::Block(_)
    ));
    // rpc (fail closed) runs it too.
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        evaluate(
            &config,
            dir.path(),
            "Shell",
            &json!({"command": "make"}),
            ApprovalChannel::Headless { fail_closed: true },
        ),
        Verdict::Allow
    );
}

#[test]
fn the_shipped_file_names_every_mode() {
    let config = default_config();
    assert_eq!(config["active_mode"], "build");
    assert_eq!(config["safe_shell"], true);
    assert_eq!(config["skip_permissions"], false);
    for mode in ["ask", "plan", "build", "debug"] {
        assert!(config["modes"][mode].is_object(), "{mode}");
    }
}

#[test]
fn plan_writes_to_project_config_still_ask() {
    let config = shipped("plan");
    for path in [".hoocode/hoo-config.json", ".hoocode/settings.json"] {
        assert_eq!(
            verdict(&config, "Write", json!({ "path": path })),
            Verdict::Prompt,
            "{path}"
        );
    }
    // Even with skip_permissions on.
    let mut skip = shipped("plan");
    skip.insert("skip_permissions".into(), true.into());
    assert_eq!(
        verdict(&skip, "Edit", json!({"path": ".hoocode/hoo-config.json"})),
        Verdict::Prompt
    );
}

#[cfg(unix)]
#[test]
fn plan_writes_through_a_symlink_out_of_dot_hoocode_are_blocked() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".hoocode")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("src"), dir.path().join(".hoocode/link")).unwrap();
    let config = shipped("plan");
    assert!(matches!(
        evaluate(
            &config,
            dir.path(),
            "Write",
            &json!({"path": ".hoocode/link/main.rs"}),
            UI
        ),
        Verdict::Block(_)
    ));
    assert_eq!(
        evaluate(
            &config,
            dir.path(),
            "Write",
            &json!({"path": ".hoocode/plans/a.md"}),
            UI
        ),
        Verdict::Allow
    );
}
