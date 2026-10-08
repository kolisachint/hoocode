# Subagents: the process model, and what was fixed

Status: **as-built, 2026-10-03.** The original design in this file (in-process sessions,
delta token accounting, enforced budgets, a provider-error classifier enum) was reviewed
against the code and the recorded failures in October 2026 and **rejected in three places**.
What shipped is the child-process model with seven targeted fixes. The in-process migration
is still open; see §6.

Not a migration-ledger task: the migration is paused and this was taken out of it.

## 1. The problem, and what the evidence actually said

Subagents were failing on every recorded run. In `hoobot/.hoocode/dispatch/`, a clean
success deletes its dispatch dir (`pool.rs`, clean-success branch), so every dir left on disk
is a failure. **Ten dirs, zero successes.**

| Outcome | Count | What the parent was told |
|---|---|---|
| `timeout` | 4 | `task_timeout`, no partial work |
| `cancelled` | 3 | `subagent cancelled` |
| `stalled` | 1 | a 10 KB stdout dump |
| `failed` | 1 | `Provider finish_reason: error` |
| *(no `output.json`)* | 1 | nothing |

Six causes were written up. Three of the six were wrong on inspection, and two of the three
surviving causes were mis-diagnosed. The corrections are the most useful part of this
record, so they come first.

### Corrections to the original analysis

**The load multiplier does not make subagents *more* likely to be killed — the opposite.**
`lifeguard.rs` re-arms the timer rather than firing when `mult > 1.0`, and scales the stall
threshold by it too. More concurrency means a genuinely stuck task is reaped *later*, up to a
4x ceiling. And it never engaged in any recorded failure: all four timeouts fired at exactly
300s = 5 min x 1.0 multiplier, because concurrency was 1.

**`output.json` was not an unbounded transcript.** Capture is a 256 KB tail
(`MAX_CAPTURED_STREAM_CHARS`, `append_tail`). The recorded 267-275 KB files were a *saturated*
buffer — the head was already discarded. "Redundant" was right; "grows without bound" was not.

**The timeout table matched no shipped agent, and that was the proximate cause of the
recurring timeouts.** `base_timeout_ms` keyed on `edit | test | review`; we ship
`code-review`, `explore`, `general-purpose`, `plan`, `security-review`. **No arm matched**,
so every agent silently got the 5-minute default. hoocode has the same dead table against the
same agent names — this is a faithful port of an upstream bug, and it is why three
`code-review` runs died holding 220-297s of finished work.

**The region errors are not in the artifacts.** `400 Upstream request failed: This Go model
requires Global regions` appears in **zero** of the ten dispatch dirs. It may well have been
seen live in the parent session, but the evidence base does not carry it, and the three most
recent failures ran on `anthropic/claude-opus-5-5` and still timed out at 300s.

**`capable` is not the priciest model.** The configured `defaultProvider`/`defaultModel` wins;
priciest is only the fallback. `fast` is the cheapest *among models priced at or below*
`capable`. The original write-up simplified this wrongly; the code comment beside it was right.

**`dispatch.rs` was never the stdout framing.** It is `dispatch-evaluator.ts`: the depth guard
and complexity heuristic. Framing and `SubagentStdoutLine` live in **`events.rs`**.

### Two causes that survived, and are worse than described

**Post-hoc salvage does not work.** The obvious fix — reconstruct a result from `session.jsonl`
when a run is killed — produces garbage. `derive_summary` walks back for the last assistant
text block, and in the recorded runs:

| runs | assistant text messages | what salvage would return |
|---|---|---|
| 5 of 10 | **zero** | *"Task completed with no textual summary."* |
| 4 of 10 | 1 | a progress note (*"I'll start by exploring…"*) |
| 1 of 10 | 3 | a real summary — but it died anyway |

The one run that had a real summary (`dispatch-1790866463943-0ozuar`) still failed, because
`result.json` is written only after `prompt()` returns cleanly. **Salvage has to be
pre-armed: the child must be told to wrap up before the kill, not reconstructed after it.**

