# Subagents

Status (2026-11-06): built. Each subagent is a child process. Runs are background by default. Writers work in git worktrees and commit but never push. Open items are SA-1 to SA-11.

Doc map: [README.md](README.md) · [roadmap.md](roadmap.md) · [runtime.md](runtime.md) (caps, lanes, reliability) · [agent.md](agent.md) (loop, tools, models) · [tui.md](tui.md) (task panel, `/agents`).

## Principles

| Rule | Detail |
|---|---|
| Child process per dispatch | A child's crash costs one dispatch. In-process is not the default (SA-8) |
| `result.json` decides | The ledger is telemetry only |
| Background by default | Unless the call or the agent says `background: false` |
| Parent keeps working | Dispatch all independent tasks in one message. `AgentOutput(wait: true)` only for one named result the next step needs |
| Brief every child | The child sees only its `prompt`. Brief format in How it works |
| Worktrees only for writers | Writers: general-purpose, or a custom agent with Edit, Write or Shell. Read-only agents (explore, plan) use the parent tree |
| Isolation by instruction | The harness never enforces worktree isolation |
| Commit, never push | A child commits on its own branch. The user merges |
| Cleanup rule | At session end, remove only what `git branch --merged <base>` lists. Keep and report the rest |
| Every agent gets AGENTS.md and CLAUDE.md | Read-only agents too |
| Advisory budget | Never stops a run. Counts output tokens only |
| Liveness is progress | Stdout bytes are a signal. Turn end or tool event is progress |
| One deadline for every type | No per-type timeout table |

Worst case at defaults (depth-4 tree): 12 + 12·6 + 12·6² + 12·6³ = **3108** live children.

## How it works

### Limits

Resolution: frontmatter (where the agent has the key), then `HOOCODE_*` env, then the settings key, then the default. Zero, non-numeric or out-of-range values fall through. Env values are inherited by children.

| Limit | Default | Settings key | Env var | On hit |
|---|---|---|---|---|
| Turns per run | 200 | `subagentMaxTurns` | `HOOCODE_SUBAGENT_MAX_TURNS` (frontmatter `maxTurns`) | Asked to wrap up at 90%; stopped at the cap |
| Token budget (advisory) | 150k; general-purpose 300k | `subagentTokenBudget` | `HOOCODE_SUBAGENT_TOKEN_BUDGET` (frontmatter `tokenBudget`) | `budget_warning` 80%, `budget_exceeded` 100%. Never stops |
| Deadline | 4 h base; load-scaled to 4 h 30 m | `subagentDeadlineMinutes` | `HOOCODE_SUBAGENT_DEADLINE_MS` | Child wraps up 90 s before the base; parent kills at the scaled deadline |
| Progress stall | 20 min, load-scaled | `subagentStallSeconds` | `HOOCODE_SUBAGENT_STALL_MS` | SIGUSR1, 30 s grace, SIGKILL |
| Tool in flight | 30 min × load; a Shell call gets max(this, its timeout + 60 s) | `subagentToolStallSeconds` | `HOOCODE_SUBAGENT_TOOL_STALL_MS` | As progress stall. `timeout: 0` = unbounded |
| Child RSS | 4096 MiB | `subagentMaxRssMb` | `HOOCODE_SUBAGENT_MAX_RSS_MB` | Child reaped |
| Subagent Shell default | 1200 s | `subagentShellTimeoutSeconds` | `HOOCODE_BASH_DEFAULT_TIMEOUT_SECS` | Command group killed; partial output returned. Main session has none |
| Root concurrency | 12 (1–32) | `subagentConcurrency` | `HOOCODE_SUBAGENT_CONCURRENCY` | Extra dispatches queue |
| Nested concurrency (depth ≥ 1) | 6 | `nestedSubagentConcurrency` | `HOOCODE_NESTED_SUBAGENT_CONCURRENCY` | Queue, per pool |
| Queue | 8 per running slot | `subagentQueuePerSlot` | `HOOCODE_SUBAGENT_QUEUE_PER_SLOT` | Refused beyond running + queued |
| Nesting depth | 4 (absolute max 8) | `maxSubagentDepth` | `HOOCODE_SUBAGENT_MAX_DEPTH`, `--max-subagent-depth` | No Agent tool at the cap. Refused with `Maximum subagent depth (N) reached` |
| `AgentOutput` wait | 300 s | `subagentWaitTimeoutSeconds` | `HOOCODE_SUBAGENT_WAIT_TIMEOUT_MS` | Wait returns; run continues |
| Provider retries | 5 | `retry.maxRetries` | none | Error reported |
| Output tokens per request | 64000 | `maxOutputTokens` | `HOOCODE_MAX_OUTPUT_TOKENS` | Anthropic only (SA-11) |

