//! `dispatch/ledger.jsonl`: an append-only record of every dispatch **attempt**.
//!
//! Why this exists: the only evidence of how subagents behave was whatever
//! dispatch dirs survived on disk — and a clean success deletes its own dir
//! (`pool.rs`), so every dir left was a failure and the rest was invisible. Ten
//! recorded runs, zero successes, and no way to answer "what is the success
//! rate?" without archaeology. `docs/design/subagents.md` §1 has that write-up;
//! §7 notes the raw evidence is swept after 24h and mostly already gone.
//!
//! One line per attempt (not per dispatch): a run that failed on its preferred
//! model and succeeded on the inherited one is two lines, which is exactly the
//! shape the reliability question needs. Nothing here is authoritative for
//! correctness — `result.json` is. This is telemetry: it is best-effort, an
//! unwritable ledger never fails a run, and a corrupt line is skipped.
//!
//! Read side: [`stats`] aggregates a window; [`crate::stats`] and the
//! `/subagent-stats` command render it.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// The ledger's file name inside the dispatch root.
pub const LEDGER_FILE_NAME: &str = "ledger.jsonl";

/// Lines kept before the file is rewritten. A long session would otherwise grow
/// one line per attempt forever; 20k attempts is far more than any window a
/// reader looks at, and the rewrite keeps the newest [`KEEP_LINES`].
const MAX_LINES: usize = 20_000;
/// How much survives a prune.
const KEEP_LINES: usize = 10_000;
/// An error string is a sentence, not a log. The full text is in `output.json`.
const MAX_ERROR_CHARS: usize = 300;

/// One child's life, from spawn to settle. Hand-serialized through `serde_json`
/// rather than derived: this crate depends on `serde_json` only, and a
/// telemetry record must not drag a dependency into every dispatch path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DispatchAttempt {
    /// Epoch milliseconds when the attempt settled.
    pub ts: u64,
    pub task_id: String,
    pub agent_type: String,
    /// `background` when the caller asked for a notification, else `blocking`.
    pub mode: String,
    /// Nesting depth of the child (1 = dispatched by the root session).
    pub depth: u8,
    /// What the caller asked for: a concrete model, a `complexity` category, or
    /// nothing (inherit).
    pub requested_model: Option<String>,
    /// What `build_args` resolved it to, when that is known.
    pub resolved_model: Option<String>,
    pub provider: Option<String>,
    /// True when this attempt is the inherited-model retry, not the first try.
    pub fallback_attempt: bool,
    /// `complete` | `partial` | `failed` | `stalled` | `timeout` | `cancelled`.
    pub status: String,
    /// The parent's verdict: a verified result, or nothing usable.
    pub ok: bool,
    /// `result.json` passed the output verifier.
    pub verified: bool,
    /// The child's self-reported confidence, when it wrote one.
    pub confidence: Option<f64>,
    pub duration_ms: u64,
    pub tokens_generated: u64,
    pub peak_context: u64,
    pub exit_code: Option<i32>,
    pub budget_exceeded: Option<bool>,
    /// A one-line cause: truncation, newlines collapsed.
    pub error: Option<String>,
}

/// `<cwd>/.hoocode/dispatch/ledger.jsonl`.
pub fn ledger_path(cwd: &Path) -> PathBuf {
    hoocode_code_paths::dispatch_root(cwd).join(LEDGER_FILE_NAME)
}

/// Collapse a cause to one bounded line.
fn clean_error(text: &str) -> Option<String> {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(collapsed.chars().take(MAX_ERROR_CHARS).collect())
}

