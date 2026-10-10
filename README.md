# hoocode

hoocode is a coding agent in Rust. It fully replaces the earlier TypeScript build, hoocode-ts (retired 2026-10-10).

This is a multi-crate workspace that mirrors the structure of the [pycortex](https://github.com/kolisachint/pycortex) Python migration. Each namespace (`ai`, `agent`, `code`, `tui`) is split into focused, version-locked crates published to crates.io.

## Workspace structure

```
crates/
  hoocode-ai-types/
  hoocode-ai-models/
  ...
  hoocode-agent-core/
  ...
  ...
  ...
```

All crates share a single lockstep version defined in the workspace `Cargo.toml`.

## Installation

**macOS / Linux** (no root, no Node):

```bash
curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh
```

It installs `~/.hoocode/bin/hoocode` (and `hoo`), checks the download against the
release's `SHA256SUMS`, and adds `~/.hoocode/bin` to `PATH`. `--help` lists the
options (`--version`, `--dir`, `--no-modify-path`).

**npm / bun:**

```bash
npm install -g @kolisachint/hoocode     # or: bun add -g @kolisachint/hoocode
```

The package pulls in only the binary for your platform (macOS or Linux, x64 or
arm64). There is no Windows build yet.

### From source

The command is **`hoocode`** (alias `hoo`).

```bash
git clone https://github.com/kolisachint/hoocode
cd hoocode
scripts/install.sh                     # → updates ~/.hoocode/bin/hoocode if installed, else ~/.local/bin
scripts/install.sh --prefix /usr/local/bin --also-hoocode   # also keep a `hoocode` link
```

Plain cargo still works and installs the binary as `hoocode`:
`cargo install --path crates/hoocode-code-main --bin hoocode`.

### Pre-built binaries

Download `hoocode-<target>.tar.gz` from the
[GitHub Releases](https://github.com/kolisachint/hoocode/releases) page and check it
against `SHA256SUMS`. It holds the `hoocode` binary; put it on your `PATH`. Linux builds are static (musl) and run on any distro.

## Usage

```bash
# Single-shot print mode (text or JSON)
hoocode -p "Explain this codebase"
hoocode -p --mode json "Explain this codebase"

# Interactive TUI mode
hoocode

# JSON-RPC server mode
hoocode --mode rpc

# Subagent mode (used internally by the Agent tool)
hoocode --mode subagent --task-id <id>
```

## Development

```bash
# Build the entire workspace
cargo build

# Run checks for all crates
cargo check --workspace

# Run all tests (CI uses nextest; `cargo test --workspace` works too)
cargo nextest run --workspace
```

### Releases

Label a PR `rust:patch`, `rust:minor` or `rust:major`; merging it runs the release:
CI gates (parallel) → version bump, tag and GitHub release → binaries for 4 targets in
parallel, each uploaded as soon as it is built. Optional labels: `release:skip-gates`
(skip the gates, the PR's CI already passed) and `release:crates` (also publish to
crates.io, off by default). Manual runs: Actions → Release.

## Migration status (paused 2026-10-01, ready to use)

The port of hoocode **v0.6.0** (commit `2223437c`) is paused, and `hoocode` is usable as a
daily coding agent. Phases 7, 8 and 11 are complete. Phase 10 is complete apart from the
deferred items listed below. Every remaining task is **deferred** by user decision; none is
in progress. Status per task: `python3 archive/migration/ledger.py status`.

What works (the pinned hoocode behavior, checked against hoocode itself):

- **Modes:** interactive TUI, print mode (`-p`, text and `--mode json`), and `--mode rpc`
  (with a Rust `RpcClient`).
- **Providers:** every catalog provider, plus OAuth logins (`/login`) and `models.json`.
- **Sessions:** AgentSession with compaction, `/new`, `/resume`, `/fork`, `/tree`,
  `--continue`, `--session`, and `/export <file>.jsonl`.
- **Tools:** Read, Shell, Edit, Write, lexical CodeSearch, TodoWrite, AskUserQuestion, and the
  Yes/No/Always permission prompt.
- **Agent modes:** ask/plan/build/debug (`/mode`, `/plan`, `/grill`, `/approve`, alt+a).
- **Subagents:** the Agent/AgentOutput tools, the task panel, and the subagent roster.
- **Resources:** skills, prompt templates and slash commands, AGENTS.md/CLAUDE.md context
  files, themes, settings (`/settings`) and keybindings.

Deferred (not available yet, even though `--help` still lists some of them):

| Area | Ledger task |
|---|---|
| MCP servers (`mcp.json`, stdio/HTTP/SSE, OAuth) | 9.1, 10.11 |
| `WebFetch` / `WebSearch` (`--enable-webtools`) | 10.2e |
| Plugins and marketplace (`--enable-plugintools`) | 12.1 |
| `hoocode install/remove/update/list` package manager | 12.2 |
| Code extensions (`-e`, extension flags) | 12.3 |
| Semantic and hybrid search (`embsearch`). Search is lexical-only. | 12.4 |
| `/loop`, `/goal` autonomous loop, cron/scheduler | 12.5 |
| Warm subagent pool (`--warm-subagents`) | 12.6 |
| HTML export and `/share`, `/learn`, canvas, `--team`, voice, telemetry, version check | 12.7 |

Known parity gaps: tasks 10.2a/b/c/d/f/g, 10.4c and 10.5 pass Level 1 but not Level 2. The
default-bundle system prompt in hoocode also advertises the DocSearch self-knowledge tool
from 12.4, so model requests differ in that one block.

The plan and its rationale are in
[`archive/docs/ts-to-rust-migration.md`](archive/docs/ts-to-rust-migration.md)
(§0 status, §9 phases), and the handoff log is in `archive/migration/PROGRESS.md`. To resume, take a
deferred task out of `deferred` and say "continue migration" (see `CLAUDE.md`).

## Quick start

```bash
cargo install --path crates/hoocode-code-main --bin hoocode   # puts `hoocode` on PATH
export ANTHROPIC_API_KEY=...        # or any provider key from `hoocode --help`, or /login
hoocode                              # interactive TUI
hoocode -p "Summarize this repo"     # one-shot
```

- Config, auth, sessions and settings live in `~/.hoocode` (override with
  `HOOCODE_CODING_AGENT_DIR`). Project overrides go in `./.hoocode/`.
  Data from the pre-1.2 home and project folders is merged in once, with backups
  (`hoocode migrate --dry-run` shows what would change).
- The settings, `models.json`, `auth.json`, session JSONL and `hoo-config.json` formats match
  hoocode's, so an existing hoocode setup can be copied over.

## Publishing

Publishing is driven from GitHub Actions:

- `Reserve crates.io names` — one-off workflow that publishes `0.0.1` placeholder crates.
- `Release` — bump, build, publish, and create a GitHub release.
- `Merge Release` — auto-releases PRs labeled `rust:patch`, `rust:minor`, or `rust:major`.
- `Build binaries` — cross-compiles the `hoocode` binary for Linux, macOS (Intel/Apple Silicon), and Windows.

Crates marked with `[package.metadata.hoocode] publish = true` are included in automated releases.
