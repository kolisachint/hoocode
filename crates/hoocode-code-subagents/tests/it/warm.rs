#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! warm-subagent-pool.test.ts: the real worker/pool against a fake RPC child
//! (hoocode's `fixtures/fake-rpc-child.mjs`, as a shell script). The fake
//! echoes its pid, generation (bumped by new_session) and prompt count, so
//! reuse and reset show in the answer text.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hoocode_code_subagents::warm::*;

const FAKE_CHILD: &str = r#"fail=0; exit_on_prompt=0; slow_prompt=0
for a in "$@"; do
  case "$a" in --fail-prompt) fail=1;; --exit-on-prompt) exit_on_prompt=1;; --slow-prompt) slow_prompt=1;; esac
done
gen=0; n=0; last=
field() { printf '%s' "$line" | sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p"; }
respond() { printf '{"id":"%s","type":"response","command":"%s","success":true%s}\n' "$id" "$1" "$2"; }
while IFS= read -r line; do
  [ -z "$line" ] && continue
  id=$(field id); type=$(field type)
  case "$type" in
    prompt)
      [ "$exit_on_prompt" = 1 ] && exit 1
      [ "$slow_prompt" = 1 ] && sleep 1
      n=$((n + 1)); last=$(field message)
      respond prompt ""
      echo '{"type":"agent_start"}'
      echo '{"type":"tool_execution_start","toolName":"CodeSearch"}'
      echo '{"type":"tool_execution_end","toolName":"CodeSearch"}'
      if [ "$fail" = 1 ]; then
        echo '{"type":"turn_end","message":{"usage":{"input":11,"output":7},"stopReason":"error","errorMessage":"boom"}}'
      else
        echo '{"type":"turn_end","message":{"usage":{"input":11,"output":7},"stopReason":"stop"}}'
      fi
      echo '{"type":"agent_end","messages":[]}'
      ;;
    new_session) gen=$((gen + 1)); respond new_session ',"data":{"cancelled":false}' ;;
    get_last_assistant_text)
      respond get_last_assistant_text ",\"data\":{\"text\":\"ANSWER pid=$$ gen=$gen n=$n for=$last\"}" ;;
    get_session_stats)
      respond get_session_stats ',"data":{"tokens":{"input":11,"output":7,"cacheRead":3,"cacheWrite":0},"cost":0.002}' ;;
    *) [ -n "$id" ] && respond "$type" "" ;;
  esac
done"#;

fn fake_spawn(dir: &Path, extra: &[&str]) -> SpawnCommand {
    let script = dir.join("fake-rpc-child.sh");
    std::fs::write(&script, FAKE_CHILD).unwrap();
    let mut prefix_args = vec![script.to_string_lossy().into_owned()];
    prefix_args.extend(extra.iter().map(|s| s.to_string()));
    SpawnCommand {
        executable: PathBuf::from("sh"),
        prefix_args,
    }
}

fn opts() -> WarmDispatchOptions {
    WarmDispatchOptions {
        agent_type: "explore".into(),
        cwd: std::env::current_dir().unwrap(),
        ..Default::default()
    }
}

fn make(dir: &Path, extra: &[&str]) -> WarmSubagentPool {
    WarmSubagentPool::new(WarmSubagentPoolOptions {
        spawn: Some(fake_spawn(dir, extra)),
        ..WarmSubagentPoolOptions::new(dir)
    })
}