impl DispatchAttempt {
    /// One JSONL line, without the newline. Serializing a `Value` is
    /// infallible.
    pub fn to_line(&self) -> String {
        json!({
            "ts": self.ts,
            "task_id": self.task_id,
            "agent_type": self.agent_type,
            "mode": self.mode,
            "depth": self.depth,
            "requested_model": self.requested_model,
            "resolved_model": self.resolved_model,
            "provider": self.provider,
            "fallback_attempt": self.fallback_attempt,
            "status": self.status,
            "ok": self.ok,
            "verified": self.verified,
            "confidence": self.confidence,
            "duration_ms": self.duration_ms,
            "tokens_generated": self.tokens_generated,
            "peak_context": self.peak_context,
            "exit_code": self.exit_code,
            "budget_exceeded": self.budget_exceeded,
            "error": self.error,
        })
        .to_string()
    }

    /// Parse one line. A truncated, hand-edited or foreign line yields `None`
    /// rather than failing the read: telemetry must not become an error path.
    /// Unknown keys are ignored and missing keys take their defaults, so an
    /// older ledger stays readable after this file grows fields.
    pub fn from_line(line: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        let object: &Map<String, Value> = value.as_object()?;
        let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_string);
        let number = |key: &str| object.get(key).and_then(Value::as_u64);
        let flag = |key: &str| object.get(key).and_then(Value::as_bool);
        Some(Self {
            ts: number("ts").unwrap_or(0),
            task_id: text("task_id").unwrap_or_default(),
            agent_type: text("agent_type").unwrap_or_default(),
            mode: text("mode").unwrap_or_default(),
            depth: number("depth").unwrap_or(0) as u8,
            requested_model: text("requested_model"),
            resolved_model: text("resolved_model"),
            provider: text("provider"),
            fallback_attempt: flag("fallback_attempt").unwrap_or(false),
            status: text("status").unwrap_or_default(),
            ok: flag("ok").unwrap_or(false),
            verified: flag("verified").unwrap_or(false),
            confidence: object.get("confidence").and_then(Value::as_f64),
            duration_ms: number("duration_ms").unwrap_or(0),
            tokens_generated: number("tokens_generated").unwrap_or(0),
            peak_context: number("peak_context").unwrap_or(0),
            exit_code: object
                .get("exit_code")
                .and_then(Value::as_i64)
                .map(|v| v as i32),
            budget_exceeded: flag("budget_exceeded"),
            error: text("error"),
        })
    }
}

/// Append one attempt. Best-effort by design: a ledger that cannot be written
/// (read-only project, full disk) must never fail a dispatch, so every error
/// here is swallowed after a debug log line.
pub fn append(cwd: &Path, attempt: &DispatchAttempt) {
    let path = ledger_path(cwd);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            crate::agent_log::agent_log(&format!(
                "[LEDGER] cannot create {} for task {}",
                parent.display(),
                attempt.task_id
            ));
            return;
        }
    }
    if let Err(error) = append_line(&path, &attempt.to_line()) {
        crate::agent_log::agent_log(&format!("[LEDGER] append failed: {error}"));
    }
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    let existing = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if existing > 0 {
        prune_if_needed(path)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    // One write, newline included. A dispatch pool settles tasks on several
    // threads, and two `write_all` calls interleave: an eval run with eight
    // concurrent dispatches produced exactly one unparseable line before this.
    let mut record = String::with_capacity(line.len() + 1);
    record.push_str(line);
    record.push('\n');
    file.write_all(record.as_bytes())
}

/// Keep the file bounded. Counts lines rather than bytes so the cost stays
/// predictable; a rewrite only happens once per `MAX_LINES - KEEP_LINES`
/// appends.
fn prune_if_needed(path: &Path) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX_LINES {
        return Ok(());
    }
    let kept = &lines[lines.len() - KEEP_LINES..];
    let mut buffer = String::with_capacity(text.len() / 2);
    for line in kept {
        buffer.push_str(line);
        buffer.push('\n');
    }
    // Write beside it and rename over it. `fs::write` truncates first, so a
    // reader in another process (`/subagent-stats`, a bot's `/healthz`) could
    // see an empty or half-written ledger for the length of the rewrite.
    let tmp = path.with_extension(format!("jsonl.{}.tmp", std::process::id()));
    std::fs::write(&tmp, buffer)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Every attempt in the ledger, oldest first. Unparseable lines are skipped.
