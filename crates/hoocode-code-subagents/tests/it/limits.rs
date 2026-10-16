//! Precedence of the subagent run limits: the `HOOCODE_` env var wins over the
//! setting, the setting over the default, and agent frontmatter over both.

#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hoocode_code_subagents::lifeguard::{
    PROGRESS_STALL_THRESHOLD_MS, SHELL_DEFAULT_TIMEOUT_ENV, STALL_THRESHOLD_ENV,
    SUBAGENT_DEADLINE_MS, SUBAGENT_SHELL_DEFAULT_TIMEOUT_SECS, TOOL_IN_FLIGHT_STALL_MS,
};
use hoocode_code_subagents::limits::{
    SubagentLimitSettings, SubagentLimits, DEADLINE_ENV, DEFAULT_QUEUE_PER_SLOT,
    DEFAULT_WAIT_TIMEOUT_MS, MAX_RSS_ENV, MAX_TURNS_ENV, QUEUE_PER_SLOT_ENV, TOKEN_BUDGET_ENV,
    TOOL_STALL_ENV, WAIT_TIMEOUT_ENV,
};
use hoocode_code_subagents::pool::{
    SubagentPool, SubagentPoolOptions, SubagentPoolTask, DEFAULT_SUBAGENT_MAX_TURNS,
};
use hoocode_code_subagents::runner::{BoxFuture, RunSpec, Runner, RunnerHandle};
use hoocode_runtime::MIB;
use tokio::sync::mpsc;

const MINUTE: u64 = 60 * 1000;

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn settings() -> SubagentLimitSettings {
    SubagentLimitSettings {
        max_turns: Some(75.0),
        token_budget: Some(90_000.0),
        deadline_minutes: Some(90.0),
        stall_seconds: Some(45.0),
        max_rss_mb: Some(1024.0),
        tool_stall_seconds: Some(600.0),
        queue_per_slot: Some(5.0),
        shell_timeout_seconds: Some(900.0),
        wait_timeout_seconds: Some(60.0),
    }
}

#[test]
fn defaults_apply_with_nothing_set() {
    let limits = SubagentLimits::resolve(&SubagentLimitSettings::default(), &env(&[]));
    assert_eq!(limits, SubagentLimits::default());
    assert_eq!(limits.max_turns, 200);
    assert_eq!(DEFAULT_SUBAGENT_MAX_TURNS, 200);
    assert_eq!(limits.token_budget_override, None);
    assert_eq!(limits.deadline_ms, 4 * 60 * MINUTE);
    assert_eq!(limits.deadline_ms, SUBAGENT_DEADLINE_MS);
    assert_eq!(limits.stall_ms, 20 * MINUTE);
    assert_eq!(limits.stall_ms, PROGRESS_STALL_THRESHOLD_MS);
    assert_eq!(limits.max_rss_bytes, 4096 * MIB);
    assert_eq!(limits.tool_stall_ms, 30 * MINUTE);
    assert_eq!(limits.tool_stall_ms, TOOL_IN_FLIGHT_STALL_MS);
    assert_eq!(limits.queue_per_slot, 8);
    assert_eq!(limits.queue_per_slot, DEFAULT_QUEUE_PER_SLOT);
    assert_eq!(limits.shell_timeout_secs, 1200.0);
    assert_eq!(
        limits.shell_timeout_secs,
        SUBAGENT_SHELL_DEFAULT_TIMEOUT_SECS
    );
    assert_eq!(limits.wait_timeout_ms, 300_000);
    assert_eq!(limits.wait_timeout_ms, DEFAULT_WAIT_TIMEOUT_MS);
}

#[test]
fn settings_apply_in_their_own_units() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[]));
    assert_eq!(limits.max_turns, 75);
    assert_eq!(limits.token_budget_override, Some(90_000));
    assert_eq!(limits.deadline_ms, 90 * MINUTE);
    assert_eq!(limits.stall_ms, 45_000);
    assert_eq!(limits.max_rss_bytes, 1024 * MIB);
    assert_eq!(limits.tool_stall_ms, 600_000);
    assert_eq!(limits.queue_per_slot, 5);
    assert_eq!(limits.shell_timeout_secs, 900.0);
    assert_eq!(limits.wait_timeout_ms, 60_000);
}

