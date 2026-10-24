# hoocode-agent-loop

Agent loop for hoocode agents

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

The agent loop works with `AgentMessage` throughout and converts to LLM `Message`s only
at the provider call. Hooks are synchronous closures, and tools keep a synchronous
`execute` that runs on tokio's blocking pool.
