# Decisions, 2026-10-08

Made with the user in a review of [concurrency.md](concurrency.md). Where that card
disagrees with this page, this page wins. Everything in
[decisions-2026-10-07.md](decisions-2026-10-07.md) still stands; this page only adds.

## Order of work (updated)

1. **Reliability first** ([reliability.md](reliability.md)), unchanged.
2. **Concurrency phases 0–1**: measure, then one runtime and the caps.
3. **Close the 8 `l1_done` tasks** (`SearchHooCode`), unchanged.
4. **MCP client.**
5. **Concurrency phases 2–5**: terminal-output thread, session writer, lanes and
   priority, watchdog and memory limits.
6. **Plugins.**
7. **MCP Apps** and the **scheduler**.

Phase 6 (moving syntax highlighting off the UI thread) happens only if Phase 0's
numbers show it is needed.

## Decisions

| Question | Decision |
|---|---|
| Thread model | **Lanes**, not a thread per subsystem. Dedicated threads for UI, input, terminal output, session writer and watchdog; one tokio runtime of 2–4 workers for I/O; a capped pool for tools; subagents stay processes. |
| Build order | **Split**: phases 0–1 before MCP, phases 2–5 after it. |
| "gcc separate" | Meant **compaction and cleanup**. Compaction runs on `cortex-io` and `cortex-tools`; cleanup runs in the Low lane. No GC thread (Rust has no GC). |
| Priority of the agent's `bash` commands | **Normal (nice 0)**, lowered with `performance.bashNice`. |
| Memory limits | **On by default**: soft at the lower of 2 GiB and 25% of RAM, hard at the lower of 4 GiB and 50% of RAM. |
| Settings | **Four keys** in a `performance` block: `maxParallelTools`, `memorySoftLimitMb`, `memoryHardLimitMb`, `bashNice`. Every other cap is fixed in code. |
| Background compaction while typing | **Later, in its own card.** Not part of concurrency. |
| Performance numbers | **`/perf`** command and **`--perf-log <file>`**. |
