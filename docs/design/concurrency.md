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
| Each `bash` call adds 3 threads | `code-tool-bash` (2 pipe readers + 100 ms flusher) | 8 parallel bash calls = 24 extra threads |
| Each lone ESC spawns a timer thread; resize is a 100 ms polling thread | `tui-terminal` | Threads for timers |
| UI loop writes frames to stdout itself | `tui-render` `do_render` | A slow terminal (ssh, paused pane) blocks input handling |
| Session entries are appended by opening the file per entry | `code-session` `manager.rs` `persist` | Blocking file I/O on whichever thread emits the event; a slow disk stalls it |
| Subagents are child processes, 5 at once, 2 when nested, depth 1 | `code-subagents` `pool.rs`, `depth.rs` | Good. But each child starts its own core-sized runtime |
| No thread priorities, no `nice`, no memory watch | — | Background work competes with the UI on equal terms |

## Decisions

| Decision | Why |
|---|---|
| **Threads where latency matters, tasks for I/O, processes for isolation.** Not one thread per subsystem. | A thread per subsystem (MCP thread, tool thread, …) still blocks inside that subsystem and adds context switches. Async tasks on one small runtime scale to hundreds of streams; processes already isolate subagents. |
| **One tokio runtime per process**, `min(4, cores)` workers; 2 in subagent children. Built only by a new crate `cortexcode-runtime`. | Replaces the 7 runtime builders in non-test code. I/O needs few workers; more just fight the UI for cores. |
| **Three lanes: High, Medium, Low.** Each lane has its own threads, queue and cap. | Tokio can't prioritise tasks, so priority comes from separation: Low work never runs on a High thread. |
| **The UI thread never blocks**: no file or network I/O, no `block_on`, no waiting on another lane's lock. Terminal output moves to its own thread. | A frozen UI is the bug users notice first. |
| **Ctrl+C is handled on the input thread**, not via the UI loop. | Abort works even when the UI loop or the runtime is stuck. |
| **OS priority only goes down, never up.** High = normal priority; Medium and Low are lowered. | Raising priority needs root (`CAP_SYS_NICE`) on Linux, and real-time priority can lock up a machine. Lowering is always allowed. |
| **Every queue is bounded** and has a stated policy: wait, keep-latest, or drop-with-count. | Unbounded channels turn a slow consumer into unbounded memory. |
| **Session data is never dropped.** Its queue applies backpressure; it is flushed before exit, fork and export. | Sessions are the durable record. |
| **Memory has a soft and a hard limit on the process.** Soft sheds load; hard aborts the turn cleanly. Nothing kills cortex itself. | A process that kills itself loses the session; one that slows down does not. |
| **Every wait on something outside the process has a timeout.** | A hung server or tool must end as an error, not a hang. |
| **Measure first.** Phase 0 adds the numbers; later phases must move them. | "Faster" needs a baseline. |
| **Compaction and cleanup are work, not threads.** Compaction's LLM call runs on `cortex-io`, its CPU part on `cortex-tools`; cleanup runs in the Low lane. | Rust has no garbage collector; memory is freed on drop. |
| **The agent's `bash` commands run at normal priority** (nice 0); `performance.bashNice` lowers them. | Builds and tests the agent runs are as fast as from your shell. The UI needs very little CPU. |
| **Memory limits are on by default.** | Off by default leaves the freeze this card exists to stop. |
| **Four settings, everything else fixed** (§3). | Fewer ways to misconfigure. |
| **Numbers are visible**: `/perf` and `--perf-log <file>`. | For the load test and for bug reports. |

## What we build

### 1. Thread map (target)

| Thread / pool | Count | Lane | Does |
|---|---|---|---|
| `main` (UI) | 1 | High | Owns TUI state. Turns events into actions and builds frames. |
| `cortex-input` | 1 | High | Reads stdin, parses keys, ESC timeout, Ctrl+C fast path. Resize via SIGWINCH (polling only on Windows). |
| `cortex-term-out` | 1 | High | Writes frames to the terminal. Holds one pending frame; a newer frame replaces an unwritten one. |
| `cortex-io` runtime | 2–4 workers | High / Medium | Agent loop, LLM streams, MCP clients, rpc and app-server transports, async process pipes. |
| `cortex-session-io` | 1 | Medium | The only writer of session files. Keeps the file open, batches appends, flushes on turn end. |
| `cortex-tools` pool | up to 16 threads | Medium | Sync tool bodies (read, edit, write, grep, find, compaction token counting). |
| `cortex-bg` | 1 thread, own `current_thread` runtime | Low | Housekeeping: prune old dispatch dirs and temp files, session list index, version check, file watchers, future search indexing. |
| `cortex-watchdog` | 1, mostly asleep | High | Heartbeats, stall reports, RSS sampling, limit enforcement. |
| Subagents | child processes, 5 (2 nested), as today | Low | Unchanged pool; children start with 2 workers and lowered priority. |
| `bash` commands | child processes | Medium | Pipes read by async tasks on `cortex-io`, not by 3 threads each. |

