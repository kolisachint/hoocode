# hoocode-agent-core

Core agent runtime for hoocode agents

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

`Agent` is the stateful wrapper around the agent loop. It owns the transcript, runs one
prompt at a time with its own abort signal, notifies listeners, and queues steering and
follow-up messages.
