//! `a always` on the clearance card is session-only (M6 reviews): the gate
//! remembers the Shell program in memory for (cwd, mode) — never in config,
//! so a later `rm` still prompts and a later `printf` does not. The Edit
//! path still persists to the global config, which is redirected to a
//! scratch dir here and restored afterwards.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

use hoocode_code_permissions::{
    always_allows, evaluate, is_always_wrapper, shell_program, ApprovalChannel, ClearanceChoice,
    ClearanceInfo, HooPermissionGate, PermissionUi, Verdict,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Redirects the agent dir at `HOOCODE_CODING_AGENT_DIR`, restoring it on
/// drop. Only the non-Shell (`auto_allow`) path writes config; the Shell
/// path must never touch it.
struct AgentHome {
    _guard: MutexGuard<'static, ()>,
    saved: Option<std::ffi::OsString>,
    dir: tempfile::TempDir,
}

impl AgentHome {
    fn new() -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let saved = std::env::var_os("HOOCODE_CODING_AGENT_DIR");
        std::env::set_var("HOOCODE_CODING_AGENT_DIR", dir.path());
        Self {
            _guard: guard,
            saved,
            dir,
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("hoo-config.json")
    }
}

impl Drop for AgentHome {
    fn drop(&mut self) {
        match &self.saved {
            Some(value) => std::env::set_var("HOOCODE_CODING_AGENT_DIR", value),
            None => std::env::remove_var("HOOCODE_CODING_AGENT_DIR"),
        }
    }
}

/// A UI answering scripted clearance choices, counting prompts and notices.
struct ScriptedUi {
    answers: Mutex<VecDeque<ClearanceChoice>>,
    prompted: Mutex<usize>,
    notices: Mutex<Vec<String>>,
}

impl ScriptedUi {
    fn new(answers: Vec<ClearanceChoice>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            prompted: Mutex::new(0),
            notices: Mutex::new(Vec::new()),
        }
    }

    fn prompts(&self) -> usize {
        *self.prompted.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notices(&self) -> Vec<String> {
        self.notices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl PermissionUi for ScriptedUi {
    fn select(&self, _title: &str, _options: &[&str]) -> Option<String> {
        None
    }

    fn notify(&self, message: &str) {
        self.notices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(message.to_string());
    }

    fn select_clearance(&self, _info: &ClearanceInfo) -> ClearanceChoice {
        *self.prompted.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .unwrap_or(ClearanceChoice::Deny { reason: None })
    }
}

fn shell(command: &str) -> Value {
    json!({"command": command})
}

fn gate(cwd: &Path, ui: Arc<ScriptedUi>) -> HooPermissionGate {
    HooPermissionGate::new(
        cwd.to_path_buf(),
        hoocode_code_permissions::ApprovalChannel::Ui,
        Some(ui as Arc<dyn PermissionUi>),
    )
}

/// A fresh cwd per test: the session store is process-global, keyed by cwd.
fn fresh_cwd() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn program_is_the_first_word() {
    assert_eq!(shell_program("printf x | sort").as_deref(), Some("printf"));
    assert_eq!(shell_program("  wc -l src/wrap.rs").as_deref(), Some("wc"));
    assert_eq!(shell_program("").as_deref(), None);
    assert_eq!(shell_program("   ").as_deref(), None);
    assert_eq!(
        shell_program("\"mytool\" --flag").as_deref(),
        Some("mytool")
    );
}

#[test]
fn wrappers_match_basename_case_insensitive() {
    assert!(is_always_wrapper("sudo"));
    assert!(is_always_wrapper("SUDO"));
    assert!(is_always_wrapper("/bin/sh"));
    assert!(is_always_wrapper("/usr/bin/sudo"));
    assert!(is_always_wrapper("./sudo"));
    assert!(is_always_wrapper("cd"));
    assert!(is_always_wrapper("export"));
    assert!(is_always_wrapper("[["));
    assert!(is_always_wrapper("if"));
    assert!(!is_always_wrapper("printf"));
    assert!(!is_always_wrapper("sort"));
}

#[test]
fn allow_table() {
    let printf = |command: &str| always_allows(command, &["printf".to_string()]);
    // Covered: the program alone, with arguments, leading spaces.
    assert!(printf("printf"));
    assert!(printf("printf y"));
    assert!(printf("  printf y"));
    // Another program anywhere in the chain prompts.
    assert!(!printf("printf x; rm -rf /"));
    assert!(!printf("printf x && rm"));
    assert!(!printf("printf x || rm"));
    assert!(!printf("printf x | rm"));
    assert!(!printf("rm"));
    assert!(!printf("printfoo"));
    assert!(!printf(""));
    // A pipe where every program is approved stays covered.
    assert!(always_allows(
        "printf x | sort -r",
        &["printf".to_string(), "sort".to_string()]
    ));
    assert!(!always_allows(
        "printf x | sort -r",
        &["printf".to_string()]
    ));
    // Newlines, substitutions, expansions and here-docs prompt.
    assert!(!printf("printf x\nrm"));
    assert!(!printf("printf x\rrm"));
    assert!(!printf("printf $(rm -rf /)"));
    assert!(!printf("printf `rm -rf /`"));
    assert!(!printf("printf <(rm)"));
    assert!(!printf("printf >(rm)"));
    assert!(!printf("echo ${HOME}"));
    assert!(!printf("printf x <<EOF"));
    // Quoted markers are data, not code.
    assert!(printf("printf '$(not-a-command)'"));
    assert!(printf("printf 'a;b'"));
    // Assignments and wrappers prompt.
    assert!(!printf("VAR=1 printf"));
    assert!(!printf("sudo printf"));
    assert!(!always_allows("sudo printf", &["sudo".to_string()]));
    // Builtins and keywords never approve, even when listed.
    for builtin in [
        "cd", "export", "read", "if", "for", "time", "kill", "[[", "exec", "set",
    ] {
        assert!(
            !always_allows(builtin, &[builtin.to_string()]),
            "{builtin} must prompt"
        );
    }
    // File redirects prompt; adjacent descriptor dups and /dev/null do not.
    assert!(!printf("printf x > file"));
    assert!(!printf("printf x >> file"));
    assert!(!printf("printf x 2>file"));
    assert!(!printf("printf x 2> &1"));
    assert!(!printf("printf x > &2"));
    assert!(printf("printf x 2>&1"));
    assert!(printf("printf x > /dev/null"));
    // Slash programs are never approvable.
    assert!(!always_allows("./run.sh", &["./run.sh".to_string()]));
    assert!(!always_allows("/bin/printf x", &["printf".to_string()]));
    // Exact names otherwise: odd but harmless programs work.
    assert!(always_allows("c++ x", &["c++".to_string()]));
    // Overlong commands never auto-allow.
    assert!(!printf(&format!("printf {}", "y".repeat(40 * 1024))));
    assert!(printf(&format!("printf {}", "y".repeat(1024))));
}

#[test]
fn project_config_cannot_seed_always() {
    // A project config naming rm changes nothing: the key is gone, and the
    // session store starts empty for this cwd.
    let dir = fresh_cwd();
    let config = json!({
        "active_mode": "build",
        "modes": {"build": {"always_bash_programs": ["rm"]}}
    })
    .as_object()
    .unwrap()
    .clone();
    assert_eq!(
        evaluate(
            &config,
            dir.path(),
            "Shell",
            &shell("rm x"),
            ApprovalChannel::Ui,
        ),
        Verdict::Prompt
    );
}

#[test]
fn always_is_session_only_and_scoped() {
    let _home = AgentHome::new();
    let dir = fresh_cwd();
    let ui = Arc::new(ScriptedUi::new(vec![ClearanceChoice::Always]));
    // `a` on `printf x | sort`: allowed, remembered, nothing written.
    assert_eq!(
        gate(dir.path(), ui.clone()).check("Shell", &shell("printf x | sort -r")),
        None
    );
    assert_eq!(
        ui.notices(),
        vec!["\"printf\" always allowed in this session".to_string()]
    );
    assert!(!_home.path().exists());

    // A later `printf` runs free; `rm`, `printfoo` and chained `rm` prompt
    // (and are denied by the scripted UI).
    let ui = Arc::new(ScriptedUi::new(vec![
        ClearanceChoice::Deny { reason: None },
        ClearanceChoice::Deny { reason: None },
        ClearanceChoice::Deny { reason: None },
    ]));
    let gate = gate(dir.path(), ui.clone());
    assert_eq!(gate.check("Shell", &shell("printf y")), None);
    assert_eq!(ui.prompts(), 0);
    assert!(gate.check("Shell", &shell("rm -rf target")).is_some());
    assert!(gate.check("Shell", &shell("printfoo x")).is_some());
    assert!(gate.check("Shell", &shell("printf y; rm")).is_some());
    assert_eq!(ui.prompts(), 3);
}

#[test]
fn always_on_a_wrapper_runs_once_without_storing() {
    let _home = AgentHome::new();
    let dir = fresh_cwd();
    let ui = Arc::new(ScriptedUi::new(vec![ClearanceChoice::Always]));
    // `a` on a wrapper: allowed once, nothing stored, notice explains.
    assert_eq!(
        gate(dir.path(), ui.clone()).check("Shell", &shell("sudo rm -rf /tmp/x")),
        None
    );
    assert!(!_home.path().exists());
    assert_eq!(
        ui.notices(),
        vec!["\"always\" isn't offered here; ran once".to_string()]
    );
    // Nothing was learned: the same call prompts again.
    let ui = Arc::new(ScriptedUi::new(vec![ClearanceChoice::Deny {
        reason: None,
    }]));
    assert!(gate(dir.path(), ui.clone())
        .check("Shell", &shell("sudo rm -rf /tmp/x"))
        .is_some());
    assert_eq!(ui.prompts(), 1);
}

#[test]
fn always_scopes_other_tools_by_name() {
    let _home = AgentHome::new();
    let dir = fresh_cwd();
    let ui = Arc::new(ScriptedUi::new(vec![ClearanceChoice::Always]));
    assert_eq!(
        gate(dir.path(), ui.clone()).check("Edit", &json!({"path": "src/a.ts"})),
        None
    );
    let stored = std::fs::read_to_string(_home.path()).unwrap();
    assert!(stored.contains("\"auto_allow\""), "{stored}");
    assert!(stored.contains("\"Edit\""), "{stored}");
    // A later Edit runs free without prompting.
    let ui = Arc::new(ScriptedUi::new(vec![]));
    assert_eq!(
        gate(dir.path(), ui.clone()).check("Edit", &json!({"path": "src/b.ts"})),
        None
    );
    assert_eq!(ui.prompts(), 0);
}
