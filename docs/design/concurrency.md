# Concurrency: threads, lanes and limits

Status: **agreed 2026-10-08**, design only ([decisions-2026-10-08.md](../../archive/docs/decisions-2026-10-08.md)).
Build order: phases 0–1 after [reliability.md](reliability.md) and before
[mcp.md](mcp.md); phases 2–5 after MCP.

## Goal

Use Rust's real parallelism for speed, and never freeze. Typing and Ctrl+C must stay
instant while the model streams, tools run, MCP servers stall and subagents work.
Thread count, CPU share and memory stay inside fixed limits, so a busy session cannot
hang the process or the machine.

hoocode-ts is single-threaded (one Node event loop), so there is no reference to
follow. This card is Rust-only.

## Today (2026-10-08)

The parts already run in parallel, but nothing bounds or orders them.

| What | Where | Problem |
|---|---|---|
| Main tokio runtime, one worker per core | `code-cli` `runtime.rs` `async_runtime()` | A 16-core machine gets 16 workers per process, subagent children included |
| Second runtime, 2 workers | `ai-stream` `shared_runtime()` | Two runtimes in one process |
| A new runtime per HTTP MCP server | `agent-mcp` `transport.rs` `Runtime::new()` | One core-sized runtime per server |
| 4 more "thread + `current_thread` runtime" fallbacks | `code-agent-session` `spawn`, `code-subagents` `tools.rs` and `runner.rs`, `code-auth` | Hidden threads; `block_on` inside a runtime can deadlock or panic |
| 40 `block_on` calls, 26 `thread::spawn`, 20 unbounded channels | non-test code | No count, no names, no backpressure |
| Tool calls run on tokio's blocking pool | `agent-loop` `run_tool` | Pool cap is 512 threads; parallel tool calls are not capped |
| Each `Shell` call adds 3 threads | `code-tool-bash` (2 pipe readers + 100 ms flusher) | 8 parallel Shell calls = 24 extra threads |
| Each lone ESC spawns a timer thread; resize is a 100 ms polling thread | `tui-terminal` | Threads for timers |
| `@file` autocomplete spawns `fd` and waits for it | `tui-components` `autocomplete` | Runs on the UI thread; typing stalls on a big tree (fixed by reliability item 6) |
| UI loop writes frames to stdout itself | `tui-render` `do_render` | A slow terminal (ssh, paused pane) blocks input handling |
| Session entries are appended by opening the file per entry | `code-session` `manager.rs` `persist` | Blocking file I/O on whichever thread emits the event; a slow disk stalls it |
| Subagents are child processes, 5 at once, 2 when nested, depth 1 | `code-subagents` `pool.rs`, `depth.rs` | Good. But each child starts its own core-sized runtime |
| No thread priorities, no `nice`, no memory watch | — | Background work competes with the UI on equal terms |

## Decisions

