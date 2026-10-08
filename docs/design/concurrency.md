# Concurrency: threads, lanes and limits

Status: **agreed 2026-10-08**, design only ([decisions-2026-10-08.md](decisions-2026-10-08.md)).
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
| Subagents | child processes, 5 (2 nested), as today | Low | Unchanged pool; children start with 2 workers and lowered priority. |
| `Shell` commands | child processes | Medium | Pipes read by async tasks on `hoocode-io`, not by 3 threads each. |

Steady state is about 10 threads per process; the ceiling under load is about 26.
Today it is unbounded.

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
| Subagent child memory | 2 GiB each | Lifeguard reap: SIGTERM, grace, SIGKILL (today's path) | — |

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
| Panics | A panic on any hoocode thread is logged with the thread name. On the UI thread the panic hook restores the terminal and flushes the session. Tool panics stay errors (today). |
| Ordered shutdown, 3 s total | Abort agent → kill tool process groups → flush session → close MCP → restore terminal. Each step has its own deadline. |

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
| 0 | `/perf`, load scenario, baseline numbers in this card | Numbers recorded for Linux and macOS |
| 1 | `hoocode-runtime`: one runtime, named threads, `run_blocking`, caps on tools and parallel calls, 2 workers in children; remove the 7 builders and the `block_on` sites | No runtime built outside the crate; thread ceiling holds in the load test |
| 2 | `hoocode-term-out`, Ctrl+C on the input thread, SIGWINCH, ESC timer without a thread, async `Shell` pipes | Keystroke-to-frame p99 under 16 ms in both load runs; Ctrl+C aborts within 500 ms with the paused terminal. **Part 1 landed 2026-10-08** (writer thread, input batching, SIGWINCH; see "Phase 2, part 1" below). Part 2 (Ctrl+C on input, ESC timer, async `Shell` pipes) and the done-when numbers are open. |
| 3 | `hoocode-session-io` with flush barriers | No session file I/O on the UI thread or on `hoocode-io`; kill -9 mid-turn loses at most the unflushed entries of that turn. **Status: writes landed** (`runtime` `session_io.rs`; 4096 entries or 64 MiB, producers wait; barrier at turn end, 1 s flush at dispose; torn last line skipped on read and not glued to the next entry). **Open:** session reads still run on the caller (they first wait for queued writes, up to 1 s), and `create_dir_all` and the `open`/`resume` paths are not yet moved to a lane. |
| 4 | Lanes with OS priority; `hoocode-bg`; `nice` for children | Housekeeping and subagents never delay a frame in the load test |
| 5 | Watchdog, stall reports, memory limits and shedding | A forced 3 GiB allocation sheds, then recovers; a stall shows its phase |
| 6 | Only if Phase 0 shows it: move syntax highlighting and big markdown parses to `hoocode-tools` | Frame build p99 under 8 ms on a 10k-line transcript |

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
[decisions-2026-10-08.md](decisions-2026-10-08.md).

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