Fixed in code: 64 finished results kept; 50 settled inbox records; ledger trimmed past 20 000 lines to 10 000; dispatch dirs keep the newest 50 and drop those older than 24 h (never running or queued).

Warm workers (`warmSubagents`, opt-in) share the concurrency, queue and Shell limits. Warm and cold keep separate counters.

Child-only flags, not in `--help`: `--task-id`, `--deadline-ms`.

### Agent definition

Files: `crates/hoocode-code-resources/templates/agents/*.md`. Parser: `crates/hoocode-code-resources/src/agent_frontmatter.rs`. Shipped: `explore`, `plan`, `general-purpose`, `code-review`, `security-review`. All five ship `background: true`.

| Field | Meaning |
|---|---|
| `name` | Agent id, used as `subagent_type` |
| `description` | Use-when text for the parent |
| `tools` | Allowlist. Agent is still offered when delegation is allowed |
| `model` | Default model. A call's `model` overrides |
| `background` | `false` = foreground default, kept as `Some(false)` |
| `delegate` | `true` = may dispatch |
| `fork` | Same flag helper as `background` |
| `maxTurns`, `tokenBudget` | Per-agent limits (above) |

Rules:

- A writer has Edit, Write or Shell. Writers get the worktree instruction. Read-only agents do not.
- Every body ends with the final-answer rule. Custom bodies get `BASE_GUIDELINES` appended only at depth > 0. `--system-prompt` and `--light` still replace.
- Child prompt is one wrapper, `Task: <prompt>` (`pool::child_prompt`). Run description is the call's first line, at most 60 characters, `"` replaced by `'`.
- Code review and security review: Shell only for `git diff`, `git log`, `git show` (text rule; SA-3).

**Final-answer rule (every body):**

> Your final message is the only thing the caller receives. Start with the status (complete, partial or failed). Then the result, with the Report fields from the brief. Do not say "see above".

### Brief

```text
Goal: <outcome, one sentence>
Context: <absolute file paths, findings so far, why it matters>
Workspace: <absolute path> · branch <name or none> · base <commit or none>
Constraints: <read-only, or may edit under <path>> · may commit: yes/no · never push
Done when: <a check you can run>
Report: status (complete | partial | failed) · summary · files changed · commits · verified (what you ran, and the result) · not verified · open questions
```

- Absolute paths only. The Agent schema has no cwd or workspace field.
- `git -C <path>` for git. A Shell `cd` is not assumed to carry.
- The parent fills Workspace. The harness never does.

### Lifecycle, liveness, deadlines

| Stage | Rule |
|---|---|
| Admission | Refused beyond running + queued. Queue priority ages one step per minute waited |
| Start | The parent's call gets a placeholder that names the run: `explore#2 ("…") started in the background` |
| Heartbeat | Stdout silent past 60 s is one signal. Silence alone is not progress |
| Stall | No progress (turn end or tool event) for 20 min, load-scaled. Also a tool in flight past its bound |
| Reap | SIGUSR1 to the child pid only. The child writes a partial `result.json` and exits. 30 s grace, then SIGKILL. A valid partial result is accepted |
| Wrap-up | The child gets `--deadline-ms` = base 4 h and wraps up 90 s before it. Settles `partial`, confidence 0.6 |
| Deadline kill | Parent SIGTERMs the child group at the scaled deadline (≤ 4 h 30 m). A valid result is still accepted |
| Settle | Valid `result.json` = success. Killed with a valid result = `partial`, never `complete`. Killed without one = `failed` or `cancelled`, with cause and an 8 KB stderr tail in `output.json` |
| Fallback | Region error or `finish_reason: error` retries on the parent's concrete model (`inherited_model`). Never a user cancel |
| Write error | `result.json` written atomically. A failed write is logged and the child exits non-zero |
| Group sweep | After a reaped child exits, its process group gets SIGTERM, then SIGKILL after grace |
| Depth | A child at the cap gets no Agent tool and no delegation block (`subagents_allowed`, `hoocode-code-cli/src/runtime.rs`) |

### Runs, handles, cancel