pub fn read_all(cwd: &Path) -> Vec<DispatchAttempt> {
    let Ok(text) = std::fs::read_to_string(ledger_path(cwd)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(DispatchAttempt::from_line)
        .collect()
}

/// The newest `limit` attempts, oldest first.
pub fn recent(cwd: &Path, limit: usize) -> Vec<DispatchAttempt> {
    let mut all = read_all(cwd);
    if all.len() > limit {
        all.drain(..all.len() - limit);
    }
    all
}

/// `AgentStats`: one agent type's tally.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentStats {
    pub attempts: usize,
    /// `complete` + `partial`: the run produced findings the parent can use.
    pub usable: usize,
    pub failed: usize,
    pub tokens_generated: u64,
    pub wall_ms: u64,
}

/// `LedgerStats`: the aggregate a reader actually wants.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LedgerStats {
    pub attempts: usize,
    /// Attempts that ended `complete` or `partial`.
    pub usable: usize,
    pub complete: usize,
    pub partial: usize,
    pub failed: usize,
    pub stalled: usize,
    pub timeout: usize,
    pub cancelled: usize,
    /// Statuses this build does not know about, kept rather than dropped.
    pub other: usize,
    /// Attempts that were the inherited-model retry.
    pub fallback_attempts: usize,
    pub tokens_generated: u64,
    /// Median / p90 / max wall clock over every attempt in the window.
    pub median_ms: u64,
    pub p90_ms: u64,
    pub max_ms: u64,
    pub by_agent: BTreeMap<String, AgentStats>,
}

/// `usable / attempts`, as a percentage. Zero attempts is 0, not 100.
pub fn success_rate(stats: &LedgerStats) -> f64 {
    if stats.attempts == 0 {
        return 0.0;
    }
    stats.usable as f64 * 100.0 / stats.attempts as f64
}

/// Aggregate the ledger over a window. `since_ms` is epoch millis; `None` is
/// everything retained.
pub fn stats(cwd: &Path, since_ms: Option<u64>) -> LedgerStats {
    let mut stats = LedgerStats::default();
    let mut durations: Vec<u64> = Vec::new();
    for attempt in read_all(cwd) {
        if since_ms.is_some_and(|since| attempt.ts < since) {
            continue;
        }
        stats.attempts += 1;
        durations.push(attempt.duration_ms);
        stats.tokens_generated += attempt.tokens_generated;
        if attempt.fallback_attempt {
            stats.fallback_attempts += 1;
        }
        match attempt.status.as_str() {
            "complete" => {
                stats.complete += 1;
                stats.usable += 1;
            }
            "partial" => {
                stats.partial += 1;
                stats.usable += 1;
            }
            "failed" => stats.failed += 1,
            "stalled" => stats.stalled += 1,
            "timeout" => stats.timeout += 1,
            "cancelled" => stats.cancelled += 1,
            _ => stats.other += 1,
        }
        let agent = stats
            .by_agent
            .entry(attempt.agent_type.clone())
            .or_default();
        agent.attempts += 1;
        agent.tokens_generated += attempt.tokens_generated;
        agent.wall_ms += attempt.duration_ms;
        if matches!(attempt.status.as_str(), "complete" | "partial") {
            agent.usable += 1;
        } else {
            agent.failed += 1;
        }
    }
    durations.sort_unstable();
    stats.median_ms = percentile(&durations, 50);
    stats.p90_ms = percentile(&durations, 90);
    stats.max_ms = durations.last().copied().unwrap_or(0);
    stats
}

/// Nearest-rank percentile over a sorted slice.
fn percentile(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (pct * sorted.len()).div_ceil(100).max(1) - 1;
    sorted[rank.min(sorted.len() - 1)]
}

