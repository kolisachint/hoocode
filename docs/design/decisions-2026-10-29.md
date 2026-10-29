# Decisions, 2026-10-29

Made with the user while planning performance. Where an earlier page disagrees, this page wins.

## Performance (short sessions)

The card is [perf-short-sessions.md](perf-short-sessions.md).

| # | Question | Answer |
|---|---|---|
| 1 | What to optimise for | **Sessions of 10 to 15 turns.** At that length the parent is already lean (about 33 to 36 MiB RSS, frame p50 about 0.7 ms). |
| 2 | Long-session items (whole-transcript re-render, single copy of rendered lines, lazy catalog, regex LRU, allocator swap) | **Deferred**, with their measured evidence and reasons in the card's "Not doing". |
| 3 | P1: `[profile.release]` with `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`; keep `panic = "unwind"` | **Yes, approved.** Build in a separate session. |
| 4 | P2: idle wakeups (perf sampler only on `--perf-log` or `/perf`; watchdog tick 250 ms; git HEAD poll 1 s; UI loop cap 1 s) | **Yes, approved.** Build in a separate session. |
| 5 | M3: subagent child footprint | **Measure first.** Propose a lean child only if it saves 8 MiB or more per child. Changing the default concurrency needs the user's decision. |
| 6 | M4: streaming markdown re-lex | **Measure first.** Propose incremental lexing only if lexing is 25% or more of streaming CPU. |
