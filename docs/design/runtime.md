# Runtime

Status (2026-11-06): built: one io runtime, three lanes, the caps below, watchdog, ordered shutdown, panic hook, rpc fail-closed, fuzz targets, in-process `@file`. Open: 38 items, RT-1 first. Load numbers are Linux only.

## Principles

| # | Rule | Enforced by |
|---|---|---|
| 1 | Build threads, runtimes and channels only in `hoocode-runtime`. | `clippy.toml` disallows `Runtime::new`, `Builder::new_*`, `thread::spawn`, `Builder::spawn`. Gap: unbounded channels (RT-13). |
| 2 | The UI thread never blocks: no file or network I/O, no `block_on`, no wait on another lane's lock. | Source-scan test over `crates/hoocode-code-tui-app/src/mode/` (`session_op.rs`, about line 197). See the block_on rule. |
| 3 | No blocking call on a `hoocode-io` worker. Blocking work goes through `run_blocking` or `spawn_blocking`. | Review. Gap: tool bodies use the uncapped io blocking pool (see How it works). |
| 4 | Every queue states its cap and policy. Every wait on something outside the process has a deadline. | Session writer (4096 entries or 64 MiB, producer waits). MCP 60 s deadline. Gaps: LLM stream queue (RT-13), LLM first-byte and idle deadlines (RT-14), SSE cap (RT-12). |
| 5 | No lock held across `.await` or while calling listeners. | Review. Agent listeners run on a copy, outside the lock. |
| 6 | Session data is never dropped. Its queue applies backpressure and is flushed before exit, fork and export. | `crates/hoocode-runtime/src/session_io.rs` |
| 7 | OS priority only goes down, never up. | Lanes at nice 0, +5, +10 (`lanes.rs`). Linux only so far (RT-37, RT-38). |
| 8 | Nothing kills hoocode itself. Memory pressure sheds load or aborts a turn. The child lifeguard kills children only. | Watchdog, shed and abort paths. Hard-limit abort untested (RT-34). |
| 9 | Four performance settings. Every other cap is fixed in code. | `crates/hoocode-code-settings/src/manager.rs` (`performance_*`) |
| 10 | Measure first; a speed or memory change needs a number before it lands. A correctness fix does not wait. Optimise for sessions of 10 to 15 turns. | Gates in Open. |
| 11 | A panic on user or model input is a bug. | `text_slice` for string slices; panic hook logs the thread name. |

### The block_on rule

