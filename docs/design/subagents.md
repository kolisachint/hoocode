# Subagents

Status (2026-11-19): built. Each subagent is a child process. Runs are background by default. Writers work in git worktrees and commit but never push. Tool lists take selectors (2026-11-19, plugins.md PL-17). Built in P2 for `--tools` and agent lists. Built in P2b: a malformed agent entry rejects the definition, and clearance requests carry the tool's origin. Open items are SA-1 to SA-11. Detached sessions (SA-12) phase 1 built 2026-11-25.

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
| `tools` | Allowlist of selectors: a tool name, `plugin:<p>`, `plugin:<p>/<Tool>`, `plugin:*`, `mcp:<s>`, `mcp:<s>/<tool>` or `mcp:*` (plugins.md Naming and identity). Agent is still offered when delegation is allowed. A malformed entry rejects the definition |
| `model` | Default model. A call's `model` overrides |
| `background` | `false` = foreground default, kept as `Some(false)` |
| `delegate` | `true` = may dispatch |
| `fork` | Same flag helper as `background` |
| `maxTurns`, `tokenBudget` | Per-agent limits (above) |

Rules:

- A writer has Edit, Write or Shell. Writers get the worktree instruction. Read-only agents do not.
- Entries in an agent's `tools` or `disallowedTools` are selectors, matched on tool origin (plugins.md Naming and identity). `WebFetch` and `WebSearch` are the webtools plugin's own tool names. A name does not cover other plugin tools: to keep an agent off the network, deny `plugin:*`. Built in P2 (2026-11-19).
- Built-in tool lists: general-purpose is `Read, Shell, Edit, Write, CodeSearch, plugin:*`. explore, plan, code-review and security-review have no plugin selector (plugin tools may write later).
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
| Parent rules | `parent_region_error_falls_back`, `parent_unknown_model_name`, `parent_legacy_alias_rejected`, `subagent_shell_runs_under_a_headless_parent` |
| Saturation | `parent_queue_saturation` (peak must equal root cap), `nested_depth_chain` (depth 3 gets no Agent tool), `nested_fanout_saturation` (peak must equal nested cap) |
| Writer | `worktree_writer_commits_on_branch` |

Not covered by any scenario: warm pool (SA-1), in-process mode, RSS limit, progress and tool stall numbers (SA-7).

The mock tests mechanics only. It does not show that a real model follows the brief (SA-2).

**Last recorded:** `--include-slow` 22/22 as declared, 33 attempts, 32 usable (97%), 2026-10-30. Matrix 65/65 and soak 26 rounds, 0 leaks, were 2026-10-05 and predate the cancel and tree-kill rounds (SA-4).

### Detached sessions (SA-12)

Designed and built (phase 1) 2026-11-25. Eval: `scripts/eval/detached_evals.py`, 9/9 on 2026-11-25.

Purpose: hand work to a fresh session with its own context. Compaction keeps a tail and summarises summaries; a fresh session starts from a brief only.

| | Subagent | Detached session |
|---|---|---|
| Result | Returns to the parent | None. The parent shows a link |
| Lifetime | Dies with the parent | Own process group, outlives the parent |
| Trust | Parent's gate via clearance | Tier 1, same as a session the user started |
| First prompt | Brief | Brief the user approved |

Flow:

1. The model calls `SessionSpawn { brief }`, when asked or on its own. The call is gated: the approval shows the brief and the effective `auto_allow` list (what will run unattended). An approved brief carries user authority. A headless parent follows its gate's headless default.
2. The brief has four sections: Goal, Decisions, Dead ends, Next step. A missing section is a tool error. The harness appends facts: cwd, branch, HEAD, dirty files, todos, parent session path.
3. The harness creates the child file path itself, `<parent session dir>/<ts>_<id>.jsonl` (absolute), and starts `hoocode -p --session <path> --max-turns <n> --deadline-ms <ms>` in the parent's cwd, with `setsid`, SIGHUP ignored, stdin `/dev/null`, stdout and stderr to `<path>.log`. Env: `HOOCODE_DETACHED=1`, `HOOCODE_SUBAGENT_DEPTH` removed. The new header sets `parentSession`.
4. The parent shows a code block `hoocode --session <path>` and a status: running, parked, done, failed. Status comes from the lock and the session file.
5. The child runs. Calls `auto_allow` lists run. The first gated call outside `auto_allow` parks: the pending call is written, the file flushed, the process exits as parked. Print mode's "allow and note on stderr" (runtime.md) does not apply.
6. `hoocode --session <path>`: lock held, read-only follow view; Ctrl-C sends SIGINT to the lock's pid. Lock free, normal interactive session; a parked call shows as an approval and is re-checked first (target changed: tool error to the model, no run). An "approve and detach" key starts the detached process again.

