# hoocode-runtime

The one place that builds tokio runtimes, OS threads and channels for hoocode.
See `docs/design/concurrency.md` (sections 1 to 4).

- `io_handle()`: the `hoocode-io` runtime (`min(4, cores)` workers, 2 in subagent children).
- `block_on_entry(fut)`: for `main` and tests only.
- `run_blocking(f)`: runs sync work on the `hoocode-tools` pool (at most 16 threads).
- `spawn_named_thread(name, f)` and `bounded_channel(cap)`.
- `session_io()`: the `hoocode-session-io` thread, the only writer of session files. Bounded queue
  (4096 entries or 64 MiB; producers wait), flush barriers with deadlines.
- `ParallelToolLimit`: the per-turn cap on parallel tool calls.
- `total_memory_bytes()`: physical RAM, used for the memory limit defaults.
- `Lane` (High, Medium, Low): `hoocode-io` workers are High, the tools pool and `hoocode-session-io`
  are Medium (the input and terminal threads come in Phase 2). Priority only goes down (Linux nice 0/+5/+10, macOS QoS,
  Windows priority classes); a failure is logged once and ignored.
- `spawn_bg(future)`: housekeeping (dispatch-dir cleanup, the git watcher) on `hoocode-bg`, one Low
  thread with its own current-thread runtime.
- `lower_child_priority(command, nice)`: subagent children run at nice +5 (`SUBAGENT_CHILD_NICE`);
  `Shell` children at `performance.bashNice`.
