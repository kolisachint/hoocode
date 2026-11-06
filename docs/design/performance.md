# Performance: speed, concurrency and memory

Unified 2026-11-06. This page is the index and the status of record for runtime performance. Detail and history stay in [concurrency.md](concurrency.md), [perf-short-sessions.md](perf-short-sessions.md) and [tui-activity.md](tui-activity.md); where they disagree on status, this page wins. Dated decision pages win on decisions: [decisions-2026-10-29.md](decisions-2026-10-29.md) and [decisions-2026-11-06.md](decisions-2026-11-06.md).

Every status below was checked against the code on 2026-11-06 ("review 2026-11-06"). Developer build speed is not covered here. See [build-speed.md](build-speed.md).

## Goal

- Never freeze. Typing and Ctrl+C stay instant while the model streams, tools run and subagents work.
- Keep 10 to 15 turn sessions lean. At that length the parent is about 33 to 36 MiB RSS, frame p50 about 0.7 ms.
- Measure first. A speed or memory change needs a number before it lands. A correctness fix does not wait.

## Rules

The first nine rules are from [concurrency.md](concurrency.md) and are carried as written there. Rule 10 is new on 2026-11-06.

1. Build threads, runtimes and channels only through `hoocode-runtime`.
   - Enforced by: `clippy.toml:4-9` (runtime builders and `thread::spawn`).
   - Gap: clippy does not forbid unbounded channels. There are unbounded channels outside the runtime (O-6).
2. Never block the UI thread or a `hoocode-io` worker; use `run_blocking`.
   - Enforced by: `run_blocking` at `crates/hoocode-runtime/src/runtime.rs:113`.
   - Gap (partial): `spawn_blocking` is used directly at `crates/hoocode-agent-loop/src/lib.rs:1144`, `crates/hoocode-code-subagents/src/runner.rs:334`, `crates/hoocode-code-rpc/src/mode.rs:550` and `crates/hoocode-code-cli/src/auth.rs:111`. Tool bodies run on tokio's blocking pool, uncapped by decision (Decisions 6, Open question 3).
3. Every queue states its cap and policy; every external wait states its deadline.
   - Enforced by: the session writer queue (4096 entries or 64 MiB, producer waits) and the MCP 60 s deadline.
   - Gap: the stream queue is unbounded (Open question 2). Unbounded channels remain (O-6). No LLM first-byte or idle deadline exists (O-7). The 16 MiB SSE line cap is only in MCP streamable HTTP (O-5).
4. No `block_on` except at entry points (`main`, tests).
   - Enforced by: nothing in `clippy.toml`. It lists runtime builders and thread spawns only. `concurrency.md:147` says otherwise (see Doc drift).
   - Gap (partial): 13 `block_on` calls outside tests; 8 are bridges or entry points, 5 are left to fix (see Implemented).
5. No lock held across `.await` or while calling listeners.
   - Enforced by: review. Agent listeners are called on a copy, outside the lock (`crates/hoocode-agent-core/src/lib.rs:236-238`).
   - Gap: none found in review. Listeners still run inline on the loop task, so a slow listener slows the loop.
6. Session data is never dropped.
   - Enforced by: the session writer. Its producer waits at `crates/hoocode-runtime/src/session_io.rs:245-252`.
   - Gap: none recorded.
7. OS priority only goes down, never up.
   - Enforced by: lanes at nice 0, +5, +10 (`crates/hoocode-runtime/src/lanes.rs:28-33`). Child processes start at nice +5 (`crates/hoocode-runtime/src/child.rs:27-52`).
   - Gap: the lane code is Linux only.
8. Nothing kills hoocode itself.
   - Enforced by: no single check in the code. Related: the ordered shutdown (`crates/hoocode-runtime/src/shutdown.rs`), the watchdog (`crates/hoocode-runtime/src/watchdog.rs:19-31`) and the child lifeguard, which stops child processes, not hoocode.
   - Gap: no direct test recorded.
9. Four settings, everything else fixed.
   - Enforced by: the settings schema (`crates/hoocode-code-settings/src/types.rs`).
   - Gap: the count of four is not checked. Settings in use include `maxParallelTools` (`crates/hoocode-code-settings/src/manager.rs:164`), `bashNice` and the memory limits (`manager.rs:660-686`).
