<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="../assets/hoocode.svg">
    <img alt="HooCode" src="../assets/hoocode-light.svg" height="64">
  </picture>
</p>

<p align="center">Deterministic terminal coding agent.</p>

# Install

## Requirements

- **Node.js** ≥ 20 (for the npm install; the prebuilt binary needs no runtime)

## Install from npm

```bash
npm install -g @kolisachint/hoocode-agent
hoocode --help
```

This installs the `hoocode` (and `hoo`) command globally.

## First run

```bash
hoocode               # start in build mode
hoocode /mode plan    # or draft a plan first
hoocode --help        # see all flags
```

Pick a provider and model with `--provider` / `--model`; HooCode supports 25+
providers. See [docs/product.md](product.md) for the mode and tool model.

## Build from source

bun is the toolchain. It is pinned to the npm-compatible **hoisted** linker in
`bunfig.toml`, so it produces a flat `node_modules`. `bun.lock` is the
authoritative lockfile.

```bash
git clone https://github.com/kolisachint/hoocode.git
cd hoocode

bun install          # Install all dependencies
bun run build        # Build all packages
bun run check        # Lint, format, and type check
./test.sh            # Run tests (skips LLM-dependent tests without API keys)
```

See [docs/bun-migration.md](bun-migration.md) for the completed npm → bun
migration history and rules.

## Restricted / offline environments

HooCode runs without network access. Two capabilities normally reach out to
GitHub to download helper binaries (`fd` for the `find` tool and file
autocomplete, `rg` for the `grep` tool); everything else is self-contained.

- **`HOOCODE_OFFLINE=1`** (or `--offline`) disables all startup network
  operations — no binary downloads, no version checks. The `find`/`grep` tools
  fall back to a built-in pure-JS implementation, so search keeps working.
- **`HOOCODE_NATIVE_SEARCH=1`** forces the pure-JS `find`/`grep` path even when
  `fd`/`rg` could be downloaded. It also engages automatically whenever those
  binaries are unavailable.
- **Pre-seed the binaries** to get native `fd`/`rg` speed offline: install them
  from your OS package manager (they are used straight from `PATH`), or drop the
  executables into `~/.hoocode/bin/{fd,rg}`.

The interactive UI no longer blocks on these downloads — it starts immediately
and wires `fd` in once resolved, so a slow or blocked network never delays
launch.

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines and
[AGENTS.md](../AGENTS.md) for project-specific rules (for both humans and
agents).
