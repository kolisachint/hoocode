# Extensions: standards only

Status: **agreed 2026-10-07**, design only. Replaces ledger 12.3.

## Goal

Decide how third-party code extends `cortex`.

## Decisions

| Decision | Why |
|---|---|
| Extensions are **MCP servers, skills and subagents**, delivered by [plugins.md](plugins.md) and [mcp.md](mcp.md) | This is what the standards cover (MCP; Agent Plugins' components are skills and MCP) |
| **No code-extension runtime.** hoocode-ts's TypeScript extensions (its `ExtensionAPI`) don't run in `cortex`. | Rust has no stable plugin ABI; a Node bridge would be a second runtime to keep in step |
| **No hooks** (shell commands on events) | Not standardized; dropped with plugins |
| The experimental WASM crate (`code-extensions`, wasmtime) is **deleted** (2026-10-08) | Nothing uses it, and it costs about 55 seconds on every clean build. Replaces [build-speed.md](build-speed.md) D5. |

## What we build

1. Delete `crates/cortexcode-code-extensions` and its `wasmtime` entries (workspace,
   `dep-firewall.json`, any CI job). Done in the first coding session.

## Not doing

- A Node extension host for hoocode-ts extensions.
- Hooks.
- A WASM extension API.

## Open questions

- None.