- Every Agent call is its own run, even with the same type. Two runs of one type with different descriptions are not duplicates.
- Handle: `type#N`, unique per agent type per session. Counters continue after `--resume`. An old handle answers `explore#1 ran in an earlier process and is not running now`.
- A background call's reply names the handle and description at once. A foreground call names it on its `resume_task_id:` line.
- `AgentOutput(list: true)` shows every background run with handle, description and status.
- `AgentOutput("explore#1", cancel: true)` stops that run only. A cancel while starting settles it `cancelled` and it never starts. A cancel on a settled run says `nothing was cancelled`.
- Tool names: `Agent` and `AgentOutput` (`wait`, `list`, `cancel`, `timeout_ms`, `resume_task_id`). Old names (`Task`, `TaskOutput`, `AgentOut`) are unknown. No aliases.
- Reservations are keyed by (session, call id). A refused call settles its handle `failed` with the reason.

### Process safety (tree kill)

- **Policy:** a subagent's descendants are SIGKILLed on cancel, deadline or dispose. Jobs re-parented to init are not.
- **Only our own unreaped child is walked.** `plan_tree_kill` returns nothing unless the table shows the root with `ppid == getpid()`. This process, its ancestors and their groups are never killed. Table from `/proc` (or `/bin/ps` off Linux).
- **A reaped pid is never signalled.** The registry (`lifeguard::child_spawned`, `child_exited`) marks the child exited before the reap. Kills by pid check it under the lock.
- **Spawn generations.** A kill whose generation is not the live one for that pid is refused, so a stale kill never hits a reused pid.
- **Groups** are killed only when a descendant leads them (`pgid == pid`, `pgid > 1`), not when they are ours or an ancestor's. `terminate_group` refuses pid 0.
- **Freeze, then kill:** SIGSTOP the group, walk, SIGSTOP descendants, walk again, SIGKILL. `StoppedGuard` SIGKILLs anything left stopped. The warm worker walks under its child lock before stdin closes.
- **Limits:** SA-9 and SA-10.

Tests: `crates/hoocode-code-subagents/tests/it/tree_kill.rs`, `lifeguard.rs`, `warm.rs`.

### Ledger

- File: `<cwd>/.hoocode/dispatch/ledger.jsonl`. One line per attempt, one `write_all` per record. Telemetry only. A write failure is logged and dropped. Bounded: past 20 000 lines, trimmed to the newest 10 000.
- Reads: `/subagent-stats [24h|7d|all]`, or Rust `ledger::stats(&cwd, since)` and `success_rate`. Code: `crates/hoocode-code-subagents/src/ledger.rs`.

### Parent behaviour

`crates/hoocode-code-subagents/templates/prompts/task-main.md` is the one template. Results arrive at the next turn boundary (no mid-turn injection). Dispatch all independent subtasks in one message. Use `wait: true` only for one named result.

### Writer worktree protocol

| # | Who | Action |
|---|---|---|
| 1 | Parent | Check the task needs a writer. Note base with `git rev-parse HEAD`. Warn if the tree is dirty |
| 2 | Parent | Pick a unique `<label>` (`git branch --list agent/<label>` must be empty). Run `git worktree add -b agent/<label> ~/.hoocode/worktrees/<repo>/<label> <base>` |
| 3 | Parent | One writer per worktree. No overlapping files, including the parent's own. Put path, branch and base in the brief |
| 4 | Child | Work only under the worktree, with `git -C <path>`. Run the project's checks. Commit on `agent/<label>`. Never push, never switch branches, never write the parent tree. Report branch, commits, diff stat, checks |
| 5 | Parent | Read `git log --oneline <base>..agent/<label>` and the diff stat. Verify, or dispatch code-review. Report branch and `git merge agent/<label>`. The user merges |
| 6 | Parent | Session end: for each branch in `git branch --merged <base>`, `git worktree remove`, then `git branch -d`. Keep and report the rest |

Build output is per project. Each worktree uses its language's default location. Sharing a cache is a project rule in its AGENTS.md or CLAUDE.md, not harness code.

### Evals

Harness `scripts/eval/subagent_evals.py`. Mock `scripts/eval/mock_provider.py`. Needs `cargo build -p hoocode-code-main` first (the harness runs `target/debug/hoocode`).

```bash
scripts/eval/subagent_evals.py                    # fast suite, about 25 s
scripts/eval/subagent_evals.py --include-slow     # plus the two slow scenarios, about 3 min
scripts/eval/subagent_evals.py --matrix           # every scenario × 5 faults
scripts/eval/subagent_evals.py --soak 600         # randomised faults, leak check
```
Also: `--only <name>`, `--repeat N`, `--min-success 0.8` (gate on usable rate).