**Delta token accounting would not have fixed the budget.** `usage.totalTokens` is the
*context size* of the turn, so summing it is quadratic — the recorded run reported 1 654 795
against a 35 000 budget when its real generated total is 4 111. The obvious repair is to
difference it. Measured across all eight runs with a transcript, the delta still crosses 35k
at **turn 1 or 2 in 7 of 8**, because turn-1 context alone is 25k-62k (system prompt + tool
schemas) and no subagent can shrink it. Delta accounting plus an enforced budget would have
hard-stopped nearly every subagent on its first turn. The metric was the problem, not the
arithmetic: `output` tokens run 1.2k-22k, and they are the only figure that tracks work done.

## 2. What shipped

Seven fixes to the child-process model. The pool's public API is unchanged; every consumer
(Task tool, TaskOutput, inbox, TUI task panel, `/subagent`, the warm pool) ports untouched.

| | Fix | Where |
|---|---|---|
| T1 | Timeout table keyed on agents we do not ship, so **every** agent got the 5-minute default. Real arms added; the warm pool's flat 180s now tracks the same table | `lifeguard.rs`, `warm.rs` |
| T2 | `used` summed cumulative `totalTokens` (quadratic). Now sums `output + cacheWrite`; context reported separately as `peak_context` | `token_budget.rs` |
| T3 | Children are killed at their deadline with `result.json` never written. The parent now passes `--deadline-ms`; the child wraps up 90s before it and settles `partial` | `pool.rs`, `runtime.rs`, `result.rs`, `args.rs` |
| T4 | Killed tasks returned before the fallback ladder was consulted, so a run that died because its model was unreachable never retried. They consult it now — except a user cancellation, which is never retried | `pool.rs` |
| T5 | The classifier matched neither `requires Global regions` nor `finish_reason: error`. Both added | `pool.rs` |
| T6 | With `complexity` passed, the fallback re-resolved the same category and retried the identical model. A new `inherited_model` field carries the parent's concrete model | `pool.rs`, `tools.rs` |
| T7 | `output.json` embedded a 256 KB stdout tail. Now outcome + cause + an 8 KB stderr tail | `pool.rs` |

### T3 in detail — the only structural change

`--deadline-ms` is an internal flag (not in `--help`, like `--task-id`). The pool sends the
**base** per-agent deadline, not the load-scaled budget: under load the parent widens its
kill, so the wrap-up always lands first. Ninety seconds of lead covers the one or two turns a
model needs to turn findings into a summary — recorded subagent turns took 5-65s.

The pool side needed no change. `settle` already treats a valid `result.json` as success even
when the task was killed (`kill_reason.filter(|_| !verification.valid)`), so all the child has
to do is produce one. A deadline wrap-up settles `partial` with confidence 0.6, which clears
the verifier's 0.5 floor.

## 3. Deviations from hoocode

Every one is commented in the source with the evidence.

- **Timeout table** — hoocode's arms are kept for plugin agents that use those names, plus
  real arms for the shipped five. (`lifeguard.rs`)
- **Warm run timeout** — hoocode's `WARM_RUN_TIMEOUT_MS` is a flat 180s for every agent; we
  use the same per-agent table, never tighter. (`warm.rs`)
- **Token accounting** — `used` counts generated tokens; `peak_context` carries the context
  size. (`token_budget.rs`)
- **`--deadline-ms`** and the wrap-up steer. (`runtime.rs`)
- **Fallback classification** — hoocode's pattern plus region and unexplained-provider-failure
  groups. Kept as one pattern rather than a classifier enum; the alternatives are grouped so
  they can be lifted out later. (`pool.rs`)
- **`inherited_model`** — hoocode falls back to `task.model`, which is a `complexity` category
  when the caller passed one. (`pool.rs`, `tools.rs`)
- **`output.json`** — no embedded `stdout`. (`pool.rs`)

One candidate was **rejected during review**: setting a prose `error` on killed tasks. It
changes a user-visible TUI line from hoocode's `subagent stalled`, and no parity scenario
covers a killed task, so it would have shipped an uncovered divergence inside a correctness
pass. `error` stays unset.

## 4. Verification