#[test]
fn env_wins_over_setting() {
    let limits = SubagentLimits::resolve(
        &settings(),
        &env(&[
            (MAX_TURNS_ENV, "30"),
            (TOKEN_BUDGET_ENV, "12345"),
            (DEADLINE_ENV, "60000"),
            (STALL_THRESHOLD_ENV, "5000"),
            (MAX_RSS_ENV, "512"),
        ]),
    );
    assert_eq!(limits.max_turns, 30);
    assert_eq!(limits.token_budget_override, Some(12_345));
    assert_eq!(limits.deadline_ms, 60_000, "env deadline is milliseconds");
    assert_eq!(limits.stall_ms, 5_000, "env stall is milliseconds");
    assert_eq!(limits.max_rss_bytes, 512 * MIB);
}

#[test]
fn env_values_are_trimmed_and_parsed() {
    let limits = SubagentLimits::resolve(
        &SubagentLimitSettings::default(),
        &env(&[(MAX_TURNS_ENV, " 42 ")]),
    );
    assert_eq!(limits.max_turns, 42);
}

#[test]
fn bad_env_falls_back_to_the_setting_then_the_default() {
    for bad in ["0", "-3", "soon", "", "NaN", "inf"] {
        let with_setting = SubagentLimits::resolve(&settings(), &env(&[(MAX_TURNS_ENV, bad)]));
        assert_eq!(
            with_setting.max_turns, 75,
            "{bad:?} falls back to the setting"
        );
        let with_nothing = SubagentLimits::resolve(
            &SubagentLimitSettings::default(),
            &env(&[(MAX_TURNS_ENV, bad)]),
        );
        assert_eq!(
            with_nothing.max_turns, DEFAULT_SUBAGENT_MAX_TURNS,
            "{bad:?} falls back to the default"
        );
    }
}

#[test]
fn bad_setting_falls_back_to_the_default() {
    let bad = SubagentLimitSettings {
        max_turns: Some(0.0),
        token_budget: Some(-1.0),
        deadline_minutes: Some(f64::NAN),
        stall_seconds: Some(0.0),
        max_rss_mb: Some(-5.0),
        tool_stall_seconds: Some(0.0),
        queue_per_slot: Some(0.0),
        shell_timeout_seconds: Some(-1.0),
        wait_timeout_seconds: Some(f64::INFINITY),
    };
    let limits = SubagentLimits::resolve(&bad, &env(&[]));
    assert_eq!(limits, SubagentLimits::default());
}

#[test]
fn fractional_values_are_floored() {
    let limits = SubagentLimits::resolve(
        &SubagentLimitSettings {
            max_turns: Some(75.9),
            ..Default::default()
        },
        &env(&[]),
    );
    assert_eq!(limits.max_turns, 75);
}

#[test]
fn frontmatter_max_turns_wins_over_env_and_setting() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[(MAX_TURNS_ENV, "30")]));
    assert_eq!(limits.turns_for(Some(12)), 12);
    assert_eq!(limits.turns_for(None), 30, "env beats setting");
    assert_eq!(
        limits.turns_for(Some(0)),
        30,
        "a zero frontmatter value is no value"
    );
}

#[test]
fn frontmatter_token_budget_wins_over_env_setting_and_type_default() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[(TOKEN_BUDGET_ENV, "12345")]));
    assert_eq!(limits.token_budget_for("explore", Some(5_000)), 5_000);
    assert_eq!(limits.token_budget_for("explore", None), 12_345);
    assert_eq!(limits.token_budget_for("general-purpose", None), 12_345);
}