- The UI thread (`main` and the TUI modules in `hoocode-code-tui-app`) never calls `block_on`.
- `block_on` is allowed only at entry points (`main`, the CLI's `async_runtime().block_on`, tests) and in the runtime's own sync bridges (`crates/hoocode-runtime/src/runtime.rs`). Other non-test sites are listed in RT-27.
- `clippy.toml` does not forbid `block_on` or `unbounded_channel`. The guard is the source-scan test only.

## How it works

### Lanes and thread map

| Thread / pool | Count | Lane | Does |
|---|---|---|---|
| `main` (UI) | 1 | High | TUI state. Turns events into actions and builds frames. |
| `hoocode-input` | 1 | High | Reads stdin, parses keys, ESC deadline, Ctrl+C fast path. |
| `hoocode-term-out` | 1 | High | Only terminal writer. One pending frame; the UI builds the next only after the writer takes it. |
| `hoocode-io` workers | `min(4, cores)`; 2 in children | High | Agent loop, LLM streams, MCP clients, rpc and app-server transports, `Shell` pipes. |
| `hoocode-io` blocking pool | uncapped (tokio default, 512) | High | Tool bodies (Read, Edit, Agent, MCP) and child `wait()`. Decided 2026-10-15. |
| `hoocode-tools` pool | up to 16 | Medium | Only `run_blocking` users: attach, MCP elicitation, session wait. |
| `hoocode-session-io` | 1 | Medium | Only writer of session files. Keeps the file open, batches appends, flushes at turn end. |
| `hoocode-bg` | 1 | Low | Housekeeping: prune dispatch dirs and temp files, session index, version check, file watchers. Own current-thread runtime. |
| `hoocode-watchdog` | 1 | High | Heartbeats, stall log, RSS sample, starvation probe. |
| `hoocode-resize` | 1 | none | SIGWINCH listener on Unix. A 100 ms size poll on Windows, or if the listener fails. |
| `autocomplete-files` | 1 per `@` provider | none | In-process `@file` walk, newest query only. Exits with its provider. |
| `hoocode-perf` | 0, or 1 with `--perf-log` | none | Once a second reads `/proc`, appends a JSON line. Without the flag, `/perf` reads `/proc` once, inline. |
| `hoocode-progress`, `hoocode-clearance`, `hoocode-cli-worker` | 1 each (clearance per subagent pool) | not set | Timer in `tui-terminal`; clearance relay in the parent (`code-subagents`); CLI worker (`code-cli`). |
| Subagent children | processes; 12 root, 6 nested | Low (nice +5) | Own `hoocode-io` (2 workers). Pipes read by io tasks, so no thread on Unix. |
| `Shell` children | processes | nice 0 (`bashNice`) | Pipes read by io tasks; chunk queue 256. |

- Steady state: about 12 threads (13 with `--perf-log`).
- Thread ceiling under load: 26, parent process only. The 2026-10-15 load run peaked at 24 (34 before the subagent pipe fix). The 11 blocking-pool threads at peak are watched, not capped.

### Lane OS priority

| Lane | Linux | macOS | Windows |
|---|---|---|---|
| High | nice 0 | QoS `USER_INTERACTIVE` | `THREAD_PRIORITY_NORMAL` |
| Medium | nice +5 | QoS `UTILITY` | `BELOW_NORMAL` |
| Low | nice +10 | QoS `BACKGROUND` | `LOWEST` |
| Subagent children | process nice +5 | process nice +5 | `BELOW_NORMAL_PRIORITY_CLASS` |
| `Shell` children | nice 0 or `bashNice` | nice 0 or `bashNice` | normal; `bashNice` above 0 gives below-normal |

A priority that fails to set is logged once, and the work carries on. Child processes get their priority at spawn (`pre_exec` on Unix, a creation flag on Windows).

### Caps (code values)

| Resource | Cap | When reached |
|---|---|---|
| `hoocode-io` workers | `min(4, cores)`; children 2 | Tasks queue |
| `hoocode-tools` pool | 16 threads | Calls queue |
| Parallel tool calls per turn | `maxParallelTools` 8 (1 to 32) | Rest wait, in call order |
| Background tool managers per session | 64 | Oldest evicted |
| Tool abort grace | 250 ms (Agent tool 5 s) | Loop writes the call's result without waiting. |
| Subagents, root / nested | `subagentConcurrency` 12 (1 to 32) / `nestedSubagentConcurrency` 6 | Queue |
| Subagent depth | `maxSubagentDepth` 4 | Dispatch refused |
| Subagent dispatch queue | `subagentQueuePerSlot` 8 per slot | Refused |
| Subagent child RSS | `subagentMaxRssMb` 4096 MiB | Lifeguard: SIGUSR1 (stall), SIGTERM (deadline), grace, SIGKILL |
| Subagent deadline, stall, tool stall | 240 min; 1200 s; 1800 s | Child ends |
| In-flight requests per MCP server | 8 | Wait for a permit |
| MCP response body; request deadline | 32 MiB; 60 s | Error result |
| SSE line or event (MCP Streamable HTTP only) | 16 MiB | Stream error |
| `Shell` output chunk queue | 256 | Reader waits |
| Tool output in model context | 32 KiB, 800 lines (`code-tool-api` `truncate.rs`) | Spill to file |
| LLM events to UI | unbounded today (target 1024, not built) | RT-13 |
| Tool progress to UI | unbounded today; keep-latest only in MCP | RT-11 |
| Frames to `hoocode-term-out` | one pending; control writes queue 1024 | UI waits for the writer |
| Session write queue | 4096 entries or 64 MiB | Producer waits. Never drops. |
| `@file` results | 100 found, 20 shown | Cut off |

### Four performance settings

| Setting | Default | Range | Notes |
|---|---|---|---|
| `performance.maxParallelTools` | 8 | 1 to 32 | Applied at launch in the agent loop |
| `performance.memorySoftLimitMb` | min(2048, 25% of RAM); 2048 if RAM unknown | 256 up; 0 off | |
| `performance.memoryHardLimitMb` | min(4096, 50% of RAM) | above soft; 0 off | |
| `performance.bashNice` | 0 | 0 to 19 on Unix; Windows 0 or above 0 | Lowers `Shell` children |

Memory limits are on by default.

- Soft limit reached: parallel tools drop to 1, no new subagent dispatches, footer warning. Recovers when memory is 10% under the limit. The render-cache clear is not built (RT-33).
- Hard limit reached: abort the turn, flush the session, show a notice (path untested, RT-34).

### Watchdog, shutdown, emergency exit

| Item | Value |
|---|---|
| Watchdog tick | 250 ms. RSS sampled every 4 ticks (1 s). |
| UI silent | 500 ms: log the UI phase once. 2 s: footer flag. |
| Starvation probe | every 1 s; late by over 250 ms = io runtime starved. Sees only total starvation. |
| Ordered shutdown | 3 s: abort 0.8 s, kill tool groups 0.1 s, flush 1.0 s, MCP close 0.7 s, release 0.2 s; terminal restore last, in the 0.2 s reserve |
| Emergency exit | Two Ctrl+C within 1 s with the UI silent 500 ms: restore terminal, flush (1 s), exit 130 |
| Ctrl+C abort | Paused-pty run: 30 ms to reach the HTTP client (target 500 ms) |

### Idle wakeup budget

| Source | 2026-10-29 (before) | 2026-10-30 (after) |
|---|---|---|
| `hoocode-watchdog` | 9.9 per s | 3.9 to 4.0 per s |
| UI loop | 4.0 per s | 1.0 per s (idle cap 1 s) |
| `hoocode-io` worker | 2.3 per s | 2.0 to 2.2 per s; source not traced (RT-23) |
| `hoocode-perf` | 1.0 per s | 0 unless `--perf-log` |
| `hoocode-bg` (git repo only) | not measured | 2.0 to 2.1 per s; the 1 s git poll explains 1 |
| Total, plain directory | 17.2 per s | 6.9 to 7.2 per s |
| Total, git repo | not measured | 8.9 to 9.3 per s |
| Idle CPU | 0.20% (10 s), 0.24% (25 s) | 0.16 to 0.2% |

- Budget: 7 idle wakeups per s, accepted. 5 per s is the aspiration, not met. Idle CPU under 0.2%.
- Rule: every idle deadline is in `next_wakeup`. A new idle timer states its deadline.
- Idle-only notices (extension UI requests, update notice, memory and stall flags) can show up to 1 s late. Accepted.

### Release profile

```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
# panic stays "unwind": run_blocking turns a panic into Err through JoinError.
```

- Stripped binary 34.86 MiB (acceptance: at or below 43 MiB, met). First prompt 31 to 34 ms (was 42 to 54 ms).
- `--version` median 5.29 ms against 3.8 ms before. Not yet an A/B. Build cost is not isolated (RT-22).
- Stripped builds lose function names in crash backtraces. Unstripped artifact is RT-22(d).

### Reliability rules

| Rule | Detail | Where |
|---|---|---|
| rpc fails closed | In `--mode rpc` a tool that needs approval is blocked unless `auto_allow` lists it. MCP and plugin lifecycle tools (`SearchPlugins`, `ListPlugins`, `SuggestPluginInstall`, `InstallPlugin`, `ProposePlugin`) are gated too. Approval dialogs over rpc are dropped; use the app-server. | `code-permissions` `evaluate`; `code-cli` `build_permission_gate` |
| Children and approvals | Children inherit `HOOCODE_INTERNAL_APPROVALS_FAIL_CLOSED`; warm workers opt out with `HOOCODE_INTERNAL_WARM_WORKER` unless the parent is closed. A `--mode json` child with `--task-id` relays each approval to its parent. No UI, EOF, cancel or deadline: deny. | `code-subagents` `clearance.rs` |
| Print and json | Allow, with one stderr line after the run naming the approval tools that ran. | `code-main` `approval_note.rs` |
| Data directory | `~/.hoocode` (`agent_dir()`). `HOOCODE_` is the only env prefix. No old-name aliases, no migration. MCP trust store stays here. | `hoocode-code-paths` |
| `.agents` scope | Read-only: `~/.agents` and `<project>/.agents` (skills, commands, agents, AGENTS.md, MCP config). Writes only `<project>/.agents/scheduled_tasks.json` and its `.lock`. | `hoocode-code-paths` |
| home directory | `$HOME` if set and non-empty, else the passwd entry. `None` if neither. Never `/`: callers leave the path alone. | `hoocode-home` `home_dir` (other copies: RT-36) |
| String slices and unwraps | Every slice in TUI and tool crates goes through `text_slice` (`prefix`, `suffix_from`, `range`); `clippy::string_slice` is on. Unwraps are fixed only where reachable from user text, model output, files or network. | `hoocode-tui-util` `text_slice.rs` |
| Fuzz targets (9) | `sse`, `rpc_jsonl`, `session_jsonl`, `settings_json`, `models_json`, `tui_keys`, `js_regex`, `ansi_wrap`, `markdown`. Smoke runs on stable as `fuzz_smoke` in each owning crate. Longer run nightly. | `fuzz/fuzz_targets/`; `.github/workflows/fuzz.yml` |
| `@file` autocomplete | In-process walk with the `ignore` crate, fd's rules: hidden included, links followed, `.gitignore` honoured, `.git` skipped, smart case. Off the UI thread, newest query only. No `fd`, no `rg`. | `hoocode-code-tools` `file_finder`; `tui-components` `autocomplete/file_search.rs` |
| macOS | A macOS test failure is a bug. The `test-macos` job runs on pull requests. | `.github/workflows/ci.yml` |

### Measurement

- `/perf` and `--perf-log <file>`: threads per lane, queue depths, frame build and write time, keystroke-to-frame latency, UI stalls, RSS.
- Load scenario `scripts/perf/load_scenario.py`: streaming mock, 8 parallel `Shell` calls, 5 subagents, a stdio MCP server that never answers (`scripts/perf/mcp_stall_server.py`), keys throughout, plus a paused-pty run.
- Forced memory test: `scripts/perf/alloc_shim.c` (test only, `LD_PRELOAD`, acts only on a process named `hoocode` with `HOOCODE_PERF_ALLOC_MB` set).
- Mock model `scripts/tui/mockllm.py`; subagent evals `scripts/eval/subagent_evals.py --include-slow`.

### Baseline

Measured, as recorded. Linux only. RT-16 adds real 10 and 15 turn runs.

| Measure | Value | When |
|---|---|---|
| Binary, stripped | 43.1 MiB on the plain profile; 34.86 MiB on the release profile | 2026-10-29 / 10-30 |
| `--version` | 3.8 ms, 12.2 MiB RSS before; 5.29 ms after (no A/B) | 2026-10-29 / 10-30 |
| TUI first prompt RSS | 23.2 to 24.5 MiB | 2026-10-29 |
| 300 turns | RSS 85.0 MiB, +0.17 to 0.18 MiB per turn; frame p50 0.73 ms at turn 30, 2.37 ms at turn 300; p99 12.5 ms at turn 300 | 300-turn run |
| 10 to 15 turns | RSS about 33 to 36 MiB and frame p50 about 0.7 ms are derived from the 300-turn run, not measured | RT-16 measures them |
| Subagent child, late | RSS 25.7 MiB, private 7.5 MiB, heap peak 4.4 to 4.6 MiB; 12 children: 339.6 MiB RSS sum, 124.1 MiB PSS sum | 2026-10-30 |
| Load, keystroke p99 | 5.2 ms (bash, 300 keys); 3.8 ms (load, 975 keys); one 27.3 ms miss at load average 3.4 | 2026-10-15 |
| Burst frame build | p50 0.7 ms, p99 136 ms (bash block) | 2026-10-15 |
| Thread peak | 24 (34 before the fix) | 2026-10-15 |
| Forced 3 GiB allocation | RSS peak 3,095 MiB; shed notice appeared and lifted; hard limit 4,096 MiB not reached | 2026-10-15 |
| Lazy model catalog | About 2.6 MiB per child | 2026-10-29 |

## Decisions

| Date | Decision |
|---|---|
| 2026-10-08 | Threads where latency matters, tasks for I/O, processes for isolation. No thread per subsystem, no GC thread. |
| 2026-10-08 | One tokio runtime per process, `min(4, cores)` workers; 2 in children. Built only in `hoocode-runtime`. |
| 2026-10-08 | Three lanes (High, Medium, Low). The UI never blocks. Ctrl+C is handled on the input thread. |
| 2026-10-08 | OS priority only goes down. Session data is never dropped. |
| 2026-10-08 | Queues bounded with a stated policy, except the LLM stream queue (see 2026-11-06). |
| 2026-10-08 | Memory limits on by default. Soft sheds load; hard aborts the turn. Nothing kills hoocode itself. |
| 2026-10-08 | Agent `Shell` commands run at nice 0; `performance.bashNice` lowers them. Compaction and cleanup are work, not threads. |
| 2026-10-08 | Four performance settings; every other cap fixed in code. `/perf` and `--perf-log` show the numbers. |
| 2026-10-08 | rpc fails closed for tools that need approval. Data in `~/.hoocode`; `HOOCODE_` is the only env prefix. `@file` walks in process, off the UI thread; no `fd`, no `rg`. |
| 2026-10-09 | `home_dir`: `$HOME` if set and non-empty, else passwd. Never `/`. |
| 2026-10-14 | MCP and plugin lifecycle tools in rpc are fail-closed unless `auto_allow` lists them. |
| 2026-10-14 | `.agents` is read-only, except the scheduled-task store. |
| 2026-10-15 | The io blocking pool stays uncapped: a cap can deadlock a child `wait()` queued behind an Agent body. Revisit when both are async. |
| 2026-10-15 | Subagent pipes are read by io tasks, not threads (Unix). |
| 2026-10-29 | Optimise for sessions of 10 to 15 turns. Long-session items deferred. Measure first; M3 and M4 are gated. |
| 2026-10-29 | Subagent caps stay at 12 root, 6 nested, depth 4. |
| 2026-10-30 | Release profile (thin LTO, one codegen unit, strip symbols, unwind) built. |
| 2026-10-30 | Idle wakeups: perf sampler only with `--perf-log`; watchdog tick 250 ms; git poll 1 s; UI idle cap 1 s. |
| 2026-10-30 | Accept about 7 idle wakeups per s. Lean subagent child rejected (2.6 MiB measured, gate 8 MiB). |
| 2026-11-06 | LLM stream queue stays unbounded until there is an async-permit design. |
| 2026-11-06 | Build order approved (v2), P-R0 first. Old C5 worker parse folded into the lazy model catalog. Performance status lives in this file. |

## Open

Size: S about a session, M a few, L many. IDs in brackets are the old performance IDs.

Build order, as approved 2026-11-06: step 1 RT-1; step 2 RT-2 to RT-9; step 3 RT-10, RT-11; rule debt RT-12 to RT-15; step 4 RT-16 to RT-20; step 5 RT-21, only if measured. RT-22 to RT-26 are measure-first and run alongside RT-16. RT-27 onward have no approved order; each has its own trigger.

| ID | Item | Size | Trigger or gate | Design notes |
|---|---|---|---|---|
| RT-1 [P-R0] | Responses tool-call routing fix | M | None (correctness) | `ai-provider-openai-responses` `shared.rs`. (1) A non-empty `item_id` routes to its open slot, else no-op; never fall to legacy. (2) Legacy open-block path only when `item_id` is absent. (3) `output_item.added` for a closed (id, call_id) is no-op; same id with a new call_id is a new call. (4) `output_item.done` prefers non-empty `item.arguments` over the delta buffer. (5) A late `arguments.done` after `item.done` changes nothing. Tests: events after done; replayed add after done; unknown id. |
| RT-2 [P-C4] | `--continue` picks newest valid session | S | None | Partly built: `code-session` `manager.rs` sorts by mtime but validates every file. Left: stop at the first valid one; cap first-line read; `settle_session_writes` once; header rewrite by temp file and rename. |
| RT-3 [P-M4] | `message_count()` and `has_messages()` | S | None | Two methods. No closure API (the session lock is not reentrant). |
| RT-4 [P-M3] | `SessionManager` `by_id` index | S | None | `manager.rs:183` is `HashMap<String, FileEntry>`. Map id to an index into entries; rebuild at load, new, and branch rewrite. |
| RT-5 [P-C3] | One process-wide HTTP client | S | None | `ai-util` `tls.rs` `http_client_builder`. One `OnceLock` reqwest client; timeout per request; proxy env read at first use. No HTTP/2. Codex websocket and agent-mcp out of scope. |
| RT-6 [P-X1] | CodeSearch: compile term regexes once per term | S | None | `code-tool-search` `rerank.rs:51`. |
| RT-7 [P-X2] | UserMessageComponent: render stamp at construction | S | None | `code-tui-widgets` `user_message.rs:63-65`. |
| RT-8 [P-X3] | `ShellOutput` wait: Condvar on exit | S | None | Condvar on exit plus a bounded wait slice for abort. No Condvar in `code-tool-bash` today. |
| RT-9 [P-S5] | `MessageUpdate` does not set dirty | S | None | `code-tui-app` `mode/events.rs:426-432`. The throttled `run_streaming_render` sets dirty. Status compares working time in whole seconds. |
| RT-10 [P-C1] | Approvals stop freezing the batch | L | None | `agent-loop` `lib.rs:880-970`; the gate runs inline at about 1026 today. Gate and before-hook via `spawn_blocking`, awaited in the batch select loop. One prompt outstanding, in call order. Turnstile ordinal and `ParallelToolLimit` permit taken on approval. Approved calls start while the next prompt is open. Abort cancels the open dialog as Deny. No trait change. Keep: config re-read per check (Always cascades within a batch). |
| RT-11 [P-M6] | Tool progress: latest per call id | M | None | Agent-loop keep-latest (MCP has it, `mcp.rs:775-797`). Final drain before `ToolExecutionEnd` is kept. Stream queue stays unbounded (a sync push can deadlock on current_thread). |
| RT-12 [O-5] | SSE 16 MiB cap in provider decoders | S | None (correctness) | Only MCP Streamable HTTP has `MAX_SSE_EVENT_BYTES` (`agent-mcp` `client.rs:54`). Add it to provider SSE decoders; a bigger event is a stream error. |
| RT-13 [O-6] | Unbounded channels outside the runtime | M | None (correctness) | `agent-loop` `lib.rs:777,915`; `code-cli` `runtime.rs:1247`; `app-server` `server.rs:816-817`, `transport.rs:309`; `code-rpc` `client.rs:585`. Give each a cap and policy, or record why it stays unbounded. Std `mpsc::channel` sites in the TUI and auth are one-shot replies; review them in the same pass. Clippy does not forbid either today. |
| RT-14 [O-7] | LLM first-byte and idle deadline | S | None (correctness) | Today the only deadline is the total `options.timeout_ms`, when set (e.g. `ai-provider-anthropic` `lib.rs:124-126`). Add a first-byte and an idle-gap deadline; a timeout is a stream error. Review suggests moving this up beside RT-1; not approved, so the order stays. |
| RT-15 [O-9] | Compaction off io and tools | M | None (correctness) | LLM part on `hoocode-io`; CPU part on `hoocode-tools` via `run_blocking`. Compaction is not among the `spawn_blocking` sites today. |
| RT-16 [O-1] | Step 0: measured 10 and 15 turn baseline | S | None | See Measure first. Gate for RT-17 to RT-21. |
| RT-17 [P-S1] | `parse_streaming_json`: repair once, byte scan | S | Measure first: microbench and json-mode long reply | `ai-util` `lib.rs:199-215`. `repair_json` still uses `HashSet<char>` (`:79`) and `Vec<char>` (`:109`). Compute repair once; byte scan. Output identical (repair tests and parity table). No throttling, no skip-on-plain-char. |
| RT-18 [P-S2] | Responses text: drop the `current_item` mirror | S | Measure first (as RT-17) | `openai-responses` `shared.rs:341,373,404`. Guard on `item_type()=="message"` and `block_is("text")`. Behaviour change: deltas without `content_part.added` now render. Test it. |
| RT-19 [P-S3] | Child JSON: text deltas straight to the coalescer | S | Measure first (as RT-17) | `code-subagents` `live.rs:88,114-150`. Match `MessageUpdate` before `to_json`; text deltas go to the coalescer, other updates flush. Identical child stdout; no wire change. |
| RT-20 [P-M1] | `transform_context` takes a slice | M | Measure first: transcript copies in a child at peak (about 16 times the transcript) | Signature `(&[AgentMessage], signal) -> Result<Option<Vec<AgentMessage>>>`. `convert_to_llm` takes a slice: 2 copies become 1 per model call. Job notices appended after. `code-agent-session` `services.rs:413`. |
| RT-21 [P-C2] | Speculative read-only start | M | Measure first: share of read-only calls before the last block; wall time saved on Anthropic and Responses. Build only if it pays. | Only for a read-only tool, not `ordered_start`, no before or after hook, gate allows at End. Takes a `ParallelToolLimit` permit; result buffered. Start and End events after `message_end`, only if `stop_reason == ToolUse`. On Error, Aborted or Length: cancel and discard. OpenAI chat: no gain. |
| RT-22 [O-2] | P1 follow-ups | S | Measure first: CPU A/B, build time, `--version` time | (a) 100-turn mock CPU A/B, five runs each; accept if median CPU falls and median RSS is not more than 5% higher. (b) `--version` and `--help` A/B against the old binary (3.8 and 3.6 ms before). (c) Thin LTO build cost on a clean release build, same host. (d) Unstripped copy of each binary as a CI artifact in `binaries.yml`. |
| RT-23 [O-3] | `hoocode-io` and `hoocode-bg` wakeup trace | S | Measure first: wakeup trace | `hoocode-io` about 2 per s; the worker varies between runs. `hoocode-bg` about 2 per s in a repo; the git poll explains 1. Trace per-thread wakeups over 10 s idle. |
| RT-24 [O-4] | `MALLOC_ARENA_MAX=2` in subagent child env | S | Measure first: CPU A/B and subagent evals | `code-subagents` `pool.rs` `child_env` (about line 1497). Saves about 1.2 MiB per child (`=1` saves 1.7 MiB). Risk: malloc contention when tool calls run in parallel. Test: subagent evals and a CPU A/B of a 6-way parallel Read child. |
| RT-25 [O-8] | Session reads, `create_dir_all`, open and resume off the caller | M | Measure first: caller stall during a long session load | `code-session` `manager.rs` loads run on the caller; reads wait for queued writes (up to 1 s). Move reads to `hoocode-tools` with the result sent to the UI; move `create_dir_all` and open and resume to a lane. |
| RT-26 [O-10] | Streaming markdown re-lex: close-out | S | Measure first: callgrind on a streaming session; close if lexing is under 25% | `code-tui-widgets` `assistant_message.rs:348` (`segment_streaming_markdown`) already re-lexes only the last chunk unless link definitions change. See the M4 gate. |
| RT-27 | Remaining non-test `block_on` sites | S | Runtime item closes | To fix: `code-tool-bash` `jobs.rs:945`; `code-agent-session` `mcp.rs:780`; `code-subagents` `tools.rs:787` (`block_in_place`), `:875`, `:1513`; `code-tools` `lib.rs:488`; `code-auth` `lib.rs:506`. Check whether `ai-stream` `lib.rs:213,218` and `ai-util` `retry_delay.rs:519` are test-only. Then either add `block_on` to `clippy.toml` `disallowed-methods` with an allow per site, or keep the source-scan test and drop the clippy claim. |
| RT-28 | Input-thread panic restarts the reader | S | None | `tui-terminal` `stdin_hub.rs`: the reader starts once (`READER.call_once`), so a panic there stops keys reaching the UI. Restart the reader, or show a visible notice. |
| RT-29 | Default panic hook prints non-UI panics to stderr while the TUI runs | S | Real-tty check | Can draw over a frame. Send to the debug log while the TUI runs. |
| RT-30 | Real-TUI checks: UI stall with the loop paused; panic hook and terminal restore on a real tty | S | Before the runtime item is called done | Watchdog probes are proven only by `watchdog_forced.rs`. In the TUI the log lines need `HOOCODE_DEBUG_AGENT_LOG=1`; the footer flag is the user-visible signal. |
| RT-31 | Keystroke p99 under 16 ms: unexplained misses | M | A repeat of the miss | Six of seven bash runs passed after the fix. The miss was 27.3 ms at load average 3.4; an earlier 70.6 ms run is also unexplained. Repeat at load average under 1 and under load. If a miss recurs, capture frame and lane timing at that moment. |
| RT-32 | Frame build tail from the bash block | M | User asks (deferred by the user) | Burst p99 100 to 136 ms; p50 under 1 ms. Each slow frame draws the bash block; `visual_truncate` wraps the whole block before it truncates. Target: frame build p99 under 8 ms on a 10k-line transcript (not measured). Cut the block's render cost first; move big renders off the UI thread second. |
| RT-33 | Render-cache clear on memory shed | S | None | Not built (none found). The footer notice exists. Clear render caches when the soft limit sheds. The old performance table listed it as met; code says not built. |
| RT-34 | Hard-limit turn abort: test the path | S | None | Only the constant and the shed path are tested. Add a test: a forced allocation above the hard limit aborts the turn cleanly, flushes the session, and shows a notice. |
| RT-35 | Tool output truncation: one default | S | None | Live: 32 KiB, 800 lines (`code-tool-api` `truncate.rs:15-16`). A 50 KiB copy sits in `agent-harness` `utils/truncate.rs`. Confirm it is unused, then delete it. |
| RT-36 | One `home_dir` | S | None | `hoocode-home` is the policy. `hoocode-code-paths` wraps it and falls back to `.`. `hoocode-code-resources` `package_resolve.rs:156` has its own copy. Make every copy call `hoocode-home`. |
| RT-37 | macOS: baseline measurement and lane QoS | M | A macOS machine | The load run is Linux only; macOS QoS for lanes is untested. The first green `test-macos` run on a pull request is still to confirm. |
| RT-38 | Windows: `Shell` priority | S | Windows ships (not planned) | Process-wrap's JobObject overwrites creation flags, so `bashNice` may not apply. Set the priority after the job is created. |

### Measure first

These are gates. Each names what to measure and the result that proposes a change.

**Step 0 (RT-16): 10 and 15 turn baseline.**

- Runs: one TUI session each of 10 and 15 turns on `scripts/tui/mockllm.py` (`--chunk-size 40`, 10 ms delay). Five runs each for acceptance numbers.
- Metrics: VmRSS every 0.5 s from `/proc`; CPU from `/proc/<pid>/stat`; frame p50 and p99 from `--perf-log`; idle wakeups per thread from `/proc/<pid>/task/<tid>/status`.
- Plain profile for comparison: `CARGO_PROFILE_RELEASE_LTO=false`, `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`, `CARGO_PROFILE_RELEASE_STRIP=none`.
- Record: turns, RSS at end, CPU per turn, frame p50 and p99, wakeups per s. Replace the derived figures.
- Gate: RT-17 to RT-22 are judged against these numbers, not the 300-turn run. Mock turns are paced by the mock, so CPU is comparable across runs, not a real-model figure.

**M3 (child footprint).** A lean child (no UI, highlighter or catalog init) must save 8 MiB or more per child, in steady RSS and in massif peak.

- Result 2026-10-30: 2.6 MiB steady RSS, 2.8 MiB massif peak. Gate not met; not proposed.
- The child's heap peak is 4.4 to 4.6 MiB. The model catalog is the only unused subsystem that costs memory. Revisit only if the catalog becomes lazy.
- Changing `DEFAULT_MAX_CONCURRENCY` is a user decision. Memory does not argue for a change.

**M4 (streaming re-lex).** Lexing must be 25% or more of streaming work.

- Method: one mock turn streaming a 20 KB answer (`mockllm.py --chunk-size 8`). Count instructions with `valgrind --tool=callgrind` over the streaming window; compare inclusive cost of `lexer::lex` with the whole process. Use instruction counts, not wall time.
- At 25% or more: propose incremental lexing (keep finished blocks' tokens; lex only the open tail). Under 25%: close (RT-26).

## Not doing

- A thread per subsystem (MCP thread, per-tool threads, a GC thread). Rust frees memory on drop.
- Moving the TUI component tree off the main thread. About 60 files use `Rc<RefCell<...>>`.
- Raising any priority above normal, or real-time scheduling.
- Hard memory limits on user `Shell` commands. `RLIMIT_AS` breaks Node, the JVM and Go; cgroups are Linux only.
- In-process subagents by default. Children stay processes.
- A rayon or work-stealing CPU pool up front. Rewriting the sync tools as async.
- Background compaction while typing. It changes what the agent does; it gets its own card later.
- Capping the io blocking pool (decision 2026-10-15). Revisit when child `wait()` and the Agent body are async.
- A bounded stream queue (1024 events) for now. A sync push cannot wait on current_thread. Revisit with an async-permit design.
- HTTP/2. Not compiled in (reqwest `default-features = false`), untested behind intercepting proxies. Connection reuse comes from HTTP/1.1.
- A lean subagent child (M3 gate not met). Child io workers from 2 to 1 (saves under 0.3 MiB).
- Copy-once message buffers, running totals, loop coalescing, permission config once per batch, an app-server registry cache, and wire changes on text deltas. Each was dropped for lack of a measured cost or because it breaks a rule.

Deferred, with the trigger that revisits each:

- Whole-transcript re-render (`tui-render` `component.rs:130`) and one copy of rendered lines (`tui.rs`). Cost is linear in history. Revisit when a long-session target is set; the 300-turn run is the reference.
- Lazy model catalog (`ai-models` `lib.rs:18-21`; `code-models` `lib.rs:387-401`). About 2.6 MiB per child, about 31 MiB at 12 children. First candidate if startup must shrink or child memory becomes the limit. Takes the old C5 worker parse with it.
- Regex cache bound (`tui-highlight` `engine.rs:43`, unbounded). Revisit when a session with many languages shows growth.
- Allocator swap. jemalloc: RSS +9%, CPU -9.5% (one run each). Try `MALLOC_ARENA_MAX` first (RT-24). A new dependency needs a clear win.
- Binary size beyond strip. Revisit when binary size becomes a target.
- Frame build off the UI thread (RT-32). Revisit when the user asks.
