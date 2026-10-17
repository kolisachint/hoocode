//! `a always` on the clearance card scopes to the Shell program (M6 review:
//! `a` on `printf …` must not auto-allow `rm`). The gate stores an anchored
//! rule; a later `rm` still prompts, a later `printf` does not. The global
//! config is redirected to a scratch dir, restored afterwards.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

use hoocode_code_permissions::{
    always_allows, evaluate, shell_program, ApprovalChannel, ClearanceChoice, ClearanceInfo,
    HooPermissionGate, PermissionUi, Verdict,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Redirects the agent dir at `HOOCODE_CODING_AGENT_DIR`, restoring it on drop.
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

fn gate(cwd: &std::path::Path, ui: Arc<ScriptedUi>) -> HooPermissionGate {
    HooPermissionGate::new(
        cwd.to_path_buf(),
        hoocode_code_permissions::ApprovalChannel::Ui,
        Some(ui as Arc<dyn PermissionUi>),
    )
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
    // File redirects prompt; descriptor dups and /dev/null do not.
    assert!(!printf("printf x > file"));
    assert!(!printf("printf x >> file"));
    assert!(!printf("printf x 2>file"));
    assert!(printf("printf x 2>&1"));
    assert!(printf("printf x > /dev/null"));
    // Exact names: odd but harmless programs work, lookalikes do not.
    assert!(always_allows("c++ x", &["c++".to_string()]));
    assert!(always_allows("./run.sh", &["./run.sh".to_string()]));
    assert!(!always_allows("./runXsh", &["./run.sh".to_string()]));
}
#[test]
fn gate_defers_to_the_rule() {
    let config = json!({
        "active_mode": "build",
        "modes": {"build": {"always_bash_programs": ["printf"]}}
    });
    let config = config.as_object().unwrap().clone();
    let cwd = tempfile::tempdir().unwrap();
    let allow = |command: &str| {
        evaluate(
            &config,
            cwd.path(),
            "Shell",
            &shell(command),
            ApprovalChannel::Ui,
        )
    };
    assert_eq!(allow("printf y"), Verdict::Allow);
    assert_eq!(allow("printf y | sort"), Verdict::Prompt);
    assert_eq!(allow("printf y; rm"), Verdict::Prompt);
    assert_eq!(allow("rm -rf target"), Verdict::Prompt);
}

#[test]
fn always_scopes_to_the_shell_program() {
    let _home = AgentHome::new();
    let cwd = tempfile::tempdir().unwrap();
    let ui = Arc::new(ScriptedUi::new(vec![ClearanceChoice::Always]));
    // `a` on `printf x | sort`: allowed, and the program name is stored.
    assert_eq!(
        gate(cwd.path(), ui.clone()).check("Shell", &shell("printf x | sort -r")),
        None
    );
    let stored = std::fs::read_to_string(_home.path()).unwrap();
    assert!(stored.contains("\"always_bash_programs\""), "{stored}");
    assert!(stored.contains("\"printf\""), "{stored}");
    assert!(!stored.contains("auto_allow"), "{stored}");

    // A later `printf` runs free; `rm` and `printfoo` still prompt (and are
    // denied by the scripted UI). A chained `printf` with `rm` prompts too.
    let ui = Arc::new(ScriptedUi::new(vec![
        ClearanceChoice::Deny { reason: None },
        ClearanceChoice::Deny { reason: None },
        ClearanceChoice::Deny { reason: None },
    ]));
    let gate = gate(cwd.path(), ui.clone());
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
    let cwd = tempfile::tempdir().unwrap();
    let ui = Arc::new(ScriptedUi::new(vec![ClearanceChoice::Always]));
    // `a` on a wrapper: allowed once, nothing stored, notice explains.
    assert_eq!(
        gate(cwd.path(), ui.clone()).check("Shell", &shell("sudo rm -rf /tmp/x")),
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
    assert!(gate(cwd.path(), ui.clone())
        .check("Shell", &shell("sudo rm -rf /tmp/x"))
        .is_some());
    assert_eq!(ui.prompts(), 1);
}

#[test]
fn always_scopes_other_tools_by_name() {
    let _home = AgentHome::new();
    let cwd = tempfile::tempdir().unwrap();
    let ui = Arc::new(ScriptedUi::new(vec![ClearanceChoice::Always]));
    assert_eq!(
        gate(cwd.path(), ui.clone()).check("Edit", &json!({"path": "src/a.ts"})),
        None
    );
    let stored = std::fs::read_to_string(_home.path()).unwrap();
    assert!(stored.contains("\"auto_allow\""), "{stored}");
    assert!(stored.contains("\"Edit\""), "{stored}");
    // A later Edit runs free without prompting.
    let ui = Arc::new(ScriptedUi::new(vec![]));
    assert_eq!(
        gate(cwd.path(), ui.clone()).check("Edit", &json!({"path": "src/b.ts"})),
        None
    );
    assert_eq!(ui.prompts(), 0);
}
