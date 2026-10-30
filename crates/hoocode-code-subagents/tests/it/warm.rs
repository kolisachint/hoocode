#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! warm-subagent-pool.test.ts: the real worker/pool against a fake RPC child
//! (hoocode's `fixtures/fake-rpc-child.mjs`, as a shell script). The fake
//! echoes its pid, generation (bumped by new_session) and prompt count, so
//! reuse and reset show in the answer text.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hoocode_code_subagents::warm::*;

const FAKE_CHILD: &str = r#"fail=0; exit_on_prompt=0; slow_prompt=0; hang_prompt=0; earlier=false
for a in "$@"; do
  case "$a" in --fail-prompt) fail=1;; --exit-on-prompt) exit_on_prompt=1;; --slow-prompt) slow_prompt=1;; --hang-prompt) hang_prompt=1;; --earlier-text) earlier=true;; esac
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
      [ "$hang_prompt" = 1 ] && sleep 30
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
    get_last_assistant_text_with_origin)
      respond get_last_assistant_text_with_origin ",\"data\":{\"text\":\"ANSWER pid=$$ gen=$gen n=$n for=$last\",\"stale\":$earlier}" ;;
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
        // The worker gets the same child prompt as a cold run (card 9, 4i).
        result.summary.contains("for=Task: trace the bug"),
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
async fn marks_a_summary_taken_from_an_earlier_turn() {
    let dir = tempfile::tempdir().unwrap();
    let pool = make(dir.path(), &["--earlier-text"]);
    let result = pool.dispatch("trace the bug", &opts(), None).await.unwrap();
    assert!(
        result
            .summary
            .starts_with("(from an earlier turn) ANSWER pid="),
        "{}",
        result.summary
    );
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
        inherited_model: None,
        effort: None,
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
        inherited_model: None,
        effort: None,
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

/// The warm pool sizes itself by depth like the cold pool: a nested process
/// gets the nested cap, not the root's, and its queue is QUEUE_PER_SLOT times
/// that cap.
#[test]
fn a_nested_warm_pool_gets_the_nested_cap_not_the_root_cap() {
    let dir = tempfile::tempdir().unwrap();
    let env = |pairs: &[(&str, &str)]| -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let root = WarmSubagentPoolOptions::for_depth(dir.path(), None, None, &env(&[]));
    assert_eq!((root.max_in_flight, root.max_waiting), (12, 96));

    for depth in ["1", "2"] {
        let nested = WarmSubagentPoolOptions::for_depth(
            dir.path(),
            None,
            None,
            &env(&[("HOOCODE_SUBAGENT_DEPTH", depth)]),
        );
        assert_eq!(
            (nested.max_in_flight, nested.max_waiting),
            (6, 48),
            "depth {depth} must get the nested cap"
        );
    }

    let configured = WarmSubagentPoolOptions::for_depth(
        dir.path(),
        Some(12.0),
        Some(2.0),
        &env(&[("HOOCODE_SUBAGENT_DEPTH", "1")]),
    );
    assert_eq!((configured.max_in_flight, configured.max_waiting), (2, 16));
}

/// A fake RPC child that logs `start` and `end` around each prompt, so the
/// number of prompts running at once can be read back from the log.
const TRACED_CHILD: &str = r#"trace=; prev=
for a in "$@"; do [ "$prev" = "--trace" ] && trace=$a; prev=$a; done
while IFS= read -r line; do
  [ -z "$line" ] && continue
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  type=$(printf '%s' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  case "$type" in
    prompt)
      [ -n "$trace" ] && echo start >> "$trace"
      sleep 0.3
      [ -n "$trace" ] && echo end >> "$trace"
      printf '{"id":"%s","type":"response","command":"prompt","success":true}\n' "$id"
      echo '{"type":"agent_start"}'
      echo '{"type":"turn_end","message":{"usage":{"input":1,"output":1},"stopReason":"stop"}}'
      echo '{"type":"agent_end","messages":[]}'
      ;;
    get_last_assistant_text)
      printf '{"id":"%s","type":"response","command":"get_last_assistant_text","success":true,"data":{"text":"ok"}}\n' "$id" ;;
    get_session_stats)
      printf '{"id":"%s","type":"response","command":"get_session_stats","success":true,"data":{"tokens":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0},"cost":0}}\n' "$id" ;;
    *) [ -n "$id" ] && printf '{"id":"%s","type":"response","command":"%s","success":true}\n' "$id" "$type" ;;
  esac
done"#;

/// Dispatches that arrive together must not all boot and run at once: the
/// slot is taken at admission, not after the worker has booted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_dispatches_never_runs_more_than_max_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    let trace = dir.path().join("prompts.log");
    let script = dir.path().join("traced-rpc-child.sh");
    std::fs::write(&script, TRACED_CHILD).unwrap();
    let pool = WarmSubagentPool::new(WarmSubagentPoolOptions {
        spawn: Some(SpawnCommand {
            executable: PathBuf::from("sh"),
            prefix_args: vec![
                script.to_string_lossy().into_owned(),
                "--trace".into(),
                trace.to_string_lossy().into_owned(),
            ],
        }),
        max_in_flight: 2,
        max_waiting: 16,
        ..WarmSubagentPoolOptions::new(dir.path())
    });
    let options = WarmDispatchOptions {
        agent_type: "explore".into(),
        cwd: dir.path().to_path_buf(),
        model: None,
        provider: None,
        inherited_model: None,
        effort: None,
    };

    let runs: Vec<_> = (0..6)
        .map(|n| {
            let pool = pool.clone();
            let options = options.clone();
            tokio::spawn(async move { pool.dispatch(&format!("run {n}"), &options, None).await })
        })
        .collect();
    for run in runs {
        let result = run.await.unwrap();
        assert!(result.is_ok(), "every run should be admitted: {result:?}");
    }

    let log = std::fs::read_to_string(&trace).unwrap();
    let (mut running, mut peak) = (0i32, 0i32);
    for line in log.lines() {
        match line {
            "start" => running += 1,
            "end" => running -= 1,
            _ => {}
        }
        peak = peak.max(running);
    }
    assert_eq!(log.lines().filter(|l| *l == "start").count(), 6);
    assert!(
        peak <= 2,
        "peak concurrent prompts was {peak}, above max_in_flight 2"
    );
    assert_eq!(pool.load(), (0, 0), "every slot must be released");
    pool.dispose().await;
}

