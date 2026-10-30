# Short-session performance

Agreed with the user on 2026-10-29. Two items are approved to build later, in a separate
session. Two are investigations with decision gates. The long-session items are deferred.
Where [decisions-2026-10-29.md](decisions-2026-10-29.md) disagrees, that page wins.

## Goal

Make the common session lean and quiet: 10 to 15 turns. At that length the parent is
already small (about 33 to 36 MiB RSS, frame p50 about 0.7 ms). So the gains left are in
the shipped binary and in idle CPU, not in frame cost. Long-session work waits (see
Not doing).

## Decisions

| # | Decision | Answer |
|---|---|---|
| 1 | What to optimise for | **Sessions of 10 to 15 turns.** |
| 2 | Long-session items | **Deferred.** Whole-transcript re-render, single copy of rendered lines, lazy catalog, regex LRU, allocator swap. Reasons in Not doing. |
| 3 | P1 release profile | **Approved, built 2026-10-30.** |
| 4 | P2 idle wakeups | **Approved, built 2026-10-30.** |
| 5 | M3 subagent child footprint | **Measure first.** Propose only if the gate in M3 is met. |
| 6 | M4 streaming markdown re-lex | **Measure first.** Propose only if the gate in M4 is met. |

## What we build

### Step 0. Measured 10 to 15 turn baseline (before P1 and P2)

Do this first, in the build session. The short-session figures in this card are derived from
the 300-turn run (Details, Baseline). Record a real 10-turn and a 15-turn run with the mock
before P1 and P2 are judged.

| Item | Work |
|---|---|
| Runs | One TUI session each of 10 and 15 turns, on the mock (`scripts/tui/mockllm.py`, the same settings as Method), on the release binary built as today (plain profile). Five runs each for the acceptance numbers |
| Metrics | RSS (VmRSS, every 0.5 s from `/proc`), CPU (`/proc/<pid>/stat`), frame p50 and p99 (`--perf-log`), idle wakeups per thread (`/proc/<pid>/task/<tid>/status`) |
| Record | A table in the Details baseline: turns, RSS at the end, CPU per turn, frame p50 and p99, wakeups per second. Replace the derived 33 to 36 MiB and frame 0.7 ms figures with the measured ones, and note any difference |
| Gate | P1 and P2 are judged against these numbers, not the 300-turn run |

### P1. Release profile (built 2026-10-30)

Add to `Cargo.toml` (there is no `[profile.release]` today; `[profile.dev]` is at line 193):

```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
# panic stays "unwind": hoocode-runtime run_blocking turns a panic into Err through
# JoinError (crates/hoocode-runtime/src/runtime.rs:112).
```

Shipped binaries are built by `.github/workflows/binaries.yml:83`
(`cargo build --locked --release --target ... --bin hoocode`). The profile applies to that
line with no workflow change.

| Item | Value |
|---|---|
| Evidence | Strip alone: 57.8 to 43.1 MiB (measured). CPU -5% to -15% (estimate, not measured; the 100-turn A/B is not done) |
| Cost | Longer release build. Baseline: 10m16s wall, 18m31s user on 4 vCPU, clean target (measured, plain release). Built 2026-10-30: 10m17s wall on a cold release cache, user time not recorded (see After table). The extra cost of thin LTO and one codegen unit over the plain profile is not measured |
| Measure the cost | Time a clean `cargo build --release --locked --bin hoocode` before and after, on the same host. Compare the `binaries` job duration in CI before and after |
| Risk | `strip` removes symbols, so panic backtraces from release builds lose function names. See open question 1 |

Acceptance:

- Stripped binary size is at or below 43 MiB on linux x86_64 (baseline 57.8 MiB unstripped).
- `hoocode --version` median wall is not worse than 3.8 ms; `--help` than 3.6 ms.
- TUI time to first prompt is not worse than 42 to 54 ms.
- 100-turn mock CPU A/B: five runs each of the old and new binary, same mock script. Accept
  if median CPU falls and median RSS does not rise by more than 5%. The report's single-run
  baseline (9.23 s CPU, 47.9 MiB RSS, glibc) is not enough to judge a 5% change.