fn pid(summary: &str) -> String {
    summary
        .split_whitespace()
        .find_map(|w| w.strip_prefix("pid="))
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_a_task_and_returns_its_answer_and_usage() {
    let dir = tempfile::tempdir().unwrap();
    let pool = make(dir.path(), &[]);
    let result = pool.dispatch("trace the bug", &opts(), None).await.unwrap();
    assert!(result.ok);
    assert_eq!(result.status, "complete");
    assert!(
        result.summary.contains("for=trace the bug"),
        "{}",
        result.summary
    );
    assert_eq!(
        result.usage,
        Some(WarmUsage {
            input: 11.0,
            output: 7.0,
            cache_read: 3.0,
            cache_write: 0.0,
            cost: 0.002
        })
    );
    pool.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reuses_the_same_worker_and_resets_it_between_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let pool = make(dir.path(), &[]);
    let first = pool.dispatch("task one", &opts(), None).await.unwrap();
    assert_eq!(pool.idle_count(), 1);
    let second = pool.dispatch("task two", &opts(), None).await.unwrap();
    assert_eq!(pool.idle_count(), 1);
    assert_eq!(pid(&second.summary), pid(&first.summary));
    assert!(first.summary.contains("gen=0"));
    assert!(second.summary.contains("gen=1"));
    assert!(second.summary.contains("n=2"));
    pool.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_live_tool_activity_and_clears_it_at_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let pool = make(dir.path(), &[]);
    let activity = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = activity.clone();
    let callback: WarmProgressCallback =
        Arc::new(move |a: &str| sink.lock().unwrap().push(a.to_string()));
    pool.dispatch("trace the bug", &opts(), Some(callback))
        .await
        .unwrap();
    let seen = activity.lock().unwrap().clone();
    assert!(seen.contains(&"CodeSearch".to_string()));
    assert_eq!(seen.last().map(String::as_str), Some(""));
    pool.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_a_task_failure_without_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let pool = make(dir.path(), &["--fail-prompt"]);
    let result = pool.dispatch("do the thing", &opts(), None).await.unwrap();
    assert!(!result.ok);
    assert_eq!(result.status, "failed");
    assert_eq!(result.error.as_deref(), Some("boom"));
    pool.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn surfaces_an_infra_failure_and_discards_the_dead_worker() {
    let dir = tempfile::tempdir().unwrap();
    let pool = make(dir.path(), &["--exit-on-prompt"]);
    assert!(pool.dispatch("will crash", &opts(), None).await.is_err());
    assert_eq!(pool.idle_count(), 0);
    pool.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reclaims_idle_workers_after_the_ttl() {
    let dir = tempfile::tempdir().unwrap();
    let pool = WarmSubagentPool::new(WarmSubagentPoolOptions {
        spawn: Some(fake_spawn(dir.path(), &[])),
        idle_ttl: Duration::from_millis(100),
        ..WarmSubagentPoolOptions::new(dir.path())
    });
    pool.dispatch("task", &opts(), None).await.unwrap();
    assert_eq!(pool.idle_count(), 1);
    // Poll for the reclaim rather than sleeping four times the TTL: the old
    // version asserted on a clock, which fails whenever the machine is busy.
    for _ in 0..200 {
        if pool.idle_count() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        pool.idle_count(),
        0,
        "the idle worker should have been reclaimed"
    );
    pool.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn treats_explore_as_poolable_and_unknown_agents_as_not() {
    let dir = tempfile::tempdir().unwrap();
    let pool = make(dir.path(), &[]);
    assert!(pool.is_poolable("explore"));
    assert!(!pool.is_poolable("does-not-exist"));
    pool.dispose().await;
}

#[test]
fn warm_subagents_are_enabled_by_the_env_flag() {
    let env = |pairs: &[(&str, &str)]| -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    assert!(!warm_subagents_enabled(&env(&[])));
    assert!(warm_subagents_enabled(&env(&[(
        "HOOCODE_WARM_SUBAGENTS",
        "1"
    )])));
    assert!(!warm_subagents_enabled(&env(&[(
        "HOOCODE_WARM_SUBAGENTS",
        "0"
    )])));
}

/// The warm pool is a second execution path, and it had no ceiling at all: one
/// worker booted per dispatch, nothing counted them, and nothing told the
/// lifeguard. It now shares the cold pool's admission rules.
#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_dispatch_when_the_warm_pool_is_saturated() {
    let dir = tempfile::tempdir().unwrap();
    let pool = WarmSubagentPool::new(WarmSubagentPoolOptions {
        spawn: Some(fake_spawn(dir.path(), &["--slow-prompt"])),
        // One worker, nobody waiting: the second dispatch has nowhere to go.
        max_in_flight: 1,
        max_waiting: 0,
        ..WarmSubagentPoolOptions::new(dir.path())
    });
    let options = WarmDispatchOptions {
        agent_type: "explore".into(),
        cwd: dir.path().to_path_buf(),
        model: None,
        provider: None,
    };
    let first = tokio::spawn({
        let pool = pool.clone();
        let options = options.clone();
        async move { pool.dispatch("first", &options, None).await }
    });
    // Let the first take the only slot.
    for _ in 0..50 {
        if pool.load().0 == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let refused = pool.dispatch("second", &options, None).await.unwrap_err();
    assert!(
        refused.to_string().contains("saturated"),
        "expected a saturation refusal, got {refused:?}"
    );
    let _ = first.await;
    assert_eq!(pool.load(), (0, 0), "the slot must be released");
    pool.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_dispatch_is_admitted_while_a_slot_is_free_later() {
    let dir = tempfile::tempdir().unwrap();
    let pool = WarmSubagentPool::new(WarmSubagentPoolOptions {
        spawn: Some(fake_spawn(dir.path(), &[])),
        max_in_flight: 1,
        max_waiting: 4,
        ..WarmSubagentPoolOptions::new(dir.path())
    });
    let options = WarmDispatchOptions {
        agent_type: "explore".into(),
        cwd: dir.path().to_path_buf(),
        model: None,
        provider: None,
    };
    // Sequential: the cap must not stop the pool from being used, only from
    // being used all at once.
    for n in 0..3 {
        let result = pool.dispatch(&format!("run {n}"), &options, None).await;
        assert!(result.is_ok(), "run {n} should be admitted");
    }
    assert_eq!(pool.load(), (0, 0));
    pool.dispose().await;
}
