# hoocode-runtime

The one place that builds tokio runtimes, OS threads and channels for hoocode.
See `docs/design/concurrency.md` (sections 1 to 4).

- `io_handle()`: the `hoocode-io` runtime (`min(4, cores)` workers, 2 in subagent children).
- `block_on_entry(fut)`: for `main` and tests only.
- `run_blocking(f)`: runs sync work on the `hoocode-tools` pool (at most 16 threads).
- `spawn_named_thread(name, f)` and `bounded_channel(cap)`.
- `ParallelToolLimit`: the per-turn cap on parallel tool calls.
- `total_memory_bytes()`: physical RAM, used for the memory limit defaults.