| Decision | Why |
|---|---|
| **Threads where latency matters, tasks for I/O, processes for isolation.** Not one thread per subsystem. | A thread per subsystem (MCP thread, tool thread, …) still blocks inside that subsystem and adds context switches. Async tasks on one small runtime scale to hundreds of streams; processes already isolate subagents. |
| **One tokio runtime per process**, `min(4, cores)` workers; 2 in subagent children. Built only by a new crate `hoocode-runtime`. | Replaces the 7 runtime builders in non-test code. I/O needs few workers; more just fight the UI for cores. |
| **Three lanes: High, Medium, Low.** Each lane has its own threads, queue and cap. | Tokio can't prioritise tasks, so priority comes from separation: Low work never runs on a High thread. |
| **The UI thread never blocks**: no file or network I/O, no `block_on`, no waiting on another lane's lock. Terminal output moves to its own thread. | A frozen UI is the bug users notice first. |
| **Ctrl+C is handled on the input thread**, not via the UI loop. | Abort works even when the UI loop or the runtime is stuck. |
| **OS priority only goes down, never up.** High = normal priority; Medium and Low are lowered. | Raising priority needs root (`CAP_SYS_NICE`) on Linux, and real-time priority can lock up a machine. Lowering is always allowed. |
| **Every queue is bounded** and has a stated policy: wait, keep-latest, or drop-with-count. | Unbounded channels turn a slow consumer into unbounded memory. |
| **Session data is never dropped.** Its queue applies backpressure; it is flushed before exit, fork and export. | Sessions are the durable record. |
| **Memory has a soft and a hard limit on the process.** Soft sheds load; hard aborts the turn cleanly. Nothing kills hoocode itself. | A process that kills itself loses the session; one that slows down does not. |
| **Every wait on something outside the process has a timeout.** | A hung server or tool must end as an error, not a hang. |
| **Measure first.** Phase 0 adds the numbers; later phases must move them. | "Faster" needs a baseline. |
| **Compaction and cleanup are work, not threads.** Compaction's LLM call runs on `hoocode-io`, its CPU part on `hoocode-tools`; cleanup runs in the Low lane. | Rust has no garbage collector; memory is freed on drop. |
| **The agent's `Shell` commands run at normal priority** (nice 0); `performance.bashNice` lowers them. | Builds and tests the agent runs are as fast as from your shell. The UI needs very little CPU. |
| **Memory limits are on by default.** | Off by default leaves the freeze this card exists to stop. |
| **Four settings, everything else fixed** (§3). | Fewer ways to misconfigure. |
| **Numbers are visible**: `/perf` and `--perf-log <file>`. | For the load test and for bug reports. |
| **The `hoocode-io` blocking pool stays uncapped** (tokio's default, 512 threads). Decided 2026-10-15. | A cap can deadlock. An Agent tool body holds a slot while it waits for a child whose `wait()` is queued behind it, and the MCP tool body shares the pool. The load run peaked at 11 threads from this pool, so the thread ceiling does not need a cap. Revisit only after the child `wait()` and the Agent body are async. |

## What we build

### 1. Thread map (target)

| Thread / pool | Count | Lane | Does |
|---|---|---|---|
| `main` (UI) | 1 | High | Owns TUI state. Turns events into actions and builds frames. |
| `hoocode-input` | 1 | High | Reads stdin, parses keys, ESC timeout, Ctrl+C fast path. Resize via SIGWINCH (polling only on Windows). |
| `hoocode-term-out` | 1 | High | Writes frames to the terminal. Holds one pending frame; a newer frame replaces an unwritten one. |
| `hoocode-io` runtime | 2–4 workers | High / Medium | Agent loop, LLM streams, MCP clients, rpc and app-server transports, async process pipes. |
| `hoocode-session-io` | 1 | Medium | The only writer of session files. Keeps the file open, batches appends, flushes on turn end. |
| `hoocode-tools` pool | up to 16 threads | Medium | Sync tool bodies (read, edit, write, grep, find, compaction token counting). |
| `hoocode-bg` | 1 thread, own `current_thread` runtime | Low | Housekeeping: prune old dispatch dirs and temp files, session list index, version check, file watchers, future search indexing. |
| `hoocode-watchdog` | 1, mostly asleep | High | Heartbeats, stall reports, RSS sampling, limit enforcement. |
| `hoocode-perf` | 1, always in the interactive TUI | None: normal priority | Once a second reads `/proc` (RSS, threads) and takes the UI's latest counters, and appends a JSON line to `--perf-log` when set. Started by `Perf::new`, so it runs without `/perf` or the log. Stops when the UI drops it. (`code-tui-app` `perf.rs`) |
| `hoocode-resize` | 1: a SIGWINCH listener on Unix; a size poll every 100 ms on Windows, or if the listener fails to start | None: normal priority | Calls the resize handler when the terminal size changes. Started and stopped with the terminal: the listener ends when its watch is dropped, the poll on the stop flag. (`tui-terminal` `lib.rs`; the listener is `hoocode-runtime` `watch_sigwinch`) |
| `autocomplete-files` | 1 per `@`-completion provider, made at mode start and again on session switch and settings change | None: normal priority | Runs the in-process `@file` walk for the newest query; older queued queries are skipped. Idle between queries. Exits when its provider is replaced or dropped, after any walk in progress. (`tui-components` `autocomplete/file_search.rs`) |
| Subagents | child processes, 5 (2 nested), as today | Low | Unchanged pool; children start with 2 workers and lowered priority. |
| `Shell` commands | child processes | Medium | Pipes read by async tasks on `hoocode-io`, not by 3 threads each. |

Steady state is about 13 threads per process: the 10 steady threads above, plus
`hoocode-perf`, `hoocode-resize` and `autocomplete-files`, which the rows above list.
The ceiling under load is about 26. Today it is unbounded.

### 2. Lanes and OS priority

| Lane | Linux | macOS | Windows |
|---|---|---|---|
| High | nice 0 | QoS `USER_INTERACTIVE` | `THREAD_PRIORITY_NORMAL` |
| Medium | nice +5 (thread) | QoS `UTILITY` | `BELOW_NORMAL` |
| Low | nice +10 (thread) | QoS `BACKGROUND` | `LOWEST` |
| Subagent children | process nice +5 | process nice +5 | `BELOW_NORMAL_PRIORITY_CLASS` |
| `Shell` children | nice 0; `performance.bashNice` | nice 0; `performance.bashNice` | normal; any `bashNice` > 0 means `BELOW_NORMAL_PRIORITY_CLASS` |

- Per-thread priority through the `thread-priority` crate (MIT), owned by
  `hoocode-runtime` only (add it to `migration/dep-firewall.json`).
- Child processes get their priority at spawn (`pre_exec` on Unix, a creation flag
  on Windows).
- If setting a priority fails, log once and carry on.

### 3. Caps

Start values; Phase 0's load test tunes them. Only these four are settings:

| Setting | Default | Range |
|---|---|---|
| `performance.maxParallelTools` | 8 | 1–32 |
| `performance.memorySoftLimitMb` | lower of 2048 and 25% of RAM | 256 up; 0 turns it off |
| `performance.memoryHardLimitMb` | lower of 4096 and 50% of RAM | above the soft limit; 0 turns it off |
| `performance.bashNice` | 0 | 0–19 (Unix); Windows: 0 or above 0 |

| Resource | Cap | When reached | Setting |
|---|---|---|---|
| `hoocode-io` workers | `min(4, cores)`; children 2 | — | — |
| `hoocode-tools` threads | 16 | Calls queue | — |
| Parallel tool calls per turn | 8 | Rest wait, in call order | `performance.maxParallelTools` |
| Subagents | 5, nested 2, depth 1 (today) | Queue, then refuse (today) | existing settings |
| In-flight requests per MCP server | 8 | Wait | — |
| LLM events → UI | 1024 events | Stream reading pauses | — |
| Tool progress → UI | 1 per tool | Keep latest | — |
| Frames → `hoocode-term-out` | 1 | Keep latest | — |
| Session write queue | 4096 entries or 64 MiB | Producer waits (never drop) | — |
| Tool output in memory | today's `DEFAULT_MAX_BYTES`, rolling 2× | Spill to file (today) | — |
| MCP response body | 32 MiB | Error result | — |
| SSE line or event | 16 MiB | Stream error | — |
| Process memory, soft | lower of 2 GiB and 25% of RAM | Shed load (below) | `performance.memorySoftLimitMb` |
| Process memory, hard | lower of 4 GiB and 50% of RAM | Abort the turn, flush session, show a notice | `performance.memoryHardLimitMb` |
| Subagent child memory | 2 GiB each | Lifeguard reap: SIGUSR1 (stall), SIGTERM (deadline), grace, SIGKILL | — |
| `hoocode-io` blocking pool (`spawn_blocking`: child `wait()`, Agent and MCP tool bodies) | Uncapped (tokio default, 512). **Decided 2026-10-15: stays uncapped**, see Decisions | — | — |

Shedding at the soft limit: no new subagent dispatches, parallel tools drop to 1,
render caches are cleared, and the footer shows a warning. It lifts once memory is
10% under the limit.

**Parallel tool cap (landed).** `performance.maxParallelTools` (1–32, default 8) is
applied at launch in `hoocode-agent-loop`: each batch's `SlotLauncher` takes a
`hoocode-runtime` `ParallelToolLimit` permit per call, in call order, before the call
is spawned or takes its `ordered_start` turn. The permit moves into the call's future
and frees the slot when the call settles. It cannot deadlock: slots always go to the
earliest unfinished calls, so a call waiting on the turnstile has all its predecessors
already running and able to reach their dispatch point. N=1 runs the batch strictly
one call at a time.

### 4. Never hang

| Rule | How it is enforced |
|---|---|
| No `block_on` except at entry points (`main`, tests) | clippy `disallowed-methods` in `clippy.toml`; each remaining site carries an `allow` with a reason |
| No runtime, thread or unbounded channel built outside `hoocode-runtime` | same: `Runtime::new`, `Builder::new_*`, `thread::spawn`, `unbounded_channel`, `std::sync::mpsc::channel` are disallowed elsewhere |
| No blocking call on a `hoocode-io` worker | Blocking work goes through one helper (`run_blocking`) onto `hoocode-tools` |
| No lock held across `.await` or while calling listeners | Review rule; the subagent pool already follows it (subagents.md §6) |
| Every external wait has a deadline | LLM first byte and idle gaps, MCP request, tool run, process exit, session flush |
| UI stall detection | The UI loop bumps a heartbeat and records its current phase. No beat for 500 ms: the watchdog logs the phase; for 2 s: the footer says so. |
| Runtime starvation detection | The watchdog schedules a probe task every second. Late by >250 ms: log which lane is starved. |
| Emergency exit | Ctrl+C twice within 1 s while the UI is stalled: the input thread restores the terminal, flushes the session (with a 1 s deadline) and exits 130. |
| Panics | A panic on any hoocode thread is logged with the thread name. On the UI thread the panic hook restores the terminal and flushes the session. Tool panics stay errors (today). **Status 2026-10-15: the hook is not built** (no `panic::set_hook` exists). A panic in a shutdown step is caught and reported as `Failed`, and the terminal restore still runs. |
| Ordered shutdown, 3 s total | Abort agent → kill tool process groups → flush session → close MCP → release session → restore terminal. Each step has its own deadline. **Status 2026-10-15: built** (`runtime` `shutdown.rs`, `run_ordered`; the exit path in `code-tui-app` `mode/shutdown.rs`). Deadlines: abort 0.8 s, kill tool groups 0.1 s, flush 1.0 s, close MCP 0.7 s, release 0.2 s; the terminal restore runs last on the calling thread, in the 0.2 s left of the 3 s. A step past its deadline is logged (`agent_log`) and the next still runs; a panicking step is logged as failed. Tool groups are killed from their own set (`code-tool-bash` `shell.rs`), so background jobs the user chose to leave running survive. The exit-time `block_on` is gone (TUI N14). **Open:** the restore is not timed; MCP close is now 0.7 s, down from 5 s; the Panics row above has no hook. |

### 5. Measurement

- `/perf` (and `--perf-log <file>`): threads per lane, queue depths, frame build and
  write time, keystroke-to-frame latency, UI stalls, RSS.
- A load scenario on the mock LLM (the one `subagent_evals.py` and the parity harness
  use): the model streams at full speed, the turn runs 8 parallel `Shell` calls that
  print a lot, 5 subagents run, a stdio MCP server never answers, and keys are typed
  throughout. A second run uses a terminal that stops reading (paused pty).

### 6. Phases

Each phase lands on its own, with tests, and keeps the L2 parity checks green
(threads change timing, not bytes on screen).

| Phase | Builds | Done when |
|---|---|---|
| 0 | `/perf`, load scenario, baseline numbers in this card | Numbers recorded for Linux and macOS. **Linux recorded 2026-10-15** (see the 2026-10-15 run below). **Open:** macOS is not measured. |
| 1 | `hoocode-runtime`: one runtime, named threads, `run_blocking`, caps on tools and parallel calls, 2 workers in children; remove the 7 builders and the `block_on` sites (the UI-thread `block_on` removal is done under TUI N14 in [tui-activity.md](tui-activity.md), not here) | No runtime built outside the crate; thread ceiling holds in the load test. **Status 2026-10-15: open.** The load run peaked at 34 threads (ceiling 26) for 3 s during the 8-bash and 5-subagent burst, was 24 for the next 9 s, and was 14 to 13 from second 15. **Follow-up 2026-10-15:** the extra threads were the subagent pipe readers (10) and the `hoocode-io` blocking pool (11). The pipe readers are removed (commit b8862c2); the peak is now 24, so the ceiling holds in the load test. The io blocking pool stays uncapped by decision (Decisions table), so it no longer holds the phase open. Phase stays open: `block_on` still has about 12 non-test call sites (grep, not all checked for inline test code). Runtime builders outside `hoocode-runtime` are gone (grep, non-test code). |
| 2 | `hoocode-term-out`, Ctrl+C on the input thread, SIGWINCH, ESC timer without a thread, async `Shell` pipes | Keystroke-to-frame p99 under 16 ms in both load runs; Ctrl+C aborts within 500 ms with the paused terminal. **Status:** parts 1 and 2 landed 2026-10-08; the 2026-10-15 load run is in Baseline (Ctrl+C 30 ms: pass). **Open:** the keystroke p99 passed in the final run but failed in one earlier run of the same bash scenario (70.6 ms, cause not found). **Follow-up 2026-10-15:** five bash runs gave one miss (27.3 ms, started at load average 3.4); the two runs that started under load were repeated and passed. The burst's frame-build cost is the tail (see the follow-up section). |
| 3 | `hoocode-session-io` with flush barriers | No session file I/O on the UI thread or on `hoocode-io`; kill -9 mid-turn loses at most the unflushed entries of that turn. **Status: writes landed** (`runtime` `session_io.rs`; 4096 entries or 64 MiB, producers wait; barrier at turn end, 1 s flush at dispose; torn last line skipped on read and not glued to the next entry). **Open:** session reads still run on the caller (they first wait for queued writes, up to 1 s), and `create_dir_all` and the `open`/`resume` paths are not yet moved to a lane. |
| 4 | Lanes with OS priority; `hoocode-bg`; `nice` for children | Housekeeping and subagents never delay a frame in the load test. **Status:** landed; load run 2026-10-15 in Baseline. **Open:** the burst's frame-build cost (rolling p99 about 102 ms for 5 s) is separated in the follow-up: frame build dominates, lanes add little (see the follow-up section); Windows `Shell` priority (process-wrap's JobObject overwrites creation flags); macOS QoS untested. |
| 5 | Watchdog, stall reports, memory limits and shedding | A forced 3 GiB allocation sheds, then recovers; a stall shows its phase. **Status:** memory check passes (2026-10-15, Baseline). **Open:** the thread ceiling (34 against 26 before b8862c2; 24 after, the phase 1 item); the key p99 in the burst (see the follow-up); the stall-phase check is not run, and the watchdog probes did not fire under load; the footer cannot show a UI stall while the loop is stuck; macOS child RSS and `sysctl` unverified. |
| 6 | Only if Phase 0 shows it: move syntax highlighting and big markdown parses to `hoocode-tools` | Frame build p99 under 8 ms on a 10k-line transcript. **Follow-up 2026-10-15:** the trigger is met. The bash burst's frame build p99 was 98 to 136 ms, against p50 under 1 ms. The cost is the bash block's render. Not started. |