#[test]
fn type_defaults_apply_when_no_override_is_set() {
    let limits = SubagentLimits::default();
    assert_eq!(limits.token_budget_for("explore", None), 150_000);
    assert_eq!(limits.token_budget_for("plan", None), 150_000);
    assert_eq!(limits.token_budget_for("general-purpose", None), 300_000);
    assert_eq!(limits.token_budget_for("custom-agent", None), 150_000);
    assert_eq!(limits.token_budget_for("general-purpose", Some(0)), 300_000);
}

#[test]
fn the_setting_budget_overrides_every_type() {
    let limits = SubagentLimits::resolve(
        &SubagentLimitSettings {
            token_budget: Some(90_000.0),
            ..Default::default()
        },
        &env(&[]),
    );
    assert_eq!(limits.token_budget_for("explore", None), 90_000);
    assert_eq!(limits.token_budget_for("general-purpose", None), 90_000);
}

#[test]
fn the_run_budget_follows_the_resolved_deadline() {
    let short = SubagentLimits::resolve(
        &SubagentLimitSettings {
            deadline_minutes: Some(10.0),
            ..Default::default()
        },
        &env(&[]),
    );
    assert_eq!(short.max_run_budget_ms(), 40 * MINUTE, "4x a short base");
    let default = SubagentLimits::default();
    assert_eq!(
        default.max_run_budget_ms(),
        SUBAGENT_DEADLINE_MS + 30 * MINUTE,
        "the default base plus the 30 minute headroom"
    );
}

#[test]
fn tool_stall_env_wins_over_setting_and_zero_falls_through() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[(TOOL_STALL_ENV, "90000")]));
    assert_eq!(limits.tool_stall_ms, 90_000, "env is milliseconds");
    let from_setting = SubagentLimits::resolve(&settings(), &env(&[(TOOL_STALL_ENV, "0")]));
    assert_eq!(
        from_setting.tool_stall_ms, 600_000,
        "zero env falls through"
    );
    let default = SubagentLimits::resolve(&SubagentLimitSettings::default(), &env(&[]));
    assert_eq!(default.tool_stall_ms, TOOL_IN_FLIGHT_STALL_MS);
}

#[test]
fn queue_per_slot_env_wins_over_setting_and_zero_falls_through() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[(QUEUE_PER_SLOT_ENV, "12")]));
    assert_eq!(limits.queue_per_slot, 12);
    let zero = SubagentLimits::resolve(&settings(), &env(&[(QUEUE_PER_SLOT_ENV, "0")]));
    assert_eq!(
        zero.queue_per_slot, 5,
        "zero env falls through to the setting"
    );
    let default = SubagentLimits::resolve(
        &SubagentLimitSettings::default(),
        &env(&[(QUEUE_PER_SLOT_ENV, "soon")]),
    );
    assert_eq!(
        default.queue_per_slot, 8,
        "junk env falls through to the default"
    );
}

#[test]
fn wait_timeout_env_wins_over_setting_and_zero_falls_through() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[(WAIT_TIMEOUT_ENV, "15000")]));
    assert_eq!(limits.wait_timeout_ms, 15_000, "env is milliseconds");
    let zero = SubagentLimits::resolve(&settings(), &env(&[(WAIT_TIMEOUT_ENV, "0")]));
    assert_eq!(
        zero.wait_timeout_ms, 60_000,
        "zero env falls through to the setting"
    );
    let default = SubagentLimits::resolve(&SubagentLimitSettings::default(), &env(&[]));
    assert_eq!(default.wait_timeout_ms, 300_000);
}

#[test]
fn shell_timeout_env_wins_over_setting_and_junk_falls_through() {
    let limits = SubagentLimits::resolve(&settings(), &env(&[(SHELL_DEFAULT_TIMEOUT_ENV, "45.5")]));
    assert_eq!(
        limits.shell_timeout_secs, 45.5,
        "env is seconds, fractions kept"
    );
    for bad in ["0", "-2", "nope", ""] {
        let fell = SubagentLimits::resolve(&settings(), &env(&[(SHELL_DEFAULT_TIMEOUT_ENV, bad)]));
        assert_eq!(
            fell.shell_timeout_secs, 900.0,
            "{bad:?} falls through to the setting"
        );
    }
    let default = SubagentLimits::resolve(&SubagentLimitSettings::default(), &env(&[]));
    assert_eq!(default.shell_timeout_secs, 1200.0);
}