Steady state is about 10 threads per process; the ceiling under load is about 26.
Today it is unbounded.

### 2. Lanes and OS priority

| Lane | Linux | macOS | Windows |
|---|---|---|---|
| High | nice 0 | QoS `USER_INTERACTIVE` | `THREAD_PRIORITY_NORMAL` |
| Medium | nice +5 (thread) | QoS `UTILITY` | `BELOW_NORMAL` |
| Low | nice +10 (thread) | QoS `BACKGROUND` | `LOWEST` |
| Subagent children | process nice +5 | process nice +5 | `BELOW_NORMAL_PRIORITY_CLASS` |
| `bash` children | nice 0; `performance.bashNice` | nice 0; `performance.bashNice` | normal; any `bashNice` > 0 means `BELOW_NORMAL_PRIORITY_CLASS` |

- Per-thread priority through the `thread-priority` crate (MIT), owned by
  `cortexcode-runtime` only (add it to `migration/dep-firewall.json`).
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
| `cortex-io` workers | `min(4, cores)`; children 2 | — | — |
| `cortex-tools` threads | 16 | Calls queue | — |
| Parallel tool calls per turn | 8 | Rest wait, in call order | `performance.maxParallelTools` |
| Subagents | 5, nested 2, depth 1 (today) | Queue, then refuse (today) | existing settings |
| In-flight requests per MCP server | 8 | Wait | — |
| LLM events → UI | 1024 events | Stream reading pauses | — |
| Tool progress → UI | 1 per tool | Keep latest | — |
| Frames → `cortex-term-out` | 1 | Keep latest | — |
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

### 4. Never hang

| Rule | How it is enforced |
|---|---|
| No `block_on` except at entry points (`main`, tests) | clippy `disallowed-methods` in `clippy.toml`; each remaining site carries an `allow` with a reason |
| No runtime, thread or unbounded channel built outside `cortexcode-runtime` | same: `Runtime::new`, `Builder::new_*`, `thread::spawn`, `unbounded_channel`, `std::sync::mpsc::channel` are disallowed elsewhere |
| No blocking call on a `cortex-io` worker | Blocking work goes through one helper (`run_blocking`) onto `cortex-tools` |
| No lock held across `.await` or while calling listeners | Review rule; the subagent pool already follows it (subagents.md §6) |
| Every external wait has a deadline | LLM first byte and idle gaps, MCP request, tool run, process exit, session flush |
| UI stall detection | The UI loop bumps a heartbeat and records its current phase. No beat for 500 ms: the watchdog logs the phase; for 2 s: the footer says so. |
| Runtime starvation detection | The watchdog schedules a probe task every second. Late by >250 ms: log which lane is starved. |
| Emergency exit | Ctrl+C twice within 1 s while the UI is stalled: the input thread restores the terminal, flushes the session (with a 1 s deadline) and exits 130. |
| Panics | A panic on any cortex thread is logged with the thread name. On the UI thread the panic hook restores the terminal and flushes the session. Tool panics stay errors (today). |
| Ordered shutdown, 3 s total | Abort agent → kill tool process groups → flush session → close MCP → restore terminal. Each step has its own deadline. |

### 5. Measurement

- `/perf` (and `--perf-log <file>`): threads per lane, queue depths, frame build and
  write time, keystroke-to-frame latency, UI stalls, RSS.
- A load scenario on the mock LLM (the one `subagent_evals.py` and the parity harness
  use): the model streams at full speed, the turn runs 8 parallel `bash` calls that
  print a lot, 5 subagents run, a stdio MCP server never answers, and keys are typed
  throughout. A second run uses a terminal that stops reading (paused pty).

### 6. Phases

Each phase lands on its own, with tests, and keeps the L2 parity checks green
(threads change timing, not bytes on screen).