Reports: `target/subagent-evals/` (`report.md`, `report.json`, transcripts, request log).

**Pass bars:**

| Check | Bar |
|---|---|
| Scenario | Meets its declared expectations (an absent field is not checked). Usable = `complete` or `partial` with a result |
| Exit code | 0 when all scenarios meet expectations and usable rate ≥ `--min-success` (default 0.0) |
| CI fast suite | Nightly and manual only, never on PR. `--min-success 0.8`. Non-blocking |
| CI nightly | `--matrix` and `--soak 600`. Non-blocking |
| Known issue | Recorded, not failed. None declared (SA-6) |

**Method:**

- Real binary, real parent, Agent tool, pool and child process, with a scripted mock provider. No `/bin/sh` mocks on the subagent path. Unit tests use `ScriptedRunner`.
- A child's last user message ends in `Task: …`. Each route has its own script and cursor. `--repeat` resets cursors. An exhausted script answers `500 mock: script exhausted`.
- Turn keys: `text`, `reasoning`, `tool_calls`, `usage`, `delay_s`, `hang`, `abort_after`, `error`, `corrupt_after`. `worktree: true` gives a temp repo and bare origin (`<REPO>`, `<WT>`, `<ORIGIN>`).
- Expect fields: `result_status`, `result_confidence_min`, `ledger_*`, `summary_contains`, `error_contains`, `child_requests`, `parent_requests`, `parent_tool_calls_before_result`, `peak_inflight_min/max`, `attempts_usable_min`, `branch_commits`, `parent_status_unchanged`, `nothing_pushed`, plus needles. Scenario fields: `known_issue`, `slow`, `worktree`, `settings`, `env`.

**Faults (`MATRIX_FAULTS`):** `none`, `rate-limited` (429, retry-after 1), `mid-stream-abort` (120 bytes), `corrupt-line` (120 bytes), `provider-error` (500).

**Scenarios (22 in code):**

| Family | Scenarios |
|---|---|
| Child | `child_direct_answer`, `child_tool_call_then_answer`, `child_turn_limit_settles_partial`, `child_invalid_tool_arguments`, `child_stream_abort_mid_answer`, `child_deadline_wrapup_partial` (slow), `child_rate_limited_then_answers`, `child_corrupt_jsonl_line`, `child_context_overflow` |
| Parent dispatch | `parent_dispatch_blocking`, `parent_dispatch_background`, `parent_works_while_background_runs`, `parent_tells_parallel_runs_apart`, `parent_stall_reaped` (slow) |
| Parent rules | `parent_region_error_falls_back`, `parent_unknown_model_name`, `parent_legacy_alias_rejected`, `subagent_shell_denied_without_a_ui` |
| Saturation | `parent_queue_saturation` (peak must equal root cap), `nested_depth_chain` (depth 3 gets no Agent tool), `nested_fanout_saturation` (peak must equal nested cap) |
| Writer | `worktree_writer_commits_on_branch` |

Not covered by any scenario: warm pool (SA-1), in-process mode, RSS limit, progress and tool stall numbers (SA-7).

The mock tests mechanics only. It does not show that a real model follows the brief (SA-2).

**Last recorded:** `--include-slow` 22/22 as declared, 33 attempts, 32 usable (97%), 2026-10-30. Matrix 65/65 and soak 26 rounds, 0 leaks, were 2026-10-05 and predate the cancel and tree-kill rounds (SA-4).

## Decisions

| Date | Decision |
|---|---|
| 2026-10-05 | Child processes are the shipped model. In-process is behind the seam, not enabled |
| 2026-10-03 | Token budget is advisory. Count output tokens; context goes in `peak_context` |
| 2026-10-05 | `result.json` decides correctness. Ledger is telemetry only |
| 2026-10-05 | Stall is a signal: SIGUSR1, grace, SIGKILL. A valid partial result is accepted |
| 2026-10-05 | Killed with a valid result settles `partial`. No prose `error` on killed tasks |
| 2026-10-10 | One deadline for every agent type: 4 h base, load-scaled to 4 h 30 m |
| 2026-10-10 | Parent can poll and cancel a run (`AgentOutput` with or without `cancel: true`) |
| 2026-10-08 | Tool names `Agent` and `AgentOutput`. No aliases |
| 2026-10-09 | Background by default. Frontmatter `background: false` is honoured. The parent's argument wins |
| 2026-10-09 | No barrier wait while independent work exists. Isolation by instruction. Only writers get worktrees |
| 2026-10-14 | Worktrees at `~/.hoocode/worktrees/<repo>/<label>`, branch `agent/<label>` |
| 2026-10-09 | Subagents commit on their branch and never push. User merges. Parent removes only merged worktrees and branches, at session end |
| 2026-10-14 | Harness never enforces worktree isolation. Dirty parent tree: warn only. Commit only if asked |
| 2026-10-14 | Build output is per project. No harness setting (supersedes the old `CARGO_TARGET_DIR` answer) |
| 2026-10-09 | Every agent, read-only too, gets AGENTS.md and CLAUDE.md |
| 2026-10-15 | Custom bodies get base guidelines at depth > 0. One prompt wrapper, no `context` field |
| 2026-10-30 | Every call is its own run. Handles are `type#N`, per session, continue after `--resume`. Descendants killed on cancel, deadline or dispose; re-parented jobs are not |
| 2026-10-03 | Fallback retries region errors and `finish_reason: error`. Never retries a user cancel |
| 2026-10-16 | Root 12, nested 6, queue 8 per slot, depth 4, wait 300 s, Shell default 1200 s |