/// One line per failure, for a report or a bug report. Newest first.
pub fn failure_lines(cwd: &Path, limit: usize) -> Vec<String> {
    let mut failures: Vec<DispatchAttempt> = read_all(cwd).into_iter().filter(|a| !a.ok).collect();
    failures.reverse();
    failures
        .into_iter()
        .take(limit)
        .map(|a| {
            format!(
                "{} {} {} {}ms {}",
                a.task_id,
                a.agent_type,
                if a.status.is_empty() {
                    "unknown"
                } else {
                    &a.status
                },
                a.duration_ms,
                a.error.as_deref().unwrap_or("(no cause recorded)")
            )
        })
        .collect()
}

/// Build an attempt from a settled result. Keeps the pool's record-keeping in
/// one place so the six settle paths cannot drift apart.
#[allow(clippy::too_many_arguments)]
pub fn attempt_from_result(
    task_id: &str,
    agent_type: &str,
    background: bool,
    depth: u8,
    requested_model: Option<&str>,
    resolved_model: Option<&str>,
    provider: Option<&str>,
    fallback_attempt: bool,
    status: &str,
    ok: bool,
    verified: bool,
    duration_ms: u64,
    tokens_generated: u64,
    peak_context: u64,
    exit_code: Option<i32>,
    budget_exceeded: Option<bool>,
    error: Option<&str>,
) -> DispatchAttempt {
    DispatchAttempt {
        ts: now_ms(),
        task_id: task_id.to_string(),
        agent_type: agent_type.to_string(),
        mode: if background { "background" } else { "blocking" }.into(),
        depth,
        requested_model: requested_model.map(str::to_string),
        resolved_model: resolved_model.map(str::to_string),
        provider: provider.map(str::to_string),
        fallback_attempt,
        status: status.to_string(),
        ok,
        verified,
        confidence: None,
        duration_ms,
        tokens_generated,
        peak_context,
        exit_code,
        budget_exceeded,
        error: error.and_then(clean_error),
    }
}