- `cargo nextest run --workspace` — **3494 passed**, 0 failed (was 3479; +15 tests).
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --check`, `migration/check_dep_firewall.py` — clean.
- **L2 parity: `subagent-task`, `subagent-command`, `json-subagent-child` all pass** against
  real hoocode.

**End to end**, against the release binary and the harness's mock LLM in an isolated `HOME`:

| | duration | requests | status | confidence | steers |
|---|---|---|---|---|---|
| `--deadline-ms 95000` | 44s | 25 | **partial** | 0.6 | 1 |
| control, no flag | 40s | 25 | **complete** | 0.9 | 0 |

Both recovered the same real summary from a run that otherwise would not have. The steer is
delivered exactly once and then persists as ordinary conversation history — verified as one
copy per request body, not re-injection.

A full `Task` tool dispatch — parent, pool, real child process, verified `result.json` — settled
`complete` and the parent received the summary. **The first clean subagent success in the ten
recorded runs.**

### Pre-existing L2 failures, not ours

`harness.py run all` is not fully green. Attributed with a clean `git worktree` at HEAD:

| scenario | ours | clean HEAD | cause |
|---|---|---|---|
| 9 x `print-*` | fail | **fail** | hoocode 0.6.0 ships a `SearchHooCode` tool the port does not implement |
| `config-selector` | fail | **fail** | trailing slash in a config resource path |
| `cd-reload`, `export-import` | invalid | **invalid** | *hoocode itself* fails the scenario |
| 3 x subagent | pass | **pass** | — |

## 5. Decisions taken

- **The budget stays advisory.** T2 made the number honest; nothing enforces it. With `output`
  at 1.2k-22k against a 35k limit, enforcement would essentially never fire. Wall-clock is the
  primary stop, `--max-turns 50` the secondary one, the budget telemetry. Owner, 2026-10-03.
- **The new deadlines stand as written** — `explore`/`plan` 10 min, `code-review`/
  `security-review` 15 min, `general-purpose` 20 min — from four data points. Not measured
  against a task corpus. Owner, 2026-10-03.
- **Track 2 (in-process) not started.** Owner, 2026-10-03.

## 5b. P1 hardening (2026-10-05)

The eval suite (`subagent-evals.md`) made the remaining holes measurable, and
this is what closed them. Every item is a Rust test in `tests/it/hardening.rs`
or `tests/it/lifeguard.rs`, plus one end-to-end scenario each.

| | Fix | Where |
|---|---|---|
| H1 | **Liveness is two-tier.** Silence past 60s reaps; so does no *forward progress* (a `turn_end` or a tool event) past 150s, load-scaled. A child parked in a provider call keeps pinging and used to be invisible until its hard deadline — ten minutes for `explore`, measured alive at 95s | `lifeguard.rs` |
| H2 | **A reap is SIGTERM, grace, SIGKILL.** The child catches SIGTERM, writes its partial `result.json` and exits, and the pool already accepts a valid result from a reaped task, so a stall kill returns work instead of discarding it. The grace escalates for a child too wedged to catch a signal | `lifeguard.rs`, `runtime.rs` |
| H3 | **`pid` is written.** `sweep_old_agents` has always read `dispatch/<task>/pid` to ask whether a run is alive; nothing ever wrote it, so reaping was age-only and a real orphan was never detected | `pool.rs` |
| H4 | **Admission control.** The queue was unbounded: a confused parent could enqueue hundreds of runs that never finish. It is refused at four per slot, with a message that says so | `pool.rs` |
| H5 | **Ageing instead of starvation.** Priority gained a step per minute waited, so a `code-review` cannot sit behind a stream of `explore` runs forever | `pool.rs` |
| H6 | **Finished results are bounded** (64, oldest dropped). Each carries its captured stdout and stderr and nothing released them: unbounded growth over a long session | `pool.rs` |
| H7 | **Inbox reconciliation.** A `running` record the pool has forgotten (a lost settle event) used to report `running` forever, in the roster and the panel. `TaskOutput` now settles them, with an age guard so a dispatch that has not started yet is never mistaken for a lost one | `inbox.rs`, `tools.rs` |
| H8 | **The warm pool shares the caps.** It booted one worker per dispatch with no ceiling and told the lifeguard nothing. It now has the cold pool's in-flight and admission limits | `warm.rs` |
| H9 | **Writes are not swallowed.** `write_file_atomic(result.json)` was `let _ =`: a full disk gave a clean child exit with no result and a parent told only that it failed. It returns, logs, and exits non-zero | `result.rs`, `runtime.rs` |
| H10 | **Settings deep-merge.** `instance.rs` used `global.extend(project)`, which replaced the whole `modelCategories` object — a project setting one tier silently lost the other two | `instance.rs` |
| H11 | **`complexity` is validated at the tool.** An unrecognised tier used to pass through as a model id and kill the child at startup; it now falls back to the parent's model with one log line | `tools.rs` |
| H12 | **The ledger reports what the child reported.** A run SIGTERMed and wrapped up settles `complete` in the pool (the file is valid) while `result.json` says `partial`. The ledger now keeps `partial` visible | `pool.rs` |

## 5c. P2: the runner seam (2026-10-05)

`crates/hoocode-code-subagents/src/runner.rs`. Until this existed, "run a
subagent" and "spawn this executable with these argv" were the same statement:
the pool built a `std::process::Command` and read its pipes. That made three
things untestable — anything needing a *model* in it, anything where two runs
must interleave deterministically, and the in-process model.

A `Runner` takes a `RunSpec` (argv, env, prompt, tools, model, deadline — the
same facts both as a command line and as fields) and returns a `RunnerHandle`
with the same shape as a child process: two byte streams, a wait, a kill, a pid.
Everything above the seam — queueing, priority, ageing, admission control, the
lifeguard, heartbeats, token accounting, verification, the ledger — is unchanged
and now runs against either implementation.

The seam kept *streaming* deliberately: liveness is a per-byte signal, so a
handle that returned its output at the end would quietly disable the stall
watchdog.

Two implementations ship:

- `ProcessRunner` re-execs the binary. This is what the product uses.
- `ScriptedRunner` answers from a script with no process at all. `tests/it/runner.rs`
  drives the whole pool through it: settle paths, ledger lines, priority, queue
  order, stderr capture, the watchdog. If any of those needed a `/bin/sh` mock
  again, the seam would have leaked.

**The seam found a latent kill bug on its first run.** `terminate_group(0)` is
`kill(-0, SIGTERM)`, which signals *every process in the caller's group* — the
parent agent included. Nothing could reach it before, because a dispatched run
always had a pid; a processless runner can. `kill_tree` already guarded `pid > 0`;
`terminate_group` now does too.

Alongside it:

- **Fault catalogue** in the eval harness: 429 with `retry-after`, a stream that
  dies mid-line, a torn JSONL line, a context overflow, a provider error.
- **`--matrix`**: every scenario against every fault. Current baseline: 65/65
  cells settle, 95% usable.
- **`--soak N`**: rounds of randomised faults asserting nothing leaks and
  nothing stays unsettled. A 60s run is 26 rounds, 0 leaks.
- **Flake fixes and retries**: `.config/nextest.toml` retries the subagent suite
  three times (and has a `ci` profile that does not); the two inbox `wait:true`
  tests wait on the finish instead of a 5ms sleep, and the warm-pool TTL test
  polls instead of sleeping four times the TTL.

## 5d. P3: one word for the thing (2026-10-05)

`Task` read as a to-do item — the task store really does have a `Task` type for
TodoWrite entries and MCP calls — while the tool starts a background run, and
the transcript said a third thing entirely (`Agent [explore]`).

| before | after | still accepted |
|---|---|---|
| `Task` | **`Agent`** | `Task`, registered as an alias for a release |
| `TaskOutput` | **`AgentOut`** | `TaskOutput`, same |
| `Agent [explore]`, `TaskOutput explore#1` | **`Agent explore`**, **`AgentOut explore#1`** (plus `resume`, plus `· background`) | both names render through one renderer |
| `call Task with resume_task_id` | `call Agent with …` | — |