Dispatch dirs are pruned (newest 50, older than 24 h dropped), so they are not kept for the whole session. Recover is view or resume on request, never automatic (TUI rule, 2026-10-09).

## Open

Size: S about a session, M a few, L many.

| ID | Item | Size | Trigger or gate | Design notes |
|---|---|---|---|---|
| SA-1 | Warm-pool evals: cap, reuse, cancel, stop path | S | Next change to `warm.rs`, or before warm pool goes on by default | No scenario exists. Set `warmSubagents: true`. Assert peak in flight = root cap. Cancel answers `cancelled`, the `sleep` is gone |
| SA-2 | Real-model trial of the brief and writer protocol | S | Before relying on writers in real work | One general-purpose writer on a temp repo, real model. Check one commit on `agent/<label>`, no push, parent tree unchanged. Record the result here |
| SA-3 | Review-agent Shell filter: only `git diff`, `git log`, `git show` | S | A review agent runs another command in a real run | Text rule is in `code-review.md:27`, `security-review.md:28`. Enforce with an agent-scoped allowlist in the Shell tool. Not found in code |
| SA-4 | Re-run `--matrix` and `--soak 600` after the cancel and tree-kill rounds | S | Now. Last figures (2026-10-05) predate those rounds | Run locally or read the nightly artifact. Record the figures here |
| SA-5 | Re-run `parent_queue_saturation` at the root cap of 12 | S | Next `--include-slow` run | Last run was at 8 slots. Peak in flight must equal 12 |
| SA-6 | `known_issue` mechanism is unused | S | Next eval review | Recommend removing the field and its report branch unless a real known issue appears. User decides |
| SA-7 | Evals for progress-stall, tool-stall and RSS limits | M | Before changing their defaults | One scenario each, using low env values (children inherit them): `HOOCODE_SUBAGENT_TOOL_STALL_MS`, `HOOCODE_SUBAGENT_MAX_RSS_MB` |
| SA-8 | In-process child mode (Track 2) | L | A measured need for lower per-dispatch cost. Floor 8.9 ms vs 459.9 ms (no-op, 2026-10-05) | 1: per-run depth field replaces the `SUBAGENT_DEPTH` process global (`depth.rs`, `runtime.rs`). 2: production factory in the CLI. 3: depth assertions in `an_in_process_child_can_dispatch_its_own_child`. Until then, leave off |
| SA-9 | Group sweep misses a nested subagent's own process group | M | A leaked process seen in a run or soak | The parent sweep covers the child's group only (H2). Reach nested groups through the descendant walk |
| SA-10 | Re-parented grandchildren (double-forked before cancel) are not found | M | Only if the policy on re-parented jobs changes | Accepted gap today. Options if needed: subreaper or cgroup. Do not build without a policy change |
| SA-11 | `maxOutputTokens` read only by Anthropic | S | A subagent truncates on a non-Anthropic model | OpenAI, Responses, Google and Gemini CLI cap at min(model max, 32000). Pass the setting through |

## Not doing

- A Rust worktree manager, or a `worktree` or `cwd` parameter on Agent.
- Harness enforcement of worktree isolation.
- Auto-merge, pushing, or worktrees for read-only agents.
- Mid-turn injection of results. Locks or a scheduler for file overlap (the parent checks by hand).
- Budget enforcement. Per-type deadlines. Salvaging a result from `session.jsonl` after a kill. Delta token accounting.
- Idempotency by tool call id.