| Phase | Builds | Done when |
|---|---|---|
| 0 | `/perf`, load scenario, baseline numbers in this card | Numbers recorded for Linux and macOS |
| 1 | `cortexcode-runtime`: one runtime, named threads, `run_blocking`, caps on tools and parallel calls, 2 workers in children; remove the 7 builders and the `block_on` sites | No runtime built outside the crate; thread ceiling holds in the load test |
| 2 | `cortex-term-out`, Ctrl+C on the input thread, SIGWINCH, ESC timer without a thread, async `bash` pipes | Keystroke-to-frame p99 under 16 ms in both load runs; Ctrl+C aborts within 500 ms with the paused terminal |
| 3 | `cortex-session-io` with flush barriers | No session file I/O on the UI thread or on `cortex-io`; kill -9 mid-turn loses at most the unflushed entries of that turn |
| 4 | Lanes with OS priority; `cortex-bg`; `nice` for children | Housekeeping and subagents never delay a frame in the load test |
| 5 | Watchdog, stall reports, memory limits and shedding | A forced 3 GiB allocation sheds, then recovers; a stall shows its phase |
| 6 | Only if Phase 0 shows it: move syntax highlighting and big markdown parses to `cortex-tools` | Frame build p99 under 8 ms on a 10k-line transcript |

## Not doing

- **A thread per subsystem** (MCP thread, per-tool threads, a "GC thread"). Rust has
  no garbage collector; memory is freed on drop. Cleanup work lives in the Low lane.
- **Moving the TUI component tree off the main thread.** 60 TUI files use
  `Rc<RefCell<…>>`; making it `Send` is a rewrite for little gain once stdout I/O
  is off the thread.
- **Raising any priority above normal**, or real-time scheduling.
- **Hard memory limits on the user's `bash` commands.** `RLIMIT_AS` breaks Node, the
  JVM and Go; cgroups are Linux-only. The user's builds run as they would in a shell.
- **In-process subagents by default.** subagents.md §6 keeps child processes.
- **A rayon or work-stealing CPU pool up front.** Only if Phase 0 shows CPU-bound UI
  work (Phase 6).
- **Rewriting the sync tools as async.** They run on `cortex-tools`.
- **Background compaction while you type** (compacting early, before the context is
  full). It changes what the agent does, not just where it runs; it gets its own card
  later.

## Open questions

None. The eight questions were answered on 2026-10-08 and are recorded in
[decisions-2026-10-08.md](decisions-2026-10-08.md).

## Details

### Data flow (target)

```
 stdin ──► cortex-input ──keys──► main (UI) ──frame──► cortex-term-out ──► terminal
              │ Ctrl+C                ▲   │
              ▼                       │   │ prompts, actions
          AbortSignal          events │   ▼
              │              ┌────────┴──────────────── cortex-io (2–4 workers) ───────┐
              └─────────────►│ agent loop ─ LLM stream ─ MCP clients ─ bash pipes     │
                             └──┬───────────────┬────────────────────────┬────────────┘
                                │ entries       │ run_blocking           │ dispatch
                                ▼               ▼                        ▼
                       cortex-session-io   cortex-tools (≤16)     subagent processes (≤5)
                                                                   (2 workers, nice +5)
   cortex-bg (Low): cleanup, index, version check        cortex-watchdog: beats, RSS, stalls
```

### Lane assignment

| Work | Lane | Runs on |
|---|---|---|
| Key handling, frame build, frame write | High | `main`, `cortex-term-out` |
| Main agent's LLM stream | High | `cortex-io` |
| Tool bodies, MCP calls, `bash` I/O | Medium | `cortex-tools`, `cortex-io` |
| Session writes | Medium | `cortex-session-io` |
| Session reads for picker, resume, tree | Medium | `cortex-tools` (result sent to UI) |
| Compaction (LLM part / CPU part) | Medium | `cortex-io` / `cortex-tools` |
| Subagents | Low | child processes |
| Cleanup, indexing, version check, watchers | Low | `cortex-bg` |

### New dependencies

| Crate | Licence | Owner crate |
|---|---|---|
| `thread-priority` | MIT | `cortexcode-runtime` |
| `signal-hook` (SIGWINCH, SIGTERM) | MIT / Apache-2.0 | `cortexcode-runtime` |
| `memory-stats` (own RSS) | MIT / Apache-2.0 | `cortexcode-runtime` |

Child RSS for the lifeguard uses `/proc` on Linux, `proc_pid_rusage` on macOS and
`GetProcessMemoryInfo` on Windows, inside the same crate.

### Rules for CLAUDE.md once Phase 1 lands

- Build threads, runtimes and channels only through `cortexcode-runtime`.
- Never block the UI thread or a `cortex-io` worker; use `run_blocking`.
- Every queue states its cap and policy; every external wait states its deadline.