#### Baseline (Phase 0)

Linux only, so far. macOS is not measured yet, and Phase 0 is not done until it is.
Measured 2026-10-08 on a 4-vCPU VM (kernel 6.18). One run per row, driven by
`scripts/perf/load_scenario.py` against the mock model in tmux, 20 keys a second for
15 s. Each frame is timed by `Tui::set_frame_observer`. Keystroke-to-frame runs from
the moment the terminal's reader thread receives the key to the end of the loop
iteration that draws it. The per-second log is in `target/perf/load-*/perf.jsonl`.

| Run (release build unless noted) | Frame build p50 / p99 ms | Frame write p99 ms | Keystroke→frame p50 / p99 ms | Loop iteration p50 / p99 ms | Stalls >500 ms | Threads max | RSS max |
|---|---|---|---|---|---|---|---|
| Load: 8 parallel `Shell`, 5,000 lines each | 28.6 / 48.7 | 0.2 | 29.1 / 191.6 | 29.1 / 99.2 | 0 | 17 | 47.9 MiB |
| Control: 8 `Shell`, 500 lines each | 30.9 / 51.8 | 0.7 | 31.9 / 213.4 | 31.3 / 209.8 | 0 | 17 | 45.6 MiB |
| Control: no tool calls (typing only) | 0.30 / 2.78 | 3.5 | 0.46 / 8.65 | 0.42 / 4.59 | 0 | 9 | 29.3 MiB |
| Debug build, load as the first row | 341 / 470 | 1.7 | 97,826 / 106,026 | 105,255 p99 (one iteration) | 3 | 17 | 64.1 MiB |