Rules:

- Lock: `<path>.lock` holds the pid, taken with `File::try_lock`; the OS drops it when the process dies, so a crash leaves no stale lock. One writer per session file.
- Detached runs flush after every entry, not at turn end, so the follow view is live per entry.
- Grants: config `auto_allow` only. The parent's session grants ("allow always") are not carried.
- Budget: the child enforces its own turn cap and deadline (subagent defaults, 200 turns and 4 h); no parent reaper exists. Hit: exit as failed with the reason.
- Depth 1: `SessionSpawn` is not registered when `HOOCODE_DETACHED` is set or in a subagent. The child's subagents keep the normal limits.
- Checkout: the child works in the parent's cwd. No worktree is made (the harness never enforces isolation; Not doing: a Rust worktree manager). The main use is handover, where the parent stops. If the parent keeps editing the same files, that is the user's call; the approval says so.
- A parked or denied tool result says why, so the model stops and does not retry.

Not in phase 1: interaction while running (TUI as app-server client, phase 2), opening a terminal or tmux window, messages between parent and child, a context-used signal to the model, a `cwd` for the child, `--resume <id>` by id.

Eval (gate for done): scripted long session, spawn, the fresh session does the next step without asking again and does not retry a recorded dead end. Plus: park and resume, follow view while running, lock refuses a second writer, a killed child leaves no lock, depth-1 refusal, turn cap and deadline exit.