/// Epoch millis. `SystemTime` before the epoch is clamped to 0.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Attach the child's self-reported confidence from `result.json`.
pub fn with_confidence(
    mut attempt: DispatchAttempt,
    result_data: Option<&Value>,
) -> DispatchAttempt {
    attempt.confidence = result_data
        .and_then(|d| d.get("confidence"))
        .and_then(serde_json::Value::as_f64);
    attempt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_line() {
        let attempt = DispatchAttempt {
            ts: 1_700_000_000_000,
            task_id: "dispatch-1".into(),
            agent_type: "explore".into(),
            mode: "background".into(),
            status: "partial".into(),
            ok: true,
            confidence: Some(0.6),
            duration_ms: 44_000,
            ..Default::default()
        };
        assert_eq!(
            DispatchAttempt::from_line(&attempt.to_line()),
            Some(attempt)
        );
    }

    #[test]
    fn a_corrupt_line_yields_none_not_a_panic() {
        assert_eq!(DispatchAttempt::from_line("{not json"), None);
        assert_eq!(DispatchAttempt::from_line(""), None);
    }

    #[test]
    fn a_partial_line_from_a_killed_writer_is_skipped() {
        let dir = tempdir();
        std::fs::create_dir_all(ledger_path(dir.path()).parent().unwrap()).unwrap();
        std::fs::write(
            ledger_path(dir.path()),
            "{\"ts\":1,\"task_id\":\"a\"}\n{\"ts\":2,\"task_id\":\"b",
        )
        .unwrap();
        let all = read_all(dir.path());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].task_id, "a");
    }

    #[test]
    fn aggregates_the_window_and_the_agents() {
        let dir = tempdir();
        append(
            dir.path(),
            &attempt("a", "explore", "complete", true, 1_000),
        );
        append(
            dir.path(),
            &attempt("b", "explore", "stalled", false, 60_000),
        );
        append(
            dir.path(),
            &DispatchAttempt {
                ts: 5_000,
                fallback_attempt: true,
                ..attempt("c", "code-review", "partial", true, 90_000)
            },
        );
        let stats = stats(dir.path(), Some(1_000));
        assert_eq!(stats.attempts, 3);
        assert_eq!(stats.usable, 2);
        assert_eq!(stats.complete, 1);
        assert_eq!(stats.partial, 1);
        assert_eq!(stats.stalled, 1);
        assert_eq!(stats.fallback_attempts, 1);
        assert_eq!(stats.by_agent["explore"].attempts, 2);
        assert_eq!(stats.by_agent["explore"].usable, 1);
        assert_eq!(stats.by_agent["code-review"].usable, 1);
        assert_eq!(stats.median_ms, 60_000);
        assert_eq!(stats.max_ms, 90_000);
    }

    #[test]
    fn a_window_excludes_older_attempts() {
        let dir = tempdir();
        append(
            dir.path(),
            &DispatchAttempt {
                ts: 10,
                ..attempt("old", "explore", "failed", false, 1_000)
            },
        );
        append(
            dir.path(),
            &DispatchAttempt {
                ts: 10_000,
                ..attempt("new", "explore", "complete", true, 1_000)
            },
        );
        let all = stats(dir.path(), None);
        let window = stats(dir.path(), Some(5_000));
        assert_eq!(all.attempts, 2);
        assert_eq!(window.attempts, 1);
        assert_eq!(window.complete, 1);
    }

    #[test]
    fn an_empty_ledger_reports_zero_not_a_crash() {
        let dir = tempdir();
        let stats = stats(dir.path(), None);
        assert_eq!(stats.attempts, 0);
        assert_eq!(success_rate(&stats), 0.0);
        assert_eq!(stats.max_ms, 0);
        assert!(recent(dir.path(), 5).is_empty());
        assert!(failure_lines(dir.path(), 5).is_empty());
    }

    #[test]
    fn success_rate_is_usable_over_attempts() {
        let mut stats = LedgerStats {
            attempts: 4,
            usable: 3,
            ..Default::default()
        };
        assert!((success_rate(&stats) - 75.0).abs() < 0.001);
        stats.usable = 0;
        assert_eq!(success_rate(&stats), 0.0);
    }

    #[test]
    fn recent_returns_the_newest_in_order() {
        let dir = tempdir();
        for i in 0..10 {
            append(
                dir.path(),
                &DispatchAttempt {
                    ts: i,
                    ..attempt(&format!("t{i}"), "explore", "complete", true, 1)
                },
            );
        }
        let tail = recent(dir.path(), 3);
        assert_eq!(
            tail.iter().map(|a| a.task_id.as_str()).collect::<Vec<_>>(),
            vec!["t7", "t8", "t9"]
        );
    }

    #[test]
    fn failure_lines_are_newest_first_and_name_the_cause() {
        let dir = tempdir();
        append(
            dir.path(),
            &DispatchAttempt {
                error: Some("400 requires Global regions".into()),
                ..attempt("bad", "explore", "failed", false, 10)
            },
        );
        append(
            dir.path(),
            &attempt("good", "explore", "complete", true, 10),
        );
        let lines = failure_lines(dir.path(), 5);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("bad"));
        assert!(lines[0].contains("Global regions"));
    }

    #[test]
    fn an_error_is_collapsed_to_one_bounded_line() {
        let long = format!("line one\nline two{}", "x".repeat(MAX_ERROR_CHARS));
        let attempt = DispatchAttempt {
            error: clean_error(&long),
            ..Default::default()
        };
        let error = attempt.error.unwrap();
        assert!(!error.contains('\n'));
        assert!(error.chars().count() <= MAX_ERROR_CHARS);
    }

    #[test]
    fn concurrent_appends_do_not_interleave() {
        // A pool settles tasks on several threads at once. Before the record
        // was written with a single `write_all`, eight concurrent dispatches
        // produced one unparseable line out of eight.
        let dir = tempdir();
        let mut handles = Vec::new();
        for worker in 0..8 {
            let root = dir.path().to_path_buf();
            handles.push(hoocode_runtime::spawn_thread(
                "hoocode-ledger-test",
                move || {
                    for i in 0..50 {
                        append(
                            &root,
                            &DispatchAttempt {
                                ts: i,
                                task_id: format!("w{worker}-{i}"),
                                agent_type: "explore".into(),
                                status: "complete".into(),
                                ok: true,
                                ..Default::default()
                            },
                        );
                    }
                },
            ));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        let all = read_all(dir.path());
        assert_eq!(all.len(), 400);
        let raw = std::fs::read_to_string(ledger_path(dir.path())).unwrap();
        assert_eq!(raw.lines().count(), 400);
        assert!(raw
            .lines()
            .all(|line| DispatchAttempt::from_line(line).is_some()));
    }

    #[test]
    fn percentiles_are_nearest_rank_over_a_sorted_slice() {
        let values: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&values, 50), 50);
        assert_eq!(percentile(&values, 90), 90);
        assert_eq!(percentile(&values, 100), 100);
        assert_eq!(percentile(&[], 50), 0);
        assert_eq!(percentile(&[7], 90), 7);
    }

    #[test]
    fn prunes_to_the_keep_window() {
        let dir = tempdir();
        let path = ledger_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = String::new();
        for i in 0..(MAX_LINES + 10) {
            text.push_str(&format!("{{\"ts\":{i},\"task_id\":\"t{i}\"}}\n"));
        }
        std::fs::write(&path, &text).unwrap();
        append(
            dir.path(),
            &DispatchAttempt {
                ts: 99_999,
                ..attempt("newest", "explore", "complete", true, 1)
            },
        );
        let all = read_all(dir.path());
        assert_eq!(all.len(), KEEP_LINES + 1);
        assert_eq!(all.last().unwrap().task_id, "newest");
        assert!(all[0].ts >= 10);
    }

    #[test]
    fn mode_and_defaults_come_from_the_builder() {
        let attempt = attempt_from_result(
            "t",
            "plan",
            true,
            2,
            Some("fast"),
            None,
            None,
            false,
            "complete",
            true,
            true,
            5_000,
            100,
            20_000,
            Some(0),
            Some(false),
            Some("boom"),
        );
        assert_eq!(attempt.mode, "background");
        assert_eq!(attempt.depth, 2);
        assert_eq!(attempt.requested_model.as_deref(), Some("fast"));
        assert_eq!(attempt.error.as_deref(), Some("boom"));
        let blocking = attempt_from_result(
            "t",
            "plan",
            false,
            1,
            None,
            Some("m"),
            Some("p"),
            true,
            "failed",
            false,
            false,
            1,
            0,
            0,
            None,
            None,
            None,
        );
        assert_eq!(blocking.mode, "blocking");
        assert!(blocking.error.is_none());
    }

    #[test]
    fn confidence_is_read_out_of_result_data() {
        let data = serde_json::json!({"summary": "s", "confidence": 0.9});
        let attempt = with_confidence(DispatchAttempt::default(), Some(&data));
        assert_eq!(attempt.confidence, Some(0.9));
        let none = with_confidence(DispatchAttempt::default(), None);
        assert_eq!(none.confidence, None);
        let bad = with_confidence(DispatchAttempt::default(), Some(&serde_json::json!({})));
        assert_eq!(bad.confidence, None);
    }

    fn attempt(id: &str, agent: &str, status: &str, ok: bool, duration: u64) -> DispatchAttempt {
        DispatchAttempt {
            ts: 1_000,
            task_id: id.into(),
            agent_type: agent.into(),
            status: status.into(),
            ok,
            verified: ok,
            duration_ms: duration,
            ..Default::default()
        }
    }

    /// A unique temp dir per test: the ledger is a fixed file inside it.
    fn tempdir() -> TempDir {
        TempDir(std::env::temp_dir().join(format!(
            "subagent-ledger-{}-{}",
            std::process::id(),
            next_id()
        )))
    }

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    fn next_id() -> u64 {
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