What the numbers say:

- Idle typing meets the Phase 2 target (keystroke-to-frame p99 under 16 ms). Under
  tool output it does not: p99 is 192 ms, and frames take about 30 ms at p50.
- A keystroke costs one full synchronous frame. `Tui::handle_input` calls
  `request_render`, and `run()` drains every queued input event before it looks at
  `AppEvent`s. At 20 keys a second and 30 ms a frame the UI thread is about 60% busy
  with frames, so any frame longer than 50 ms builds a backlog. The debug run shows
  the backlog case: 300 queued keys took about 105 s to draw, and the agent's output
  waited behind them.
- The cost does not grow between 500 and 5,000 lines a call. That points at the
  per-frame cost of the bash block, not at the amount of text. Not profiled in release
  yet. A debug-build stack sample showed the UI thread in
  `tool_chain` → `bash.rs` → `visual_truncate` → `Text::render` → `wrap_text_with_ansi`,
  so the block is wrapped in full on each frame before it is truncated.
- Thread count is 9 at idle and 17 once tools run (tokio workers for the `Shell` pipes).
- Frame write time is small everywhere (p99 under 4 ms). The terminal is not what
  stalls the loop; the render is.

Phase 6 says "frame build p99 under 8 ms on a 10k-line transcript". This load has
about 40,000 lines of `Shell` output, so it is a different test. It still fails by
a wide margin (p99 48.7 ms).