/// Records every child's environment, writes a valid result so the dispatch
/// settles, and exits at once.
struct EnvRecorder {
    seen: Arc<Mutex<Vec<HashMap<String, String>>>>,
    cwd: PathBuf,
}

impl Runner for EnvRecorder {
    fn spawn(&self, spec: RunSpec) -> io::Result<Box<dyn RunnerHandle>> {
        self.seen.lock().unwrap().push(spec.env.clone());
        let dir = hoocode_code_paths::dispatch_task_dir(&self.cwd, &spec.task_id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join("result.json"),
            serde_json::json!({"summary": "done", "files_changed": [], "confidence": 0.9, "status": "complete"})
                .to_string(),
        )?;
        Ok(Box::new(QuietHandle))
    }
}

struct QuietHandle;

impl RunnerHandle for QuietHandle {
    fn take_streams(&mut self) -> (mpsc::Receiver<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
        let (_, stdout) = mpsc::channel(1);
        let (_, stderr) = mpsc::channel(1);
        (stdout, stderr)
    }

    fn wait(&mut self) -> BoxFuture<'static, io::Result<Option<i32>>> {
        Box::pin(async { Ok(Some(0)) })
    }

    fn kill(&mut self) {}

    fn pid(&self) -> u32 {
        0
    }
}

/// The resolved Shell timeout is written into every child's env, so the
/// child's Shell tool and the parent's stall bound agree. A value the
/// environment already names is replaced by the resolved one (the same value).
#[tokio::test]
async fn the_resolved_shell_timeout_reaches_every_child_env() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let pool = SubagentPool::new(SubagentPoolOptions {
        executable: PathBuf::from("/nonexistent/hoocode"),
        cwd: Some(dir.path().to_path_buf()),
        env: Some(HashMap::from([(
            "PATH".to_string(),
            "/usr/bin".to_string(),
        )])),
        limits: SubagentLimitSettings {
            shell_timeout_seconds: Some(900.0),
            ..Default::default()
        },
        runner: Some(Arc::new(EnvRecorder {
            seen: seen.clone(),
            cwd: dir.path().to_path_buf(),
        })),
        ..Default::default()
    });
    pool.spawn(SubagentPoolTask {
        task_id: "env-setting".into(),
        agent_type: "explore".into(),
        task: "look".into(),
        ..Default::default()
    })
    .unwrap();
    pool.wait_for("env-setting").await.unwrap();
    let envs = seen.lock().unwrap();
    assert_eq!(envs.len(), 1);
    assert_eq!(
        envs[0]
            .get("HOOCODE_BASH_DEFAULT_TIMEOUT_SECS")
            .map(String::as_str),
        Some("900"),
        "the setting reaches the child"
    );
}

#[test]
fn huge_settings_saturate_instead_of_overflowing() {
    // A deadline setting this large saturates to u64::MAX; the load ceiling
    // and the reconcile age derived from it must not overflow.
    let settings = SubagentLimitSettings {
        deadline_minutes: Some(1e300),
        queue_per_slot: Some(1e300),
        shell_timeout_seconds: Some(1e300),
        ..Default::default()
    };
    let limits = SubagentLimits::resolve(&settings, &env(&[]));
    assert_eq!(limits.deadline_ms, u64::MAX);
    assert_eq!(limits.max_run_budget_ms(), u64::MAX);
    assert_eq!(
        limits.shell_timeout_secs,
        hoocode_code_subagents::limits::MAX_SHELL_TIMEOUT_SECS,
        "the Shell timeout is clamped so the child's Duration conversion cannot panic"
    );
    assert!(limits.shell_timeout_secs.is_finite());
}