First shipped on this branch as `Dispatch` / `DispatchStatus`; renamed to
`Agent` / `AgentOut` before release (2026-10-06) because they are shorter, read
the same in every surface (TUI, transcript, a hoobot chat line), and the
transcript already said `Agent`. `Dispatch` never shipped, so it is not an
alias.

An alias is a spelling, not a second code path: both definitions execute the
same function, and the prompt's tool list says which one is canonical
(`deprecated alias for Agent; prefer Agent`) so a model is never offered
two identical tools.

The rename touched the tool definitions, the nested-agent allowlist (which never
contained `Task` at all, so an agent that listed `tools:` silently lost it), the
allowlist normalisation, the render dispatch, the pool's and warm pool's
`--tools` grant, the availability prompt, the transcript renderer and the
prompt templates.

**Divergence is now declared, not silent.** Two pin checks compared our bytes to
hoocode's and would have failed on the rename:

- `shipped_prose_ts.rs` normalises our template back to the pinned wording using
  an explicit `DECLARED_DIVERGENCES` list, so an undeclared difference still
  fails and a declared one that rots (the pinned text changes under us) fails
  too.
- `tool_renderers_gold.rs` skips the renamed tools' pinned renderings and
  asserts ours instead, with the count asserted so the skip cannot silently
  become "compare nothing".