/// A warm run asked to stop is stopped: its task is dropped, its worker
/// discarded (its child disposed) and the result is `cancelled`, not a late
/// `complete`. The slot comes back.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_warm_dispatch_stops_its_worker_and_reports_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let pool = make(dir.path(), &["--hang-prompt"]);
    let started = std::time::Instant::now();
    let result = pool
        .dispatch_cancellable(
            "never finishes",
            &opts(),
            None,
            tokio::time::sleep(Duration::from_millis(500)),
        )
        .await
        .unwrap();
    assert_eq!(result, WarmRunResult::cancelled());
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(pool.load(), (0, 0), "the slot must be released");
    pool.dispose().await;
}

/// The same through the Agent and AgentOutput tools, with warm subagents on:
/// while the warm run works, its status says running; a cancel stops it and
/// says so only once it has stopped, and the call reports `cancelled`.
#[tokio::test(flavor = "multi_thread")]
async fn a_warm_background_run_reports_its_status_and_cancels_truthfully() {
    use hoocode_ai_types::Content;
    use hoocode_code_subagents::inbox::{subagent_inbox, TaskLifecycle};
    use hoocode_code_subagents::instance::set_subagent_pool_for_testing;
    use hoocode_code_subagents::pool::{SubagentPool, SubagentPoolOptions};
    use hoocode_code_subagents::tools::{
        create_task_output_tool_definition, create_task_tool_definition,
    };
    use hoocode_code_tool_api::ToolContext;
    use serde_json::json;

    let _serial = crate::SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    std::env::set_var("HOOCODE_WARM_SUBAGENTS", "1");
    let warm = make(&cwd, &["--hang-prompt"]);
    set_warm_subagent_pool_for_testing(Some(warm.clone()));
    // A cold pool the cancel asks first; it does not hold warm runs.
    let cold = SubagentPool::new(SubagentPoolOptions {
        executable: PathBuf::from("/bin/false"),
        cwd: Some(cwd.clone()),
        ..Default::default()
    });
    set_subagent_pool_for_testing(Some(cold.clone()));
    let inbox = subagent_inbox();
    inbox.clear();
    let ctx = ToolContext {
        cwd: Some(cwd.clone()),
        ..Default::default()
    };
    let text = |r: &hoocode_agent_types::AgentToolResult| match &r.content[0] {
        Content::Text(t) => t.text.clone(),
        other => panic!("{other:?}"),
    };

    let tool = create_task_tool_definition(&cwd);
    let call = hoocode_agent_types::AgentToolCall {
        id: "call_0".into(),
        name: "Agent".into(),
        arguments: json!({"description": "map it", "prompt": "Map it.", "subagent_type": "explore", "background": true}),
    };
    (tool.background_placeholder.clone().unwrap())(&call, Some(&ctx)).unwrap();
    let running = {
        let (execute, ctx, args) = (tool.execute.clone(), ctx.clone(), call.arguments.clone());
        tokio::task::spawn_blocking(move || {
            execute("call_0".into(), args, None, None, Some(&ctx)).map_err(|e| e.to_string())
        })
    };
    // Bound and working on its warm worker.
    for _ in 0..500 {
        if warm.load().0 == 1 && inbox.get("explore#1").is_some_and(|r| !r.starting) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(warm.load().0, 1, "the warm run is working");

    let output = create_task_output_tool_definition();
    let ask = |args: serde_json::Value| {
        let (execute, ctx) = (output.execute.clone(), ctx.clone());
        tokio::task::spawn_blocking(move || {
            execute("o".into(), args, None, None, Some(&ctx)).map_err(|e| e.to_string())
        })
    };
    let status = ask(json!({"task_id": "explore#1"})).await.unwrap().unwrap();
    assert!(
        text(&status).starts_with("explore#1 (\"map it\") is still running"),
        "{}",
        text(&status)
    );
    let cancel = ask(json!({"task_id": "explore#1", "cancel": true}))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        text(&cancel),
        "Cancelled explore#1 (\"map it\"). Only this run was stopped."
    );
    let result = running.await.unwrap().unwrap();
    assert!(
        text(&result).starts_with("explore#1 (\"map it\") cancelled ⊘"),
        "{}",
        text(&result)
    );
    assert_eq!(
        inbox.get("explore#1").unwrap().lifecycle,
        TaskLifecycle::Cancelled
    );
    assert_eq!(warm.load(), (0, 0));

    std::env::remove_var("HOOCODE_WARM_SUBAGENTS");
    set_warm_subagent_pool_for_testing(None);
    set_subagent_pool_for_testing(None);
    cold.dispose();
    warm.dispose().await;
    inbox.clear();
}