Decided 2026-11-25 (Decisions): the parked-call entry format, the follow view, `--session <path>` from another cwd, the resume restart.

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
| 2026-11-17 | Subagents get WASM plugin tools (gated calls go through clearance). Never MCP plugin tools or plugin-management tools. See [plugins.md](plugins.md) |
| 2026-11-17 | Agent tool lists accept `ext_*` (all plugin WASM tools). The built-in general-purpose agent has it; read-only built-ins do not. See [plugins.md](plugins.md) (superseded 2026-11-19: `plugin:*`) |
| 2026-11-18 | In agent definitions, `WebFetch` and `WebSearch` map to the webtools plugin tools `ext_webtools_fetch` and `ext_webtools_search` only. Other plugin WASM tools with network access are not covered. To keep an agent off the network entirely, deny `ext_*` (see [plugins.md](plugins.md)). (superseded 2026-11-19: see the row below) |
| 2026-11-19 | A plugin selector (`plugin:*`, `plugin:<p>`, `plugin:<p>/<Tool>`) takes no MCP tool. A subagent runs no plugin MCP server and exposes no plugin MCP tool, whatever its tool list says, so general-purpose (`plugin:*`) starts no MCP. Plugin WASM tools are unchanged. Never MCP plugin tools (see [plugins.md](plugins.md)). |
| 2026-11-19 | A json or print run that has subagents answers their clearance requests with its own gate (`HeadlessClearanceRelay`): the same rules, and the same headless default. A fail-closed run denies with the gate's reason; an open run runs the call, as it runs its own. Before, a subagent's gated call was denied with "no UI to ask" whatever the run would have allowed. Tests: `hoocode-code-permissions` `relay`; eval `subagent_shell_runs_under_a_headless_parent`. |
| 2026-11-19 | Agent `tools` and `disallowedTools` take selectors: a name, `plugin:<p>`, `plugin:<p>/<Tool>`, `plugin:*`, `mcp:<s>`, `mcp:<s>/<tool>` or `mcp:*`. `ext_*` is removed, with no alias. A malformed entry rejects the definition. Supersedes the `ext_*` row of 2026-11-17 and the WebFetch row of 2026-11-18. See [plugins.md](plugins.md) Naming and identity. |
| 2026-11-25 | SA-12 parked call: a `custom` entry `parked_call` with `toolCallId`, `toolName`, `reason` and, for a tool with a `path` or `file_path` argument, `target` (`path`, `sha256` or null if absent). The call is open while no tool result with its id follows. The child also ends with a `custom` entry `detached_exit` (`status`: `done`, `parked` or `failed`, `reason`). Status: lock held → running; open parked call → parked; else the exit entry; no exit entry and no lock → failed (crashed). No new entry type: `custom` already round-trips and old readers skip it. |
| 2026-11-25 | SA-12 follow view: the normal TUI, with the session opened read-only and no agent run. It re-renders through the resume path (`render_session_context`) when the file grows (1 s poll, the git-poll cadence). The prompt does not send; Ctrl-C sends SIGINT to the lock's pid. When the lock frees, the view reopens the file as a normal session. No second renderer. |
| 2026-11-25 | SA-12 `--session <path>` from another cwd: a value with `/` resolves as a path and opens directly (`resolve_session_path`), so the cross-project fork prompt never shows; the parent always passes an absolute path. Test: `session_flags` unit test. |
| 2026-11-25 | SA-12 resume restarts the child with `hoocode -p --session <path>` and no prompt: a detached print run with no prompt continues from the session's last tool result (`continue`), so no fake user message is written. |
| 2026-11-25 | SA-12 park scope: only the detached run's own calls park (`ApprovalChannel::Detached`, `Verdict::Park`; exit code 3). A gated call from one of its subagents is denied with a reason, not parked: parking would exit the process under running subagents. Deny rules still deny; `auto_allow` still runs. |
| 2026-11-25 | Detached sessions (SA-12): a model-callable, user-approved `SessionSpawn` starts a tier-1 session in its own process (`hoocode -p`, `setsid`) in the parent's cwd. No result to the parent; the parent shows `hoocode --session <path>`. Gated calls outside `auto_allow` park, not deny; opening the session answers them. Session lock, per-entry flush, self-enforced budget, depth 1. No worktree: isolation stays by instruction. Phase 2 (live interaction via an app-server client in the TUI) waits for need. See Detached sessions. |
| 2026-11-25 | SA-12 resume: an open parked call shows the clearance card with yes, approve and detach (`d`, `app.permission.detach`), no. No "always" row: grants are config `auto_allow` only. A target changed since parking is answered "not run" with no card. Approve and detach opens a new empty session to drop the lock, then restarts the child (`child_command_continue`, in `code-subagents` `detached_child.rs` so the TUI can reach it). The approved call runs without the loop's `before_tool_call` hooks. |
| 2026-11-25 | SA-12 parent status: a Detached tab in the task panel, one row per session this session spawned (rebuilt from its `SessionSpawn` results on open): running, parked, done or `failed: <reason>`, with `hoocode --session <path>` in the detail. Polled each 1 s while not done or failed; a running row costs a lock probe, a parked row is re-read only when its file grows or is held. Parked is yellow (it is the one row that waits on the user), so `task_panel.rs` joins the yellow guard's file list. A notice on a change to parked or failed, never for rows restored on open. A crash reading in the first 5 s after spawn stays running (the child may not hold its lock yet). |
| 2026-11-25 | SA-12 lock probe: `lock_holder` takes the lock for an instant, so `SessionLock::acquire` retries for about 100 ms before it reports a held lock. Without that, a writer starting during the parent's 1 s probe could be refused. |
| 2026-11-25 | SA-12 review fixes (Opus review): only the loop's own call parks; a plugin's in-tool network request in a detached run is denied (`request_in_tool`). Readers load without the lock (`open_read_only`: app-server reads, attach, follow refresh); the app-server refuses to open a held session. A lock I/O error (no flock on the filesystem) warns and keeps writing. A changed parked target is answered with a not-run decision that never runs the tool. The `SessionSpawn` card has no "always" row. Kept: a never-written session's lock file is deleted on drop while held; only a second opener of the same random path could race. |
| 2026-11-25 | SA-12 eval scope: the mock checks mechanics (the brief, Dead ends and facts reach the child; it does the next step and offers no `SessionSpawn`). It cannot show that a real model honours the brief (as SA-2). A headless (json or print) parent spawns without an approval, per its gate's default. |

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