Not in these runs yet (the scenario's TODOs): the five subagents, the stdio MCP
server that never answers, and the paused-pty run.

#### Phase 2, part 1 (2026-10-08)

- **`hoocode-term-out`** (`hoocode-tui-terminal` `output.rs`) is the only writer of
  terminal output. Frames, escapes around them, and the escapes the input, progress
  and timer threads send all go through it, in order. Its channel holds 1024 control
  writes (the sender waits when full). A frame is a diff against the previous one, so
  it is never replaced in the queue: the UI builds the next frame only after the
  writer has taken the previous one (`frame_pending`), and the writer wakes the loop
  with `TuiEvent::OutputDrained`. Shutdown writes what is queued within 1 s, then
  drops the rest.
- **Input batching** (`code-tui-app` `run`): up to 64 keys per loop pass go through
  `Tui::accept_event`, which only schedules the frame. The pass then runs app events
  and paints once (`flush_scheduled_render`).
- **SIGWINCH** (`hoocode-runtime` `watch_sigwinch`, Unix) replaces the 100 ms resize
  poll on Unix; the poll stays on Windows and as the fallback if the listener fails.
  Verified by hand in tmux (resize redraws at the new width) and by a runtime test.
- **Numbers** (Linux, 4 vCPU, debug build, `scripts/perf/load_scenario.py`, two runs
  each, same session): keystroke-to-frame p50 about 3 ms on both the base commit and
  this one. Frames for the same 300 keys: 305 here, 340 on base. p99 is 7 to 93 ms on
  this commit and 31 to 92 ms on base, and one frame of the 40k-line bash block (150 to 230 ms)
  sets it. That frame is the Phase 6 cost (`visual_truncate` wraps the whole block).
  So the p99 target is not met yet, and the Phase 0 baseline (frame build p50 341 ms in
  debug) is stale: HEAD builds frames in about 2.3 ms p50.
- **Not in part 1:** Ctrl+C on the input thread, the ESC timer without a thread, async
  `Shell` pipes, the paused-pty run. Other `std::thread::spawn` sites in `code-tui-app`
  (bash, clipboard, login, footer, session picker, perf sampler) are unchanged.
  `clippy.toml` does not forbid `thread::spawn`; adding it means touching those sites.

#### Phase 2, part 2 (2026-10-08)

- **Ctrl+C fast path** (`hoocode-tui-terminal` `interrupt.rs`). The input reader
  scans each raw chunk before the UI sees it. Ctrl+C is `0x03`, Kitty `CSI 99;5u`
  or modifyOtherKeys `CSI 27;5;99~`, but not inside a bracketed paste. Each press
  calls the hook, which `code-tui-app` `run` sets to `Agent::abort` of the session.
  The UI still gets the key. An abort is idempotent, so the second one changes nothing.
- **Emergency exit.** The UI loop beats once per pass (at most 250 ms apart while idle).
  Two presses within 1 s with no beat for 500 ms: the input thread writes the reset
  sequences straight to fd 1 (the output thread may be stuck), disables raw mode, runs
  `session_io().flush_blocking(1 s)` and exits 130.
- **ESC timer without a thread on Unix** (`stdin_hub.rs`). The `hoocode-input` reader
  reads stdin with `read(2)` after `poll(2)`. Its timeout is the ESC deadline, and
  the deadline is checked on each wake-up. Windows keeps a short timer thread per
  flush, because its read cannot time out. The Kitty query fallback still starts one
  timer thread per terminal start; it is not on the keystroke path.
- **Async `Shell` pipes** (`code-tool-bash` `operations.rs`). The command runs with
  `tokio::process` (`process-wrap` `tokio1`, process group kill as before) on
  `hoocode-io`. Two reader tasks send chunks into a bounded queue of 256. The
  calling thread receives them with a 10 ms timeout, which also drives the tool's
  output throttle (`on_idle`). The per-call flusher thread is gone. Before, each
  call took 2 reader threads and 1 flusher. Output bytes, truncation, spill, the
  100 ms pipe grace and the kill on abort or timeout are unchanged.
- **Thread-spawn rule.** `clippy.toml` disallows `std::thread::spawn` and
  `std::thread::Builder::spawn`. Production sites use `hoocode_runtime::spawn_named_thread`
  or the panicking `spawn_thread`. Test files that model a peer carry an allow with
  a reason.
- **Tests.** `tui-terminal`: Ctrl+C encodings, paste exclusion, the emergency rule,
  the hook firing while the UI is blocked, and the ESC deadline. `code-tool-bash`
  `tests/it/operations.rs`: 3000 interleaved stdout/stderr lines in order, 3 MiB
  from both pipes, kill on abort (process group), kill on timeout, the idle tick
  while silent and after the pipes close. The existing 20 Shell tests pass.
- **L2 parity** (`harness.py run`): `startup`, `chat-basic`, `tool-read`,
  `session-mixed`, `bash-command`, `tool-bash`, `print-tool-bash`,
  `print-tool-bash-light` all pass. No scenario has "abort" in its name.
- **Not in part 2:** the paused-pty run (Ctrl+C within 500 ms with a terminal that
  stops reading), the keystroke-to-frame numbers, a thread-count test, and the
  Kitty fallback timer thread. Ctrl+C is not detected when a paste marker is split
  across two reads.

#### Load run, 2026-10-15 (phases 1 to 5)

Linux, 4 vCPU, release build, mock model in tmux, through `scripts/perf/load_scenario.py --scenario all`.
Raw artifacts: `target/perf/load-20261015-095300/`. Targets as stated in §6; not tuned.

Keystroke numbers come from the ring of the last 1,024 keys (`perf.rs`). The load run types 975 keys,
so its figures cover the whole run. The bash run types 300, so it does too.

| Run | Keys typed | Keystroke→frame p50 / p99 ms | Frame build p99 ms | Loop p99 ms | Stalls >500 ms | Threads max | RSS max MiB | Worst rolling key p99 ms (any sample) |
|---|---|---|---|---|---|---|---|---|
| bash: 8 `Shell` × 5,000 lines, 15 s of typing | 300 | 0.66 / **5.22** | 2.50 | 2.88 | 0 | 13 | 34.0 | 116.1 |
| load: bash + 5 subagents + MCP stall, 75 s of typing | 975 | 1.06 / **3.84** | 2.65 | 3.11 | 0 | **34** for 3 s, 13 at end | 41.6 | 102.2 |
| paused-pty: pty not read 5 s, then Ctrl+C | 0 | n/a | 117.1 | 118.2 | n/a | n/a | n/a | n/a |

- **Keystroke to frame, p99 under 16 ms over the whole run:** load 3.8 ms, bash 5.2 ms, in this run.
  The same bash scenario gave 4.2, 1.9 and 70.6 ms in earlier runs on this code. The 70.6 ms run is kept
  in this record; its cause is not identified. Phase 2 is left open for that reason.
- **Thread ceiling, 26: MISS** in the load run. Threads were 34 for 3 s (the burst), 24 for the next 9 s,
  14 from second 15, and 13 at the end. The bash run stays at 13. These are the parent process's threads;
  subagent children are separate processes and are not counted.
- **Lanes never delay a frame: not shown for the burst.** The rolling key p99 was about 102 ms for 5 s and
  28 ms for 7 more, then under 3 ms. The likely cause is the frame build of the bash block (`visual_truncate`,
  the Phase 6 cost), but this run does not separate that from lane contention, so no lane claim is made.
- **MCP stall: PASS.** The stall server accepted one `tools/call` and never replied. The turn ended at
  60.3 s, the 60 s request deadline, with 0 loop stalls (`scripts/perf/mcp_stall_server.py`).
- **Subagents: PASS.** 5 of 5 children answered, 5 child requests. An earlier attempt with a 3-turn
  child route ran out of script and two children failed (`500 mock: script exhausted`). That was the
  harness; the route now has 40 turns. The gate counts successful answers, not requests.
- **Paused terminal: PASS.** The pty was not read for 5 s. Bytes queued were 0 at the start of the pause
  and 4,095 at its end (sampled twice; about one pty buffer, so the app was probably blocked on output,
  not proven). Ctrl+C reached the HTTP client 30.1 ms after the press (target 500 ms). The prompt came back
  after the backlog drained. The paused run's frame p99 comes from the bash block drawn during the run; it
  types no keys.
- **Memory: PASS.** A forced 3 GiB allocation with the soft limit at its 256 MB floor and the hard limit at
  its default. The shedding notice appeared after the shim reported every page resident, and lifted after
  release. RSS peaked at 3,095 MiB; the hard limit (4,096 MiB) was not reached. The allocation comes from an
  `LD_PRELOAD` shim (`scripts/perf/alloc_shim.c`, test only). It acts only in a process named `hoocode` with
  `HOOCODE_PERF_ALLOC_MB` set and `HOOCODE_SUBAGENT_DEPTH` unset or `0`, because the product has no
  allocation hook.
- **Not measured:** macOS; the watchdog's stall and starvation probes under this load; Windows priority.

#### 2026-10-15 load run, follow-up

Linux, 4 vCPU, release build, same scripts as above. Raw artifacts under `target/perf/` (`attr-load`,
`attr-bash`, `fix-load`, `bash-run-*`, `bash-rerun-*`, `probe-*`). Load average is the 1-minute value from
`/proc/loadavg` when the run starts. A reviewer's `nextest` run shared the box for part of this.

**1. Thread ceiling: who spawns the extra threads.** A sampler read `/proc/<pid>/task/*/comm` every 0.1 s
during the load run (`scripts/perf/load_scenario.py --scenario load`, before the fix).

| Source at the 34-thread peak | Threads | Where it comes from | After the fix |
|---|---|---|---|
| Steady threads (main, input, term-out, session-io, bg, watchdog, 4 io workers) | 10 | §1 map | 10 |
| `perf` sampler, resize thread, `autocomplete-files` | 3 | Not in the §1 map (now listed there) | 3 |
| `hoocode-subagent` pipe readers | 10 | `ProcessHandle` in `code-subagents` `runner.rs`: 2 threads per child, 5 children | **0** |
| `hoocode-io-*` blocking pool, above the 4 workers | 11 | `spawn_blocking` on the io runtime: 5 child `wait()` calls (`runner.rs`), 5 Agent tool bodies and 1 MCP tool body (`agent-loop` `run_tool`) | 11 |
| **Total** | **34** | | **24** |

- The 11 blocking threads are counted from the code, not sampled one by one. The total matches exactly.
- The "resize" thread is `hoocode-resize`. On Linux it is the SIGWINCH listener, not a poll, unless the
  listener failed to start (SIGWINCH replaced the poll on Unix in part 2 of phase 2).
- No `hoocode-tools` thread was alive in the load run. The 16-thread cap was never reached.
- The bash-only run peaked at 13 threads and had no blocking-pool threads. Its bash calls return
  quickly and run as background jobs on `hoocode-io`.
- **Fix (commit b8862c2, `code-subagents`):** the pipes are read by tasks on `hoocode-io`, as `Shell`
  pipes already are. On Unix no thread is started for them (on Windows tokio reads child pipes on its
  blocking pool, so the saving is Unix-only). A test checks that no pipe thread appears while a child
  runs, and that 4 MiB of output arrives whole. The test fails on the old code (2 threads) and passes now.
- **Decided 2026-10-15: the io blocking pool stays uncapped.** Tool bodies and child `wait()` calls share the
  pool, which has no cap (tokio's default is 512). A cap would risk a deadlock: an Agent body holds a slot
  while it waits for a child whose `wait()` is queued behind it. The peak was 11 threads, so the ceiling holds
  without a cap. See the Decisions table and the §3 caps table.

**2. Keystroke p99, bash scenario, five runs (after the fix).** Whole-run keystroke-to-frame p99, 300 keys
over 15 s.

| Run | Load avg (1 min) at start | Keystroke p99 ms | Threads max | 16 ms |
|---|---|---|---|---|
| 1 | 3.39 | **27.27** | 13 | MISS |
| 2 | 2.76 | 5.54 | 13 | pass |
| 3 | 2.05 | 1.04 | 13 | pass |
| 4 | 1.47 | 4.51 | 13 | pass |
| 5 | 1.05 | 2.24 | 13 | pass |
| 1, repeat (run under load first) | 0.75 | 4.59 | 13 | pass |
| 2, repeat (run under load first) | 0.62 | 4.62 | 13 | pass |

- Six of seven runs pass. The one miss started at a load average of 3.4. Its cause is not known. The
  earlier 70.6 ms run is still unexplained too.
- **Does 16 ms hold?** Usually yes, with a 1-in-7 miss on this box. Not yet stable enough to call it met.

**3. Burst frame build against lane effects.** Per-second values from the perf log of the load run (after the fix).

| Window | Frame build p50 / p99 ms | Keystroke p50 / p99 ms | Loop p99 ms |
|---|---|---|---|
| 3 to 7 s: 8 bash jobs printing, 5 subagents, MCP call open | 0.7 / 136 | 0.5 to 0.8 / 136 | 136 |
| 9 to 14 s: children done, MCP still open | 0.7 / 5 | 0.8 / 63 (window lag) | 6 |
| 16 to 30 s: bash output done | 0.7 / 2 to 3 | 0.8 / 5 to 6 | 4 to 6 |
| Whole run | n/a | n/a / **4.4** | n/a |

- Frame build p50 stays at 0.7 ms. The p99 comes from a few frames of 100 ms or more. The bash block is
  the cause: each slow frame is the one that draws it.
- The keystroke tail follows the frame tail. The keystroke p50 stays under 1 ms.
- Bash only, no subagents (`attr-bash`): burst frame build p99 112 ms, keystroke p99 114 ms, p50 0.4 to 0.6 ms.
- **Which dominates:** frame build. Lanes do not show as a separate signal. The load run peaked at 136 ms,
  the bash-only run at 112 ms. That gap is one run each, so it could be lanes or noise. It is not
  separated more finely.
- **Points to phase 6** (move big renders off the UI thread, or cut the bash block's render cost). The
  phase 6 trigger is met. Phase 6 itself is not started.

**4. Watchdog probes under load.** The probes log only when the debug log is on. In the TUI, watchdog lines go
through `agent_log`, which withholds them unless `HOOCODE_DEBUG_AGENT_LOG=1`. For these runs a wrapper turned
the log on and kept it in a directory that outlives the scenario.

| Probe | Trigger | Load run (8 bash, 5 subagents, MCP stall, 975 keys) | Paused pty (5 s not read, then Ctrl+C) |
|---|---|---|---|
| UI stall (loop silent 500 ms, footer at 2 s) | `[WATCHDOG] UI loop silent` / `UI stalled` | No line. Perf `stalls` 0 | No line. Perf `stalls` 0. Loop p99 79 ms |
| io starvation (probe 1 s, late over 250 ms) | `[WATCHDOG] io runtime starved` | No line | No line |

- Both probes stayed silent under these loads. Only `DISPATCH` lines reached the debug log, so the sink works.
- **No control run:** no forced stall or starvation was built to show that the probes fire. The "silent" result
  is therefore not proof they work. Test the probes with a forced stall before relying on them.
- Ctrl+C in the paused run: 36 ms, PASS (target 500 ms).
- **Points to phase 6?** Yes, for the frame build. The probes do not show lane starvation under these loads.

**Summary**

| Question | Answer |
|---|---|
| Thread ceiling (26) | Met after the fix: 24. Cause: subagent pipe readers (10) and the uncapped io blocking pool (11) |
| Keystroke p99 under 16 ms | Usually met (6 of 7 runs). One run missed, cause unknown |
| What dominates the burst tail | Frame build of the bash block. Lanes add little or nothing that shows |
| Probes under load | Silent. No control run, so not proven to work |
| Phase 6 | Trigger met. Not started |

Not done: macOS; the watchdog control run. The io blocking pool stays uncapped (decided 2026-10-15).

## Not doing

- **A thread per subsystem** (MCP thread, per-tool threads, a "GC thread"). Rust has
  no garbage collector; memory is freed on drop. Cleanup work lives in the Low lane.
- **Moving the TUI component tree off the main thread.** 60 TUI files use
  `Rc<RefCell<…>>`; making it `Send` is a rewrite for little gain once stdout I/O
  is off the thread.
- **Raising any priority above normal**, or real-time scheduling.
- **Hard memory limits on the user's `Shell` commands.** `RLIMIT_AS` breaks Node, the
  JVM and Go; cgroups are Linux-only. The user's builds run as they would in a shell.
- **In-process subagents by default.** subagents.md §6 keeps child processes.
- **A rayon or work-stealing CPU pool up front.** Only if Phase 0 shows CPU-bound UI
  work (Phase 6).
- **Rewriting the sync tools as async.** They run on `hoocode-tools`.
- **Background compaction while you type** (compacting early, before the context is
  full). It changes what the agent does, not just where it runs; it gets its own card
  later.

## Open questions

None. The eight questions were answered on 2026-10-08 and are recorded in
[decisions-2026-10-08.md](../../archive/docs/decisions-2026-10-08.md).

## Details

### Data flow (target)

```
 stdin ──► hoocode-input ──keys──► main (UI) ──frame──► hoocode-term-out ──► terminal
              │ Ctrl+C                ▲   │
              ▼                       │   │ prompts, actions
          AbortSignal          events │   ▼
              │              ┌────────┴──────────────── hoocode-io (2–4 workers) ───────┐
              └─────────────►│ agent loop ─ LLM stream ─ MCP clients ─ bash pipes     │
                             └──┬───────────────┬────────────────────────┬────────────┘
                                │ entries       │ run_blocking           │ dispatch
                                ▼               ▼                        ▼
                       hoocode-session-io   hoocode-tools (≤16)     subagent processes (≤5)
                                                                   (2 workers, nice +5)
   hoocode-bg (Low): cleanup, index, version check        hoocode-watchdog: beats, RSS, stalls
```

### Lane assignment

| Work | Lane | Runs on |
|---|---|---|
| Key handling, frame build, frame write | High | `main`, `hoocode-term-out` |
| Main agent's LLM stream | High | `hoocode-io` |
| Tool bodies, MCP calls, `Shell` I/O | Medium | `hoocode-tools`, `hoocode-io` |
| Session writes | Medium | `hoocode-session-io` |
| Session reads for picker, resume, tree | Medium | `hoocode-tools` (result sent to UI) |
| Compaction (LLM part / CPU part) | Medium | `hoocode-io` / `hoocode-tools` |
| Subagents | Low | child processes |
| Cleanup, indexing, version check, watchers | Low | `hoocode-bg` |

### New dependencies

| Crate | Licence | Owner crate |
|---|---|---|
| `thread-priority` | MIT | `hoocode-runtime` |
| `signal-hook` (SIGWINCH, SIGTERM) | MIT / Apache-2.0 | `hoocode-runtime` |
| `memory-stats` (own RSS) | MIT / Apache-2.0 | `hoocode-runtime` |

Child RSS for the lifeguard uses `/proc` on Linux, `proc_pid_rusage` on macOS and
`GetProcessMemoryInfo` on Windows, inside the same crate.

### Rules for CLAUDE.md once Phase 1 lands

- Build threads, runtimes and channels only through `hoocode-runtime`.
- Never block the UI thread or a `hoocode-io` worker; use `run_blocking`.
- Every queue states its cap and policy; every external wait states its deadline.
