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
| The experimental WASM crate (`code-extensions`, wasmtime) becomes an **off-by-default feature** | Nothing uses it, and it costs about 55 seconds on every clean build ([build-speed.md](build-speed.md) D5) |

## What we build

1. Put `wasmtime` behind a `wasm` cargo feature, off by default, plus one CI job
   that builds with it so it doesn't rot.

## Not doing

- A Node extension host for hoocode-ts extensions.
- Hooks.
- A WASM extension API.

## Open questions

- Delete `code-extensions` entirely if still unused in a few months?
