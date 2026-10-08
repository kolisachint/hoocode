# Subagent evals: measuring whether a dispatch works

Status: **as-built, 2026-10-05.** Companion to `subagents.md`, which is the
incident record; this file is the instrument. Not a migration-ledger task.

## 1. Why

Subagents failed on ten dispatches out of ten in early October 2026, and the
fixes that followed (`subagents.md` §2) were verified by hand: two recorded
end-to-end runs and one full `Task` dispatch. That is a good day; it is not a
measurement. There was no way to ask:

- what fraction of dispatches settle `complete` or `partial`?
- which agent type is slow, and how slow?
- does a fault (region rejection, a stream that dies, a bad tool call, a hang)
  end the run cleanly, or lose the work?
- did a change make things better or worse?

The only evidence was whatever dispatch dirs survived on disk — and a clean
success deletes its own dir, so successes were invisible and failures were
swept after 24h. Two things ship together here:

1. **The dispatch ledger** (`crates/hoocode-code-subagents/src/ledger.rs`):
   one append-only line per **attempt**, in
   `<cwd>/.hoocode/dispatch/ledger.jsonl`. Telemetry, never a source of
   truth: `result.json` decides correctness, the ledger only records what
   happened. A ledger that cannot be written is logged and dropped.
2. **The eval harness** (`scripts/eval/`): scenarios that drive the real
   binary — real parent session, real `Task` tool, real pool, real child
   process — against a scripted mock provider.

## 2. Running it

```bash
cargo build -p hoocode-code-main          # the harness runs target/debug/hoocode
scripts/eval/subagent_evals.py               # the fast suite, ~25s
scripts/eval/subagent_evals.py --include-slow  # + the 100s stall probe and the 50s wrap-up
scripts/eval/subagent_evals.py --only parent_region_error_falls_back
scripts/eval/subagent_evals.py --repeat 3    # three runs each: flakiness, not fate
scripts/eval/subagent_evals.py --min-success 0.9   # gate a build on the usable rate
```

Exit code is 0 when every scenario met its declared expectation and the usable
rate is at or above `--min-success`. Reports land in
`target/subagent-evals/` (`report.md`, `report.json`, and a per-run directory
with both transcripts and the request log).

A scenario may declare a `known_issue`: today's behaviour is recorded, the
suite does **not** fail on it, and a change in either direction is reported. A
known issue that starts passing is a signal to delete the note, not to celebrate.

`--include-slow` scenarios are excluded by default because two of them cost
minutes of wall clock by design.

## 3. The mock provider

`scripts/eval/mock_provider.py` is a routable OpenAI-compatible mock.
`tui-parity/mockllm.py` serves one global, strictly ordered script, which is
right for a parity transcript and wrong here: a dispatch interleaves the parent's
and the child's requests, and the order in which two processes happen to call a
model is not something a test should depend on. This one routes on the request —
the last user message of a child always ends in `Task: …` — and gives each route
its own script and cursor.

Turn keys: `text`, `reasoning`, `tool_calls`, `usage`, `delay_s`, `hang` (never
answer), `abort_after` (send N bytes of SSE, then drop the connection) and
`error` (`{"status": …, "message": …}`). An exhausted route answers
`500 mock: script exhausted`, so a runaway run fails loudly instead of hanging.

## 4. Baseline, 2026-10-05

Twelve scenarios, 175s with `--include-slow`, against `target/debug/hoocode` at
v0.1.7 plus the ledger. **11 as declared, 1 known issue, 12 attempts recorded,
11 usable.**

| scenario | what it pins down | result | s |
| --- | --- | --- | --- |
| `child_direct_answer` | the floor: one turn, verifiable result | complete, confidence 0.9 | 0.5 |
| `child_tool_call_then_answer` | the tool loop resumes and settles complete | complete | 0.6 |
| `child_turn_limit_settles_partial` | turn limit yields a usable partial | partial, 0.6 | 0.6 |
| `child_invalid_tool_arguments` | a malformed tool call does not sink the run | complete | 0.6 |
| `child_stream_abort_mid_answer` | the stream dies mid-answer | **complete on partial text** | 0.6 |
| `child_deadline_wrapup_partial` | the 2026-10-03 recovery: wrap up before the kill | partial, real summary | 49 |
| `parent_dispatch_blocking` | the ordinary dispatch | complete, 24ms | 0.6 |
| `parent_dispatch_background` | notification then result | complete, `mode=background` | 0.6 |
| `parent_region_error_falls_back` | region rejection retries on the parent's model | failed → complete | 0.7 |
| `parent_unknown_complexity_tier` | a bad tier never reaches a child | rejected by the parent, no dispatch | 0.6 |
| `parent_queue_saturation` | 8 dispatches, 5 slots | 8 attempts, 8 usable, none stuck | 20 |
| `parent_stall_reaped` | a hung child is reaped in ~1 min | **KNOWN: it is not** | 100 |

## 5. What the baseline found

**A hang costs ten minutes, not one.** `parent_stall_reaped` expects the stall
watchdog to reap a child that never answers. It does not: at 30s, 70s and 95s
the child was still running and the ledger was still empty. The child writes its
heartbeat on a timer, and the pool treats *any* stdout byte as liveness, so a
child blocked inside a provider call looks busy forever. Only the hard
per-agent deadline ends it — ten minutes for `explore`, twenty for
`general-purpose`. This is the same class of failure as the October incident
(deadline table keyed on agents nobody ships), wearing a different hat: a
liveness signal that a hung process can keep sending. Owner: open.

**A truncated stream still settles `complete`.** The runtime finalises whatever
text arrived, so a provider that dies mid-sentence produces a `complete` result
built from half an answer. Defensible for a summary, wrong for anything the
parent treats as a finding. The eval records the behaviour as the baseline so a
change is visible.

**Concurrent appends interleaved.** The first full run of
`parent_queue_saturation` recorded seven attempts and one unparseable line: the
ledger wrote the record and its newline as two `write_all` calls from several
settling threads. Now one write per record, with a 400-line concurrency test.
This is exactly the bug the suite existed to find, on the code the suite needed.

**The unknown-`complexity` gap is smaller than it looked.** A bad tier is
rejected by the `Task` tool's schema in the parent, so no child ever spawns. The
pool still does not validate a tier for callers that skip the schema
(`/subagent`, and anything reaching `DispatchOptions` directly).

## 6. Reading the ledger directly

`/subagent-stats [24h|7d|all]` renders the aggregate in the TUI: attempts,
usable rate, statuses, median/p90/max wall clock, tokens, fallback count, a
per-agent breakdown and the five most recent failures with their causes. In
scripts:

```rust
let stats = hoocode_code_subagents::ledger::stats(&cwd, None);
let rate = hoocode_code_subagents::ledger::success_rate(&stats);
```

The ledger is bounded: past 20 000 lines it is rewritten to keep the newest
10 000. `docs/design/subagents.md` §7 warned that the raw October evidence is
swept after 24h and mostly already gone. These lines are the replacement, and
unlike the dispatch dirs they record the successes.