10. Measure first; optimise for 10-15 turn sessions (decisions-2026-10-29 #1).
    - Enforced by: a gate on every item in What we build.
    - Gap: the Step 0 baseline is not done (O-1).

## Targets

| Target | Value | Status | Source |
|---|---|---|---|
| Keystroke-to-frame p99 | under 16 ms | open | 6 of 7 bash runs met. One 27 ms miss, cause unknown. Real-TUI stall check not done. [README.md](README.md) rows 1 and 2 |
| Ctrl+C abort | under 500 ms | met | 30 ms with a paused pty. [README.md](README.md) status, 2026-10-15 |
| Frame build p99 on a 10k-line transcript | under 8 ms | open, not measured on 10k lines | Related: burst tail p99 100 to 136 ms, from frames that draw the bash block. Phase 6 deferred by the user (Deferred) |
| Thread ceiling | 26 threads | met | Peak 24, down from 34. `hoocode-io` is uncapped by decision |
| UI stall log | 500 ms | met | `crates/hoocode-runtime/src/watchdog.rs:19-31` |
| UI stall footer notice | 2 s | met | `concurrency.md:152`; `crates/hoocode-tui-terminal/src/interrupt.rs:18` |
| Starvation probe | 1 s; late over 250 ms | met | `crates/hoocode-runtime/src/watchdog.rs:19-31` |
| Ordered shutdown | 3 s | met | 2.8 s plus a 0.2 s reserve. `crates/hoocode-runtime/src/shutdown.rs` |
| Memory soft limit | min(2 GiB, 25% RAM) | met (render-cache clear on shed open) | `crates/hoocode-code-settings/src/manager.rs:660-686` |
| Memory hard limit | min(4 GiB, 50% RAM) | met | `crates/hoocode-code-settings/src/manager.rs:660-686` |
| Idle wakeups | 5 per s | not met; accepted at about 7 per s | Accepted 2026-10-30 (`perf-short-sessions.md:342-346`, measured at :395). From 17.2 per s |
| Idle CPU | under 0.2% | borderline (0.16 to 0.2%) | `perf-short-sessions.md:401` |

## Decisions

| # | Decision | Answer | Date / source |
|---|---|---|---|
| 1 | What to optimise for | Sessions of 10 to 15 turns | 2026-10-29 #1 |
| 2 | Long-session items | Deferred. See Deferred | 2026-10-29 #2 |
| 3 | Measure-first gates for the child footprint (M3) and streaming re-lex (M4) | Propose a change only if the gate is met | 2026-10-29 #5, #6 |
| 4 | Subagent concurrency | Root 12 kept. Nested 6, depth 4 | 2026-10-29 #5 (changing the default needs the user) |
| 5 | Lean subagent child | Rejected. Measured 2.6 MiB per child; the gate was 8 MiB | Measured 2026-10-30, `perf-short-sessions.md:145-150`; gate from 2026-10-29 #5 |
| 6 | io blocking pool | Uncapped | 2026-10-15 (user decision) |
| 7 | Idle wakeups | Accept about 7 per s. Target stays 5 per s | 2026-10-30, `perf-short-sessions.md:342-346` |
| 8 | One unified performance page | This page is the status of record | 2026-11-06 |
| 9 | Design review plan (v2) | Approved. Build order in What we build | 2026-11-06 |
| 10 | C5 worker parse | Folded into the deferred lazy model catalog | 2026-11-06 |

Note on IDs. The `P-` IDs are the v2 items. The older measure-first items M3 (child footprint) and M4 (streaming re-lex) keep their names here. They are not P-M3 or P-M4.

## Implemented

Only DONE and PARTIAL items. PARTIAL items say what is missing.

### Runtime and threads

| What | Where | Notes |
|---|---|---|
| `hoocode-io` multi-thread runtime, min(4, cores) workers; 2 in children | `crates/hoocode-runtime/src/runtime.rs:21-23,38-44` | `concurrency.md:23` still says one per core (drift) |
| Tools pool: current_thread, up to 16 threads, `run_blocking` | `crates/hoocode-runtime/src/runtime.rs:25,82-88,113` | Lives in `hoocode-runtime`, not `hoocode-tools` |
| Second `ai-stream` runtime removed; per-server MCP `Runtime::new` removed | Removed | `concurrency.md:24-25` still lists both (drift) |
| Thread spawns only through `hoocode-runtime`; thread ceiling 34 to 24 | `clippy.toml:4-9` | Target 26 met |
| Lanes: nice 0, +5, +10 on Linux | `crates/hoocode-runtime/src/lanes.rs:28-33` | Linux only |
| Child process at nice +5; `bashNice` setting | `crates/hoocode-runtime/src/child.rs:8,27-52` | |
| Output thread; frames keep-latest | `crates/hoocode-tui-terminal/src/output.rs:33,66-81` | |
| **PARTIAL.** Rule 2: blocking work on `run_blocking` or `spawn_blocking` | `crates/hoocode-agent-loop/src/lib.rs:1144`; `crates/hoocode-code-subagents/src/runner.rs:334`; `crates/hoocode-code-rpc/src/mode.rs:550`; `crates/hoocode-code-cli/src/auth.rs:111` | Missing: tool bodies on the io pool (uncapped by decision). Child `wait()` and the Agent body are not async yet |
| **PARTIAL.** Rule 4: `block_on` at entry points only | 13 sites outside `tests/`: 5 are the runtime's own sync bridges (`crates/hoocode-runtime/src/runtime.rs:105,135,149,162`, `bg.rs:31`); 3 are CLI entry points (`crates/hoocode-code-cli/src/runtime.rs:1328,1496`, `app_server.rs:152`) | Left to fix: `crates/hoocode-code-tool-bash/src/jobs.rs:945`, `crates/hoocode-code-agent-session/src/mcp.rs:780`, `crates/hoocode-code-subagents/src/tools.rs:787` (`block_in_place`), `crates/hoocode-code-tools/src/lib.rs:488`, `crates/hoocode-ai-util/src/retry_delay.rs:519` (check whether test-only) |

### Agent loop and tools

| What | Where | Notes |
|---|---|---|
| Parallel tool calls: `FuturesUnordered` and a fair semaphore; `maxParallelTools` 8 (range 1 to 32) | `crates/hoocode-agent-loop/src/lib.rs:880-923`; `crates/hoocode-runtime/src/limits.rs`; `crates/hoocode-code-settings/src/manager.rs:164` | |
| Ordered start turnstile for Edit and Write; per-file mutation queue | `crates/hoocode-agent-loop/src/lib.rs:825-858`; `crates/hoocode-code-tools-fs/src/mutation_queue.rs` | Read, Grep and Glob run unordered |
| Background tool manager cap 64 | `crates/hoocode-agent-loop/src/lib.rs:1368` | |
| Abort grace 250 ms; Agent tool 5 s | `crates/hoocode-agent-loop/src/lib.rs:677`; `crates/hoocode-code-subagents/src/tools.rs:841` | |
| Tool output truncation 32 KiB, with spill | `crates/hoocode-code-tool-api/src/truncate.rs:16` | A second default (50 KiB) is in `crates/hoocode-agent-harness/src/utils/truncate.rs` (drift) |
| MCP servers start concurrently | `crates/hoocode-code-agent-session/src/mcp.rs:489-505` | |
| MCP: 8 in flight per server; 32 MiB response cap; 60 s deadline | `crates/hoocode-agent-mcp/src/client.rs:46-54` | |
| MCP progress keep-latest | `crates/hoocode-code-agent-session/src/mcp.rs:775-797` | |
| **PARTIAL.** Tool progress keep-latest | Agent loop (not MCP) | Missing: agent loop. Planned as P-M6 (`concurrency.md:119`) |

### Streaming

| What | Where | Notes |
|---|---|---|
| `SharedMessage`: provider partial is `Arc`, copy on write; copy counter | `crates/hoocode-ai-types/src/lib.rs:785-848` | Supersedes the rejection at `tui-activity.md:352-356` |
| Loop sends events without the partial | `crates/hoocode-agent-loop/src/lib.rs:446-448`; `crates/hoocode-agent-types/src/lib.rs:124-131` | `MessageUpdate` carries `Arc` fields |
| Child stdout text-delta coalescing | `crates/hoocode-code-subagents/src/live.rs:185-215` | tui-activity T3.1 |
| Markdown lexer prefilter: 57 KB reply from 1.65 s to 46 ms | `crates/hoocode-tui-util/src/js_regex.rs:437`; tests `lexer_prefilter.rs` | |
| Streaming markdown: only the last chunk re-lexes, unless link definitions change | `crates/hoocode-code-tui-widgets/src/assistant_message.rs` (`segment_streaming_markdown`, about line 348) | Limits re-lex already. See O-10 |

### Subagents

| What | Where | Notes |
|---|---|---|
| Root 12, nested 6, depth 4 | `crates/hoocode-code-subagents/src/pool.rs:460`; `crates/hoocode-code-settings/src/types.rs:364-366` | 12 kept (2026-10-29) |
| Dispatch queue 8 per slot | `crates/hoocode-code-subagents/src/limits.rs:59` | |
| Children are processes; child RSS cap 4 GiB; SIGUSR1, SIGTERM, SIGKILL | `crates/hoocode-code-subagents/src/lifeguard.rs` | |
| Wall clock 4 h; captured tail 256 KiB | `crates/hoocode-code-subagents/src/pool.rs:112` | |

### TUI render

| What | Where | Notes |
|---|---|---|
| Transcript rebuild throttle 100 ms; app repaint 16 ms minimum gap; 256 app events per pass | `crates/hoocode-code-tui-app/src/mode/mod.rs:162,170,179` | |
| Wake handle coalescing | `crates/hoocode-tui-render/src/tui.rs:316-335` | |
| Component render stamps; container settled-skip; `HOOCODE_RENDER_CACHE_CHECK`; markdown cache per width and version | `crates/hoocode-tui-render/src/component.rs:21-65,319-385` | |
| Task panel: no per-frame clone; image header-only decode; `@file` autocomplete walks in-process, newest query only | `crates/hoocode-code-tui-widgets/src/task_panel.rs:184-210`; `crates/hoocode-tui-images/src/dimensions.rs`; `file_search.rs:67,155` (thread `autocomplete-files`) | |
| Idle wakeups 17.2 per s to about 7 per s: perf sampler only with `--perf-log`; footer git poll 1 s; UI idle cap 1 s | `crates/hoocode-code-tui-app/src/perf.rs:296-310` | Accepted at about 7 per s (Targets) |

### Session and storage

| What | Where | Notes |
|---|---|---|
| Session writer: queue 4096 entries or 64 MiB; producer waits | `crates/hoocode-runtime/src/session_io.rs:26-28,245-252` | Rule 6 |
| Append-only JSONL, one line per entry | `crates/hoocode-code-session/src/manager.rs:509` | |

### Startup and binary

| What | Where | Notes |
|---|---|---|
| Release profile: thin LTO, one codegen unit, strip | `Cargo.toml:209-212` | Binary 43.1 to 34.86 MiB stripped. First prompt 42 to 54 ms to 31 to 34 ms |
| **PARTIAL.** `--version` 3.8 to 5.29 ms | Release profile | Missing: acceptance. No A/B run (O-2) |
| Update check async; MCP start async; grammars lazy | `crates/hoocode-code-update/src/lib.rs:266-295`; `crates/hoocode-code-agent-session/src/mcp.rs:342-354`; `crates/hoocode-tui-highlight/src/lib.rs:50-66` | The update check runs on io (`crates/hoocode-code-cli/src/runtime.rs:2074`), not on background as `concurrency.md:71` says |

### Limits and watchdog

| What | Where | Notes |
|---|---|---|
| Watchdog: tick 250 ms; RSS every 4 ticks; UI stall 500 ms; starvation 1 s, late over 250 ms | `crates/hoocode-runtime/src/watchdog.rs:19-31` | Forced-stall tests: `tests/it/watchdog_forced.rs` |
| Memory soft and hard defaults | `crates/hoocode-code-settings/src/manager.rs:660-686` | |
| Shedding: parallel to 1; no new subagents | `crates/hoocode-runtime/src/limits.rs:63,79`; `crates/hoocode-code-subagents/src/pool.rs:955` | |
| **PARTIAL.** Render-cache clear on shed | Not found in code | Missing: the clear on shed |
| Emergency exit: Ctrl+C twice in 1 s | `crates/hoocode-tui-terminal/src/interrupt.rs:20-24` | Ctrl+C abort measured at 30 ms |
| Ordered shutdown, 3 s budget | `crates/hoocode-runtime/src/shutdown.rs`; `crates/hoocode-code-tui-app/src/mode/shutdown.rs` | 2.8 s plus 0.2 s reserve. MCP close wait 0.7 s (was 5 s) |
| Panic hook | `crates/hoocode-tui-terminal/src/panic_hook.rs`; installed in `crates/hoocode-code-tui-app/src/mode/mod.rs` | Not checked on a real tty |

## Baselines

Measured numbers as recorded. Step 0 (O-1) will add real 10 and 15 turn runs.

| Measure | Value | Source |
|---|---|---|
| `--version` | 3.8 ms; 12.2 MiB | `perf-short-sessions.md:364` |
| TUI first prompt RSS | 23.2 to 24.5 MiB | `perf-short-sessions.md:367` |
| Idle RSS | 24.9 MiB | `perf-short-sessions.md:368` |
| Print mode, one mock turn | 50 ms wall; 17.6 ms CPU | `perf-short-sessions.md:371` |
| RSS after turn 0 | 33.0 MiB | `perf-short-sessions.md:372` |
| Frame p50 at turn about 30 | 0.73 ms | `perf-short-sessions.md:374` |
| 300 turns | RSS 85.0 MiB; +0.17 to 0.18 MiB per turn; 13,142 frames; frame p50 2.37 ms; p99 12.5 ms | `perf-short-sessions.md:378-379` |
| Subagent children, 12 running | 22 to 34 MiB RSS each; sum RSS 339.6 MiB; sum PSS 124.1 MiB | `perf-short-sessions.md:246-249` |
| Allocator, glibc vs jemalloc | glibc 47.9 MiB at 9.23 s; jemalloc 52.4 MiB at 8.35 s (one run each) | `perf-short-sessions.md:328` |
| Lazy model catalog, per child | About 2.6 MiB per child; about 31 MiB at 12 children | `perf-short-sessions.md:256-257` |
| Binary size after release profile | 43.1 MiB to 34.86 MiB (stripped) | `perf-short-sessions.md:363,390` |
| Keystroke-to-frame, bash | 6 of 7 runs under 16 ms; one 27 ms miss | `README.md` row 1 |
| Frame p99 in a burst | 100 to 136 ms | `README.md` row 2 |
| Idle wakeups | 17.2 per s to about 7 per s | `perf-short-sessions.md:370,395` |
| Idle CPU | 0.24% to 0.16 to 0.2% (25 s) | `perf-short-sessions.md:369,401` |
| Markdown lexer, 57 KB reply | 1.65 s to 46 ms | `crates/hoocode-tui-util/src/js_regex.rs:437` |
| Streaming re-lex, 20 KB reply | About 1% of a core (estimate, not measured) | Review 2026-11-06 (O-10) |

Tooling: `--perf-log`, `/perf`, `scripts/perf/load_scenario.py`, `scripts/tui/mockllm.py`, `scripts/eval/subagent_evals.py`, `scripts/eval/mock_provider.py`, and valgrind massif and callgrind (run by hand).

## What we build (planned)

The `P-` items are the v2 plan approved 2026-11-06. The `O-` items are open items found in the older pages and confirmed in review 2026-11-06. Gate: "none (correctness)" or "none" needs no measurement. "measure first" names what to measure. Size is our estimate (S, M or L).

| ID | Item | Area | Gate | Size | Source |
|---|---|---|---|---|---|
| P-R0 | Responses tool-call routing fix | Providers | none (correctness) | M | v2, Correctness |
| P-C1 | Approvals stop freezing the batch | Agent loop | none | L | v2, Concurrency |
| P-C2 | Speculative read-only start | Agent loop | measure first: share of read-only calls before the last block; wall time saved on Anthropic and Responses | M | v2, Concurrency |
| P-C3 | One process-wide HTTP client | Providers | none | S | v2, Concurrency |
| P-C4 | `--continue` picks the newest valid session | Session | none | S | v2, Concurrency |
| P-S1 | `parse_streaming_json`: repair once, byte scan | Streaming | measure first: microbench and json-mode long reply | S | v2, Streaming CPU |
| P-S2 | Responses text: drop the mirror | Streaming | measure first: as P-S1 | S | v2, Streaming CPU |
| P-S3 | Child JSON path: text deltas straight to the coalescer | Subagents | measure first: as P-S1 | S | v2, Streaming CPU |
| P-S5 | TUI: `MessageUpdate` does not set dirty | TUI render | none | S | v2, Streaming CPU |
| P-M1 | `transform_context` takes a slice; 2 copies to 1 per model call | Memory | measure first: transcript copies in a child at peak (about 16x transcript) | M | v2, Memory; O-open item |
| P-M3 | `SessionManager` `by_id` index | Session | none | S | v2, Memory |
| P-M4 | `message_count()` and `has_messages()` | Session | none | S | v2, Memory |
| P-M6 | Tool progress: latest per call id; final drain kept. Includes the agent-loop keep-latest item | Agent loop | none | M | v2, Memory; `concurrency.md:119` |
| P-X1 | CodeSearch: compile the term regex pair once per term per query | Tools | none | S | v2, Small |
| P-X2 | UserMessageComponent: render stamp at construction | TUI render | none | S | v2, Small |
| P-X3 | ShellOutput wait: Condvar on exit, bounded wait slice for abort | Tools | none | S | v2, Small |
| O-1 | Step 0: measured 10 and 15 turn baseline | Measurement | none | S | `perf-short-sessions.md:27-38` |
| O-2 | P1 follow-ups: 100-turn mock CPU A/B; LTO build cost; `--version` and `--help` regression (5.29 vs 3.8 ms); unstripped CI artifact | Startup | measure first: CPU A/B, build time, `--version` time | S | `perf-short-sessions.md:336-341,391-405`; decisions-2026-10-29 #3 |
| O-3 | io-2 wakeup source trace (about 2 per s) | Idle CPU | measure first: wakeup source trace | S | `perf-short-sessions.md:347-350` |
| O-4 | `MALLOC_ARENA_MAX=2` in subagent `child_env` (`crates/hoocode-code-subagents/src/pool.rs:1497`); saves about 1.2 MiB per child | Subagents | measure first: CPU A/B | S | `perf-short-sessions.md:264-265`; Open question 5 |
| O-5 | SSE 16 MiB line cap in provider decoders (only MCP streamable HTTP has it today) | Providers | none (correctness, rule 3) | S | Review 2026-11-06 |
| O-6 | Unbounded channels outside the runtime: `crates/hoocode-agent-loop/src/lib.rs:777,915`; `crates/hoocode-code-cli/src/runtime.rs:1247`; `crates/hoocode-app-server/src/server.rs:816-817`; `crates/hoocode-app-server/src/transport.rs:309`; `crates/hoocode-code-rpc/src/client.rs:585` | Queues | none (correctness, rule 3) | M | Review 2026-11-06 |
| O-7 | LLM first-byte and idle deadline (none today) | Providers | none (correctness, rule 3) | S | Review 2026-11-06 |
| O-8 | Phase 3: session reads off the caller (sync loads in `crates/hoocode-code-session/src/manager.rs`) | Session | measure first: caller stall during a long session load | M | `concurrency.md:177` (phase 3) |
| O-9 | Compaction LLM call off io; compaction CPU off tools | Agent loop | none (correctness, rule 2) | M | Review 2026-11-06 |
| O-10 | M4 close-out: streaming re-lex (decisions-2026-10-29 #6). Proposed close, pending callgrind | TUI | measure first: callgrind on a streaming session; close if under 25% | S | `perf-short-sessions.md:299-316`; decisions-2026-10-29 #6 |

Build order, as approved:

1. P-R0.
2. P-C4, P-M4, P-M3, P-C3, P-X1, P-X2, P-X3, P-S5.
3. P-C1, then P-M6.
4. O-1 (Step 0 baseline and measure), then P-S1, P-S2, P-S3, P-M1.
5. P-C2, only if the measurement shows it pays.

The rule debt (O-5, O-6, O-7, O-9) follows step 3. The other measure-first O- items (O-2, O-3, O-4, O-8, O-10) run with O-1 and get a place when their gate is met.

### Details

Exact designs from the 2026-11-06 review (v2). The rules, guards and tests are as approved. Do not soften them.

**P-R0 Responses tool-call routing** (`crates/hoocode-ai-provider-openai-responses/src/shared.rs:597-786`)

1. An event with a non-empty `item_id` routes to its open slot, else it is a no-op. Never fall to legacy.
2. The legacy open-block path runs only when `item_id` is absent.
3. `output_item.added` for a closed (id, call_id) is a no-op. The same id with a new call_id is a new call.
4. `output_item.done` prefers a non-empty `item.arguments` over the delta buffer.
5. A late `arguments.done` after `item.done` changes no content and no event.
- Tests: events for an id after its done; replayed add after done; unknown id.

**P-C1 Approvals stop freezing the batch** (`crates/hoocode-agent-loop/src/lib.rs:880-970`)

- Gate and before-hook run via `spawn_blocking`, awaited as a branch of the batch select loop.
- One prompt outstanding, in call order.
- Turnstile ordinal and `ParallelToolLimit` slot taken on approval.
- Approved calls start and are polled while the next prompt is open.
- Abort cancels the open dialog (clearance-style flag) = Deny.
- No trait change.
- Keep: config re-read per check (Always must cascade within a batch).

**P-C2 Speculative read-only start** (narrow; measure first)

- Only when: read-only tool, not `ordered_start`, no before or after hook, gate allows at End.
- Takes a `ParallelToolLimit` permit; result buffered.
- Start and End events are emitted after `message_end`, and only if `stop_reason == ToolUse`.
- On Error, Aborted or Length: cancel, discard.
- Helps Anthropic and Responses (calls before the last block). OpenAI chat: none.

**P-C3 One process-wide HTTP client** (`crates/hoocode-ai-util/src/tls.rs:254`, `http_client_builder`)

- One `OnceLock` reqwest client. Timeout per request.
- No rebuild key needed (trust set is set once in main). Proxy env read at first use.
- No http2. Codex websocket and agent-mcp (reqwest 0.13) out of scope.

**P-C4 `--continue` picks the newest valid session**

- List `.jsonl` files with mtime, sort newest first, validate in order, stop at the first valid one.
- `settle_session_writes` once.
- First line read with a size cap.
- Follow-up: header rewrite via temp file and rename (`fs::write` truncates).

**P-S1 `parse_streaming_json`** (`crates/hoocode-ai-util/src/lib.rs:199`)

- Compute `repair_json` once. Byte scan instead of HashSet and `Vec<char>`.
- Output identical (repair tests and parity table).
- No throttling. No skip-on-plain-char.

**P-S2 Responses text** (`crates/hoocode-ai-provider-openai-responses/src/shared.rs`)

- Drop the mirror into `current_item`. Guard on `item_type()=="message" && block_is("text")`.
- Behaviour change: deltas without `content_part.added` now render. Test it.

**P-S3 Child JSON path** (`crates/hoocode-code-subagents/src/live.rs`)

- Match `MessageUpdate` before `to_json`. Text deltas go straight to the coalescer. Other updates flush.
- Identical child stdout.
- No wire change for RPC or print json (pinned by code-print `json_events.rs` and the replay harness).

**P-S5 TUI: `MessageUpdate` does not set dirty**

- `MessageUpdate` does not set dirty. The throttled `run_streaming_render` does.
- Status line compares working time in whole seconds.

**P-M1 `transform_context`**

- Signature: `transform_context: (&[AgentMessage], signal) -> Result<Option<Vec<AgentMessage>>>`.
- `convert_to_llm` takes a slice. 2 copies become 1 per model call. Job notices are appended after.
- Covers the open item "transcript copies in subagent child" (about 16x transcript at peak).

**P-M3 `SessionManager` `by_id`**

- `by_id` maps id to an index into entries. Rebuilt at load, new, and branch rewrite.

**P-M4 `message_count()` and `has_messages()`**

- Two methods. No closure API (the lock is non-reentrant).

**P-M6 Tool progress**

- Latest progress per call id. The final drain before `ToolExecutionEnd` is kept.
- The stream queue stays unbounded (a sync push can deadlock on current_thread).
- Includes the open item "tool progress keep-latest" for the agent loop (`concurrency.md:119`). MCP already does it.

**P-X1 CodeSearch** (`crates/hoocode-code-tool-search/src/rerank.rs:51`)

- Compile the `declares_term` regex pair once per term per query.

**P-X2 UserMessageComponent**

- Render stamp at construction.

**P-X3 ShellOutput wait**

- Condvar on exit, plus a bounded wait slice for abort.

## Deferred

| Item | Why | Decision | Revisit when |
|---|---|---|---|
| Whole-transcript re-render, `into_lines` (`crates/hoocode-tui-render/src/component.rs:130`) | Long-session cost only. Frame p50 0.73 ms at turn 30 | 2026-10-29 #2 | A long-session target is set. The 300-turn run is the reference |
| One copy of rendered lines (`crates/hoocode-tui-render/src/tui.rs`, previous lines) | Long-session cost only | 2026-10-29 #2 | As above |
| Lazy model catalog (`crates/hoocode-ai-models/src/lib.rs:18-21`; `crates/hoocode-code-models/src/lib.rs:387-401`). Takes C5 (worker parse) too | About 2.6 MiB per child; about 31 MiB at 12 children. Registry 0.71 MB; merged copy 0.75 MB | 2026-10-29 #2; C5 folded 2026-11-06 | Startup must shrink, or child memory is the limit. First candidate |
| Regex cache LRU (`crates/hoocode-tui-highlight/src/engine.rs:43`, unbounded) | Not measured with many languages. Size unknown | 2026-10-29 #2 | A session with many languages shows growth |
| Allocator swap (jemalloc, mimalloc) | jemalloc: RSS +9%, CPU -9.5%. One run each | 2026-10-29 #2 | Try `MALLOC_ARENA_MAX` first (O-4). A new dependency needs a clear win |
| Binary size beyond strip | Strip alone saved about 14 MiB (`Cargo.toml:205-206`) | 2026-10-29 #2 | Binary size becomes a target |
| Phase 6: frame build off the UI thread | Trigger met. Burst tail p99 100 to 136 ms | User deferred (`README.md` row 2) | The user asks |
| Thread per subsystem; TUI tree off main; priority above normal; shell memory limits; in-process subagents; rayon pool; async sync-tools; background compaction | Listed as deferred in `concurrency.md` 452-466 | `concurrency.md` 452-466 | The phase that needs it is picked up |

## Not doing

| Item | Reason | Source |
|---|---|---|
| M3 lean child (gate 8 MiB) | Measured 2.6 MiB per child. Gate not met | `perf-short-sessions.md:145-150`; 2026-10-29 #5 |
| Child io workers from 2 to 1 | Saves under 0.3 MiB | `perf-short-sessions.md:270-271` |
| Theme lookup `Arc` (TUI Phase S, L3) | No measured cost. Phase S L1 to L4 skipped | `README.md` row 7; 2026-10-15 |
| Geometry (TUI Phase S, L4) | No measured cost | `README.md` row 7 |
| M2: one copy per message, not per delta | Only if measured. No cost shown | v2 dropped |
| M5: running totals | Two sources can diverge. Cache by (leaf_id, generation) only if measured | v2 dropped |
| S4: loop coalescing | The loop stays per delta, since RPC, json and app-server need it. Coalesce per consumer (P-S3, P-S5) | v2 dropped |
| Permission config once per batch | Breaks the Always cascade within a batch | v2 dropped |
| App-server registry cache | Per-RPC reload is the live reload | v2 dropped |
| HTTP/2 | Not compiled in today (reqwest `default-features = false`) and untested behind intercepting proxies. The gain from P-C3 is connection reuse, which HTTP/1.1 keeps | Review 2026-11-06 |
| Bounded stream queue (1024 events), for now | A sync push cannot wait; it deadlocks on current_thread. See Open question 2 | Review 2026-11-06 |
| Wire change on text deltas | RPC and print json clients see the same bytes. Pinned by `json_events.rs` and the replay harness | v2 dropped |
| C5 worker parse | Folded into the deferred lazy model catalog | v2; 2026-11-06 |

## Open questions

1. **C2 (P-C2): keep as a measured, optional item, or drop it?** Recommend: keep, gated. It helps Anthropic and Responses only (calls before the last block). Risk: more code in the batch path. Results are discarded on error, abort or length.
2. **Stream queue cap: 1024 events (`concurrency.md:118`), or keep unbounded?** Recommend: keep unbounded until there is an async-permit design, and amend `concurrency.md`. Risk: a stalled consumer grows memory without a limit. A cap now deadlocks a sync push on current_thread.
3. **io blocking pool cap, against rule 2?** Recommend: keep the 2026-10-15 decision (uncapped). Revisit when child `wait()` and the Agent body are async. Risk: unbounded threads under a heavy tool load. The thread ceiling (target 26) is then the only check.
4. **Phase 6: frame build off the UI thread?** Recommend: keep deferred (the user's decision). Risk: the burst tail (p99 100 to 136 ms) and the one unexplained keystroke miss stay.
5. **`MALLOC_ARENA_MAX=2` in subagent children, after a CPU A/B?** Recommend: try it (O-4). Trade-off: saves about 1.2 MiB per child; the CPU cost is unknown until measured. Risk: a CPU cost in every child.

## Doc drift

| Page:line | Says | Code says |
|---|---|---|
| `concurrency.md:23` | hoocode-io has one worker per core | min(4, cores) workers; 2 in children (`crates/hoocode-runtime/src/runtime.rs:21-23,38-44`) |
| `concurrency.md:24-25` | A second runtime in ai-stream; a new runtime per HTTP MCP server | Both removed |
| `concurrency.md:35` | No priorities and no memory watch | Lanes and watchdog exist (`lanes.rs`, `watchdog.rs`) |
| `concurrency.md:147` | clippy `disallowed-methods` forbids `block_on` | `clippy.toml` does not list `block_on`; 13 sites remain outside tests |
| `concurrency.md:250` | clippy does not forbid `thread::spawn` | `clippy.toml:4-9` forbids it |
| `concurrency.md:487` | Diagram: subagents at most 5 | Root 12, nested 6, depth 4 (`pool.rs:460`) |
| `concurrency.md:71`, `:503` | Version check runs on background | Runs on io (`crates/hoocode-code-cli/src/runtime.rs:2074`) |
| `concurrency.md:94-95` | `migration/dep-firewall.json` | File does not exist. `scripts/ci/check_dep_firewall.py` is the firewall |
| `concurrency.md` thread map | Lists the thread owners | Missing `hoocode-progress`, `hoocode-clearance`, `hoocode-cli-worker` |
| `crates/hoocode-code-tool-api/src/truncate.rs:16` and `crates/hoocode-agent-harness/src/utils/truncate.rs` | Two truncate defaults | 32 KiB and 50 KiB. Pick one |
| `perf-short-sessions.md` code refs | e.g. `pool.rs` child_env at line 1481 | Now `pool.rs:1497`. Many line refs have drifted |
| `tui-activity.md:352-356` | Shared partial messages rejected | `SharedMessage` built (`crates/hoocode-ai-types/src/lib.rs:785-848`) |
| `tui-activity.md:346-347` | `render_railed` symbol | Symbol not found in code |