## 6. The in-process migration: built, measured, not shipped (2026-10-05)

§6 of this file used to be an open question. It is now an answer, and the
answer is "the mechanism works, the product does not use it yet".

**What shipped.** `runner.rs` has three implementations of one seam:

| | what it is | used by |
|---|---|---|
| `ProcessRunner` | re-execs the binary per dispatch | the product, unchanged |
| `ScriptedRunner` | answers from a script, no process | the whole test suite |
| `InProcessRunner` | runs the agent loop in this process | off; wired but not enabled |

**The gate, which was the whole question, passes.** `an_in_process_child_can_dispatch_its_own_child`
runs a child in process which dispatches its own child through the *same* pool
while the first run is still going: no deadlock, no double settle, two ledger
lines, nothing stuck. The reason it works is small and worth writing down: the
pool never holds its state lock across an await, so a run the pool owns can ask
the same pool for another run. Object 4 of the old objection list — "panic
containment has no precedent" — is also answered by construction for the seam,
though a panicking *agent* still unwinds the whole process, which is the one
argument for keeping children.

**The measurement** (`in_process_and_child_process_runs_are_comparable`, 12
identical runs through the same pool, same settle path, same ledger):

```
in-process 8.9ms      child process 459.9ms
```

~0.7ms of dispatch overhead against ~38ms, for runs that do nothing. On a real
subagent the model dominates and the ratio shrinks — but the per-dispatch floor
is gone, and the floor is what makes many small subagents viable.

**Why it is not the default anyway.**

1. The session wiring lives in the CLI (`runtime.rs`), not in this crate. The
   `InProcessFactory` deliberately takes the wiring from the caller; nobody has
   written the production factory, and a second way to build a session would be
   a second way to get auth, the registry and the system prompt wrong.
2. Depth is still a process-global env var (`SUBAGENT_DEPTH`). With children,
   depth is per process and therefore trivially correct. In process it is a
   shared mutable global that a nested run has to mutate and restore — and the
   gate test only proves the *pool* nests cleanly, not that depth accounting
   does.
3. A child's crash today costs one dispatch. In process it costs the session.

**Verdict.** Keep child processes as the shipped model. The seam is the part
that was worth building: it removed the 31 `/bin/sh` path seams, it is what the
fault matrix and the soak run against, and it means the question above can be
revisited with a factory and a per-run depth field rather than a rewrite. If
someone picks it up: item 2 first, and re-run
`an_in_process_child_can_dispatch_its_own_child` with real depth assertions
rather than a pool-liveness one.

## 7. Footnote: the evidence is not permanent

`sweep_old_agents` deletes dispatch dirs older than 24h when the pool starts — hoocode-faithful.
Five of the ten recorded runs have already been swept. The figures this document cites are
baked into the tests as literals (`RECORDED_TURNS` holds the exact 15 context/output pairs;
the timeout, agent-type and status tables are in assertions), so nothing needed is lost. Copy
the survivors in as fixtures if the raw evidence is worth keeping.