### P2. Idle wakeups (built 2026-10-30)

| # | Source | Now | Change | Where |
|---|---|---|---|---|
| a | Perf sampler thread | Always started: `Perf::new` calls `Sampler::start` at startup, 1 Hz `/proc` read | Start the thread only with `--perf-log`. Without it, `/perf` reads `/proc` once, inline, and starts no thread (built: stricter than the first-`/perf` option) | `crates/hoocode-code-tui-app/src/perf.rs:238` (spawn), `:303-306` (`Perf::new`), `mode/mod.rs:701` (caller) |
| b | Watchdog tick | 100 ms | 250 ms. Stall log stays at 500 ms (`UI_LOG_STALL`), so a stall is seen 0 to 250 ms late. RSS sampling must move from every 10 ticks to every 4 ticks, or it drops to every 2.5 s | `crates/hoocode-runtime/src/watchdog.rs:27` (`TICK`), `:29` (`SAMPLE_EVERY_TICKS`), `:18` (`UI_LOG_STALL`) |
| c | Git HEAD poll | 100 ms, in a git repo | 1 s. File events would need a new crate (dependency firewall applies), so they are not in this step | `crates/hoocode-code-tui-app/src/footer_data.rs:237` (sleep), `:19` (`WATCH_POLL`) |
| d | UI loop cap | 250 ms cap on the wait | 1 s cap. Every other wake already has its own deadline (notifications, tips, autocomplete, cursor blink, loaders, picker, auto theme, stream render). Check that nothing relies on the cap | `crates/hoocode-code-tui-app/src/mode/mod.rs:1252` (cap), `:1283` (final `max`) |

Expected wakeups, from the baseline (17.2 per second, idle, non-git directory): perf 1.0
goes to 0; watchdog 9.9 goes to about 4; UI loop 4.0 goes to about 1; `hoocode-io-2` 2.3
is unchanged. Total about 7 per second, plus 1 per second for the git poll in a git repo.

**Built 2026-10-30.** The four changes are in. Four things were added beyond the table:

- The UI loop's idle wait is now a beat that counts from the end of the wait (`watchdog::ui_beat_waiting`, `interrupt::ui_beat_waiting`). Without it, the 1 s cap would make an idle loop look silent for up to 1 s, past the 500 ms `UI_LOG_STALL` (a "UI loop silent" line each idle second) and past the 500 ms Ctrl+C emergency threshold (a double Ctrl+C at idle would exit without a clean shutdown).
- The bash execution spinner advances in the loop, but its frame was not in `next_wakeup`. It now has a deadline (`BashExecutionComponent::next_deadline`), so it keeps its frame rate at the cap.
- The perf sampler (P2a) has no thread unless `--perf-log` is given and the file opens. Without it, `/perf` reads `/proc` once, inline (`Perf::snapshot`), and no thread runs. So one `/perf` leaves no 1 Hz thread behind. This is stricter than the table's "or on the first `/perf` open".
- The RSS sample count is exposed as `hoocode_runtime::rss_samples()` so a test can check the cadence.
- The footer HEAD poll takes its interval in `FooterDataProvider::with_poll_interval`. The footer tests use 100 ms, so they still check debounce and no-op updates. The app uses 1 s.
- The loop beats `ui-loop:input` after each wait, so the watchdog's beat is back to real time once the wait ends.

Loop cap check (`crates/hoocode-code-tui-app/src/mode/mod.rs`, `next_wakeup` and `run`), read on 2026-10-30:

- Deadlines in `next_wakeup` (own timers, no wake needed): notifications, tips, autocomplete debounce, cursor blink, the header and compaction loaders, picker (`ActivePicker::deadline`, including the session list while it loads), auto theme (60 s), streaming render throttle, the bash spinner (added), the once-a-second tool tick when something ticks (added: attached view, task panel ticking, turn clock, a ticking tool block).
- Autocomplete `@file` walk in flight: a 40 ms step (`Editor::autocomplete_deadline`, via `AutocompleteProvider::walk_pending`). Added, so a finished walk shows at once, not up to 1 s later.
- Session list loading: a 40 ms step (`SessionSelectorComponent::deadline`). Its results arrive on a channel that does not wake the loop. Added.
- Woken by events, so the cap does not delay them: every `AppSender::send` (session events, footer branch change as `Rerender`, the editor's walk result is polled as above), and key or resize input.
- Checked each pass with no wake of their own, so they are now up to 1 s late when idle (was up to 250 ms): extension UI requests (`drain_extension_ui_requests`), the update notice (`poll_update_notice`), and the memory and stall flags (`sync_runtime_health`). None is a correctness issue; these idle-only notices can appear up to 1 s late.
- Checked each pass and fine at 1 s: `poll_attached_prompt` and `drain_task_panel_events` (fed by input handled in the same pass), `advance_auto_loop` (driven by `PromptDone`, which wakes), `poll_auto_theme` (its 60 s deadline is in `next_wakeup`), `tick_scheduler` (30 s interval, at most 1 s late), `poll_tree_selector` and `poll_fork_selector` (their picker deadlines are already in `ActivePicker::deadline`), and `poll_model_selectors` (its events come from keys handled in the same pass).
- Footer HEAD poll at 1 s with a 500 ms debounce: a branch change now shows in about 1 to 2 s (was about 0.6 s).

The target of about 5 per second is **not reached by these four changes alone** (they give
about 7). The `hoocode-io-2` wakeups (2.3 per second) have no known source. Either find them
(see open question 2) or accept about 7.

Acceptance:

- Re-run the 10-second idle wakeup count and the 25-second idle CPU, in a non-git directory
  and in a git repo. Record wakeups per thread.
- Idle CPU is below 0.2% (baseline) and near 0.1%.
- Stall detection still works: the existing watchdog tests (`crates/hoocode-runtime/tests/it/watchdog.rs`,
  `watchdog_forced.rs`) pass. Add one check that RSS is still sampled about once a second at 250 ms.
- `/perf` shows numbers when opened after startup (one inline `/proc` read; no thread without `--perf-log`).

## Measure first (investigations, not approved work)

### M3. Subagent child footprint

Facts: each subagent is a full process, spawned from `current_exe()`
(`crates/hoocode-code-subagents/src/instance.rs:75`; `runner.rs:117` (spawn) and `:121` (`env_clear`), then the
spec's env). Up to 12 run at once (`pool.rs:457`, `DEFAULT_MAX_CONCURRENCY = 12`). Each is
22 to 34 MiB. The eval peak was 288 MB summed across 13 processes (measured).

Method:

1. Run the parent under `valgrind --tool=massif --trace-children=yes`, with a mock script
   (`scripts/tui/mockllm.py`) that starts one subagent. Each child writes its own massif file.
2. Sample each child's VmRSS every 0.5 s from `/proc/<pid>/status`, for processes whose exe is the hoocode binary.
3. List what the child loads that it does not use: UI, highlighter, model catalog.

Gate: a lean child (skips UI, highlighter and catalog init; keeps tools and the agent loop)
saves **8 MiB or more per child** in both steady RSS and massif peak. Then propose it.
Changing `DEFAULT_MAX_CONCURRENCY` is a separate decision for the user.

### M4. Streaming markdown re-lex

Facts: `set_text` (`crates/hoocode-tui-components/src/markdown/mod.rs:72-80`) bumps
`text_rev` on every change. The lexer runs again when the cached tokens are for an older
revision (`:134-135`, `lexer::lex`). So each streamed chunk lexes the whole message: O(n)
per chunk, O(n²) per message. The reason is code reading only, not measured.

Method:

1. Run one mock turn that streams a 20 KB answer. Use `mockllm.py` with `--chunk-size 8`
   (its default), `delay_s` 0 for the turn, and a custom `models.json` provider as
   `scripts/tui/goldens.py` does.
2. Count instructions with `valgrind --tool=callgrind` over the streaming window. Read
   inclusive cost of `lexer::lex` against the whole process with `callgrind_annotate --inclusive=yes`.
   Use instruction counts, not wall time (callgrind is about 20 to 50 times slower).

Gate: lexing is **25% or more** of streaming work. Then propose incremental lexing from the
last completed block (keep finished blocks' tokens; lex only the open tail).

## Not doing (deferred, with evidence)

Long-session items. They matter after about 100 turns, not at 10 to 15.

| Item | Measured evidence (300-turn TUI run) | Why deferred |
|---|---|---|
| Stop rebuilding the whole transcript each frame (`crates/hoocode-tui-render/src/component.rs:130-139` `into_lines`; `tui.rs:1769` `build_live_frame`; `:1819` `changed_rows`) | Frame p50 0.73 ms at turn about 30, 2.37 ms at turn 300. p99 4.4 to 12.5 ms. About 66 of 119 CPU-ms per turn (derived) | At 10 to 15 turns the frame is still about 0.7 ms. Cost is linear in history, so it waits for the long-session card |
| One copy of rendered lines (`tui.rs:1882` `previous_lines`; per-child line cache) | RSS grows 0.17 to 0.18 MiB per turn, linear, no plateau. About 2 to 3 MiB over 10 to 15 turns | Small at this length. Revisit with the item above |
| Lazy model catalog (`crates/hoocode-ai-models/src/lib.rs:18-21`; `crates/hoocode-code-models/src/lib.rs:387-401` `load_models`) | Registry 0.71 MB, merged copy 0.75 MB (massif). Print mode 17.6 ms CPU vs `--version` 3.8 ms | Costs about 1.5 MB and a few ms at each start. Real but small. Deferred by user decision; it is the first candidate if startup needs to shrink |
| Bound the regex cache (`crates/hoocode-tui-highlight/src/engine.rs:43`, unbounded `HashMap`) | Not measured with many languages | Only grows with distinct languages in one session. Unknown size |
| Allocator swap (jemalloc, mimalloc) | jemalloc: CPU -9.5%, RSS +9% (one run each, no noise measure). glibc 47.9 MiB, 9.23 s; jemalloc 52.4 MiB, 8.35 s | One run each. A new dependency needs a clear win. `MALLOC_ARENA_MAX` can be tried first without code |

Other deferred items from the baseline: a bigger rewrite of the highlighter, and binary
size beyond the strip in P1 (code groups: std 9.7 MB, serde 3.8 MB, aws-lc 1.5 MB, rmcp 1.3 MB).
These change disk size, not RSS.

## Open questions

1. **Symbols in crash reports after `strip = "symbols"`.** Recommended: keep the stripped
   binary and upload an unstripped copy as a CI artifact, so a crash can still be
   symbolised. Alternative: ship the unstripped binary (about 14.7 MiB larger). Trade-off:
   the artifact costs CI storage and needs a step.
   **Decided 2026-10-30: not in P1.** P1 ships the stripped binary. The CI unstripped-symbol
   artifact is a follow-up (`binaries.yml`: upload an unstripped copy beside each binary).
2. **The 5-per-second wakeup target.** After P2 the expected rate is about 7 per second.
   Recommended: accept about 7, and track `hoocode-io-2` (2.3 per second, unknown source) as a
   follow-up with its own measurement. Alternative: watchdog tick at 500 ms, which would
   double the stall log delay and is not recommended.
   **Decided 2026-10-30: accept about 7** (measured 6.9 to 7.2 per second in a plain
   directory, 8.9 to 9.3 in a repo). **Follow-up: trace the io-worker wakeups** (about
   2 per second, which worker varies between runs, so the source is not one fixed thread).
   In a repo, `hoocode-bg` also wakes about 2 per second, where the 1 s git poll explains 1;
   that second source is not traced either.

## Details

### Baseline

Commit `a38731d`, measured 2026-10-29. Host: 4 vCPU Intel Xeon 2.1 GHz, 16 GB RAM, Linux 6.18 VM.
Release build with the plain profile (no `[profile.release]`), glibc malloc, debuginfo off.

Metrics for short sessions:

| Metric | Value | Source |
|---|---|---|
| Binary size | 57.8 MiB unstripped; 43.1 MiB stripped | `ls -l`, `strip` on a copy |
| `--version` | 3.8 ms median wall (range 3.3 to 5.3), max RSS 12.2 MiB | 20 runs, `wait4` rusage |
| `--help` | 3.6 ms median wall, max RSS 12.3 MiB | same |
| TUI time to first prompt | 42 to 54 ms (5 runs) | tmux, poll for the prompt text |
| TUI RSS at first prompt | 23.2 to 24.5 MiB | `/proc/<pid>/status` VmRSS |
| Idle TUI RSS | 24.9 MiB, flat over 10 s | `/proc/<pid>/status` |
| Idle TUI CPU | 0.20% over 10 s; 0.24% over 25 s | `/proc/<pid>/stat` utime + stime |
| Idle wakeups | 17.2 per second: watchdog 9.9, UI loop 4.0, `hoocode-io-2` 2.3, perf 1.0 | voluntary context switches per thread, 10 s |
| Print mode, one mock turn | 50 ms wall, 17.6 ms CPU (median of 6) | `RUSAGE_CHILDREN` deltas |
| RSS after turn 0 (TUI) | 33.0 MiB | 300-turn run |
| RSS at 10 to 15 turns | about 35 MiB (derived: 33.0 + 0.17 per turn) | not recorded directly |
| Frame p50 | 0.73 ms at turn about 30 | `--perf-log` JSON, 1 line per second |
| Subagent children | 22 to 34 MiB each; eval peak 288 MB across 13 processes | eval run with sampler |
| Subagent evals | 19 of 19 pass, 30 attempts, 97% usable, 99.6 s wall | `scripts/eval/subagent_evals.py --bin <release>` |

Long-session context (300 turns): RSS 85.0 MiB at turn 299 (peak 85.7); slope 0.17 to 0.18 MiB
per turn; CPU 0.119 CPU-s per turn; 13,142 frames; frame p50 2.37 ms and p99 12.5 ms at turn 300.

### After P1/P2 (2026-10-30)

Built on the card's branch from `d3869aa` (P1 profile plus P2a to P2d, and the review fixes).
Host and conditions as the baseline (4 vCPU, Linux 6.18 VM), but other agents shared the
host during the builds. Release binary, stripped by the new profile. Idle figures are one
TUI in a fixed 100x30 tmux pane with a mock provider config that is never called.

| Metric | Baseline (`a38731d`, card above) | After (2026-10-30) | Notes |
|---|---|---|---|
| Binary size | 57.8 MiB unstripped; 43.1 MiB stripped (strip only, plain profile) | **34.86 MiB** (36,556,240 bytes), release profile | Acceptance (at or below 43 MiB) met. Not like for like: the 43.1 figure is strip alone on the plain profile. LTO and one codegen unit also shrink it |
| `hoocode --version` median wall | 3.8 ms (range 3.3 to 5.3) | **5.29 ms** (min 4.82, max 13.98), 20 runs | Above the 3.8 ms reference. Not an A/B on this host, so it may be host state. Needs an A/B against the old binary before the acceptance is judged |
| `hoocode --help` median wall | 3.6 ms | **5.19 ms** (min 4.66, max 7.29), 20 runs | Same caveat as `--version` |
| `--version` output | (not in the card) | `0.2.20` on **stderr**, exit 0 | Works. Observed only; not changed here |
| TUI time to first prompt | 42 to 54 ms (5 runs) | **31 to 34 ms** (3 plain, 2 repo runs) | Harness polls the prompt text every 20 ms; the baseline harness is not recorded in detail, so only roughly comparable |
| Idle wakeups per s, total (10 s) | 17.2 | **6.9 to 7.2** plain (3 runs); **8.9 to 9.3** in a repo (2 runs) | Acceptance target 5 was not reached; 7 is accepted (open question 2) |
| Idle wakeups, `hoocode-watchdog` | 9.9 | **3.9 to 4.0** | 4 ticks a second |
| Idle wakeups, UI loop thread (`hoocode`) | 4.0 | **1.0** | The cap; a wake each second |
| Idle wakeups, one io worker | 2.3 (`hoocode-io-2`) | **2.0 to 2.2** | Which worker wakes changes between runs; baseline named `io-2` only |
| Idle wakeups, `hoocode-perf` | 1.0 | **0** | No sampler thread without `--perf-log` |
| Idle wakeups, `hoocode-bg` (repo only) | not in baseline | **2.0 to 2.1** in a repo | The 1 s git poll explains 1 per s; the other is not traced |
| Idle CPU over 25 s | 0.20% (10 s), 0.24% (25 s) | **0.16 to 0.2%** | Acceptance "near 0.1%" not met. One sample set, no error bars |
| Release build wall, first build on this tree | 10m16s wall, 18m31s user (clean, plain) | **10m17s** wall (617 s), user not recorded | The release cache was empty, so dependencies compiled. Close to a clean build. Not an A/B: the plain profile was not rebuilt, so the LTO cost is not isolated |
| Release build wall, incremental after the review fixes | (none) | 2m55s (176 s) | Changed crates and link only |

Not measured in this pass (so not judged): 10- and 15-turn mock runs (Step 0), the 100-turn
mock CPU A/B (five runs each, acceptance), the subagent evals, frame p50 and p99, RSS at
the first prompt and at 10 to 15 turns, and the idle wakeups for the 25 s window
beyond the CPU figure. Stall detection was checked by tests (watchdog and forced-stall
tests pass, plus the RSS cadence test), not by a live stall. The watchdog and Ctrl+C
fixes were argued from the code and unit-tested, not observed as log output.

Consequences to know about (not bugs; see the loop-cap check above): idle-only notices
(extension UI requests, the update notice, memory and stall flags) can appear up to 1 s
late. The git branch in the footer updates in about 1 to 2 s after a branch change.

### Method (to re-run)

The scripts live in the session scratchpad, not in the repo. The method below is what they do.

- **Build:** `cargo build --release --locked -p hoocode-code-main` (binary `hoocode`). The
  pinned toolchain (1.94.1, `rust-toolchain.toml`) had no cargo in that container, so the build
  used `RUSTUP_TOOLCHAIN=stable` (cargo 1.97.0). Use the pinned toolchain for a CI-equal run.
- **Mock model:** `scripts/tui/mockllm.py --script turns.json --port 0 --port-file p.txt --log req.jsonl`.
  Point a custom `models.json` provider at it (see `scripts/tui/goldens.py`, around line 351).
  The scratchpad runs used 40-character chunks and a 10 ms delay; `mockllm.py` defaults to 8
  characters, so pass `--chunk-size 40` to match the baseline.
- **Driving the TUI:** a fixed-size tmux session (as in `scripts/tui/goldens.py`). Send the
  prompt N times. Read RSS and CPU from `/proc` every 0.5 s. Use `--perf-log` for frame stats.
- **Process totals:** `wait4` rusage for one-shot runs; `/proc/<pid>/stat` for long runs.
- **Wakeups:** voluntary context switches per thread from `/proc/<pid>/task/<tid>/status`,
  over 10 seconds of idle.
- **Heap:** `valgrind --tool=massif`. Massif figures are heap, not RSS (valgrind inflates RSS).
- **Allocator:** `LD_PRELOAD=libjemalloc.so.2`, with the glibc run as the control.

### Caveats

- Turns are paced by the mock, so CPU per turn is comparable across runs, not a real-model number.
- The 10 to 15 turn figures are derived from the 300-turn run. Run a 10 and a 15 turn session
  before P1 and P2 are judged, to confirm 33 to 36 MiB and frame p50 near 0.7 ms.
- Each A/B was one run. Use five runs for any acceptance number.
- No hardware counters (`perf` not installed). CPU is from `/proc`, 0.5 s sampling; process totals are exact.
- A VM host; numbers may move on the CI runner.
- The idle wakeup numbers are from a non-git directory. The git poll (footer_data) is not in them.
- Soak runs beyond 300 turns and multi-hour sessions are not measured; linear growth is assumed.
- Code line references were checked on 2026-10-29 against the code at `a38731d`.
