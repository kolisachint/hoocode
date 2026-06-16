# HooCode

An AI coding agent for the terminal. Extend it with modes, profiles, MCP servers, and per-project
instructions — without forking internals.

[![npm](https://img.shields.io/npm/v/@kolisachint/hoocode-agent?style=flat-square)](https://www.npmjs.com/package/@kolisachint/hoocode-agent)

---

## Install

```sh
npm install -g @kolisachint/hoocode-agent
```

Requires Node.js 20.6 or later.

---

## Quick start

```sh
# Launch interactive session (defaults to build mode)
hoocode

# Ask a question without editing anything
hoocode /mode ask

# Start planning a feature
hoocode /mode plan
```

On first run, `~/.hoocode/` is bootstrapped with a default config, all five mode
prompts, and the three built-in profile contexts.

---

## Modes

Modes control what the agent is allowed to do. Switch with `/mode <name>` or run
`hoocode /mode` to see the current mode.

| Mode | Intent | File writes | Shell |
|------|--------|-------------|-------|
| `ask` | Read-only Q&A | No | Read-only |
| `plan` | Explore and write a plan | `.hoocode/plan.md` only | Read-only |
| `build` | Implement carefully, one step at a time | Yes | Guarded |
| `debug` | Root-cause analysis, no modifications | No | Read-only |

### Mode details

**ask** — Answers questions, traces logic, compares approaches. Refuses edits and state-changing
commands. Cite files and line numbers.

**plan** — Explores the codebase, drafts a complete plan, and writes it to `.hoocode/plan.md`.
The plan has sections: Goal, Files to modify, New files, Tests, Verification. When done it
tells you to run `/approve`.

**build** — Reads before editing, shows diffs before non-trivial changes, asks for confirmation
before destructive operations, runs tests after every logical unit.

**debug** — Gathers evidence, reproduces the bug, traces the call path, states the root cause
in one sentence, and describes the fix — but does not apply it. Switch to `/mode build` to apply.

---

## Profiles

Profiles inject domain-specific rules into the system prompt. HooCode auto-detects the right
profile from file markers in your project root, or you can pin one with `/profile <name>`.

### Built-in profiles

**default** — General full-stack. Match existing patterns, prefer the smallest change,
assume Node.js / TypeScript / npm workspaces / vitest.

**data** — BigQuery / SQL. Dry-run before mutating statements, no `SELECT *` on large tables,
inspect schema first, validate join keys and cardinality.

**devops** — Infrastructure. Never run `terraform apply` or `kubectl delete` without showing
the plan first, prefer declarative config, never hardcode secrets, every change needs a
rollback strategy.

### Auto-detection markers

| File present | Profile selected |
|--------------|-----------------|
| `dbt_project.yml` | data |
| `*.sql` | data |
| `terraform.tf` | devops |
| `.github/workflows` | devops |
| `Dockerfile` | devops |
| `docker-compose.yml` | devops |
| `k8s/` | devops |

---

## Plan workflow

```sh
# 1. Switch to plan mode
hoocode /mode plan

# 2. Describe the feature
> Add a /health endpoint that returns { status: "ok", version } as JSON

# Agent explores the codebase and writes .hoocode/plan.md

# 3. Review the plan
cat .hoocode/plan.md

# 4. Approve — switches to build mode and executes the plan step by step
hoocode /approve
```

The plan file lives at `.hoocode/plan.md` and has these sections:

```
## Goal
## Files to modify
## New files
## Tests
## Verification
```

Add `.hoocode/` to your `.gitignore` to keep plans and local overrides out of version control:

```
.hoocode/
```

---

## Config directory

```
~/.hoocode/
├── config.json          # Global defaults (mode, profile, auto-allow lists, LLM providers)
├── agent/               # Session state written by the agent
├── modes/               # Mode system prompts (one dir per mode)
│   ├── ask/system.md
│   ├── plan/system.md
│   ├── build/system.md
│   └── debug/system.md
├── profiles/            # Profile context files (one dir per profile)
│   ├── default/context.md
│   ├── data/context.md
│   └── devops/context.md
├── mcp-servers/         # MCP server configs (one JSON file per server)
└── extensions/          # Global extensions
```

Edit any `.md` file to customise a mode or profile for all your projects.

---

## Project-local overrides

See [Project-local resources](docs/project-local-resources.md) for the full discovery rules covering slash commands, subagents, MCP servers, `.agents/` ancestor-walk, and the `hoocode resources` debug command.


Drop a `.hoocode/` directory in your project root.

### `.hoocode/config.json` — per-project config

Overrides global defaults for this project only. Scalar fields win; `auto_allow` arrays
are unioned.

```json
{
  "active_mode": "build",
  "active_profile": "devops",
  "modes": {
    "build": { "auto_allow": ["bash"] }
  }
}
```

### `.hoocode/agents.md` — per-project instructions

Appended to the system prompt after the mode and profile layers. Use it to describe project
conventions, forbidden patterns, or team rules.

```markdown
This is a Go monorepo. All new packages go in internal/. Run `make test` not `go test ./...`.
Never commit directly to main — always open a PR.
```

### Merge order (lowest → highest priority)

1. `~/.hoocode/config.json` — global defaults
2. `.hoocode/config.json` — project overrides (scalar fields win; `auto_allow` arrays unioned)
3. `before_agent_start` — mode + profile + agents.md injected into system prompt

---

## Permission gate

Before running `bash`, `edit`, or `write` in interactive sessions, HooCode prompts:

```
Allow: $ npm test
  Yes (once)
  No (block)
  Always (add to auto-allow for this mode)
```

Selecting **Always** writes the tool name into `modes.<active_mode>.auto_allow` in
`~/.hoocode/config.json`. It applies globally. Use `.hoocode/config.json` to set
project-scoped auto-allow lists instead.

---

## MCP servers

Drop a JSON config file in `~/.hoocode/mcp-servers/` (global) or `.hoocode/mcp-servers/`
(project-local). HooCode connects on session start and registers each server's tools automatically.

```json
{
  "name": "my-db-tools",
  "command": "npx",
  "args": ["-y", "my-mcp-server"],
  "env": {
    "DATABASE_URL": "postgres://localhost/mydb"
  }
}
```

Tool names are registered as `mcp_<server-name>_<tool-name>`.

---

## Custom modes and profiles

Add a new mode:

```sh
mkdir -p ~/.hoocode/modes/review
cat > ~/.hoocode/modes/review/system.md << 'EOF'
You are in **review mode** — code review only.

Read the diff or files provided. Comment on correctness, style, security, and test coverage.
Do not apply fixes. Summarise findings at the end.
EOF
hoocode /mode review
```

Add a new profile:

```sh
mkdir -p ~/.hoocode/profiles/rust
cat > ~/.hoocode/profiles/rust/context.md << 'EOF'
**Profile: Rust**
- Run `cargo clippy` before suggesting fixes.
- Prefer `Result<T, E>` over `unwrap()`.
- Check `Cargo.toml` for existing dependencies before adding new ones.
EOF
hoocode /profile rust
```

---

## Extensions

HooCode loads TypeScript extensions at startup. Extensions can register tools, commands,
and event handlers.

```ts
// .hoocode/my-extension.ts
import type { ExtensionAPI } from "@kolisachint/hoocode-agent";

export default function (pi: ExtensionAPI) {
  pi.registerTool({
    name: "my_tool",
    description: "Does something useful",
    parameters: Type.Object({ input: Type.String() }),
    async execute(_id, { input }) {
      return { content: [{ type: "text", text: `You said: ${input}` }] };
    },
  });
}
```

---

## Standalone binary (optional)

### Bun-compiled binary (recommended)

`bun build --compile` produces a self-contained executable that embeds the Bun
runtime, so end users need neither Node.js nor Bun installed. Requires
[Bun](https://bun.sh) on the build machine.

```sh
cd packages/coding-agent
bun run build:bun-binary
```

This builds the workspace dependencies, compiles the executable, and stages the
runtime assets next to it. Output goes to `dist/bun-binary/`:

```
dist/bun-binary/
  hoocode            # the executable
  package.json       # read for name/version/config
  README.md, CHANGELOG.md
  photon_rs_bg.wasm  # image processing
  theme/             # built-in themes
  export-html/       # HTML export templates
  docs/, examples/, templates/
```

Distribute the entire `dist/bun-binary/` directory: the assets must sit next to
the executable (the runtime resolves them relative to `process.execPath`).

Cross-compile for another platform with `--target` (the binary is written to
`dist/bun-binary/<target>/`):

```sh
bun run build:bun-binary -- --target bun-linux-x64
bun run build:bun-binary -- --target bun-darwin-arm64
bun run build:bun-binary -- --target bun-windows-x64
```

Supported targets follow Bun's naming: `bun-<os>-<arch>` where `os` is `linux`,
`darwin`, or `windows` and `arch` is `x64` or `arm64`. Cross-compiled binaries
are not smoke-tested on the host.

### pkg-compiled binary (legacy)

Alternatively, install `pkg` to build a Node.js-based self-contained binary:

```sh
npm install -g pkg
cd packages/coding-agent
npm run build:binary
```

The binary is written to `dist/hoocode`. The `build:binary` script in `package.json`
already sets `pkg.targets` for macOS, Linux, and Windows x64.

---

## License

MIT
