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

### Containers & Kubernetes (Docker, GKE)

HooCode runs in containers, including as `root` and with a read-only root
filesystem (config-directory writes fail silently rather than crashing). Three
deployment prerequisites are inherent to running an LLM agent and are not
things HooCode can work around:

1. **A provider credential.** Set an API key (`ANTHROPIC_API_KEY`,
   `OPENAI_API_KEY`, …) or, for Copilot, the explicit `COPILOT_GITHUB_TOKEN`.
   A bare `GH_TOKEN`/`GITHUB_TOKEN` is *not* treated as an LLM credential.
2. **Network egress to the model.** The container's egress policy must allow the
   provider host (e.g. `api.anthropic.com`), or point HooCode at an in-cluster
   OpenAI-compatible endpoint. A fully air-gapped pod cannot reach a hosted LLM.
3. **A writable path for config/sessions.** With `readOnlyRootFilesystem: true`,
   mount a writable volume (e.g. an `emptyDir`) for `~/.hoocode` — or set
   `HOOCODE_CODING_AGENT_DIR` to one — and either run with `--no-session` or
   point `--session-dir` at a writable location. Combine with `HOOCODE_OFFLINE=1`
   to skip all startup network operations.

Base image note: the prebuilt standalone binary is dynamically linked against
**glibc**, so it does not run on musl-based images (Alpine) or `static`
distroless. Use a glibc base (`debian:*-slim`, `gcr.io/distroless/nodejs*`), or
install via npm (`npm i -g @kolisachint/hoocode-agent`) on any image with
Node ≥ 20.

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines and
[AGENTS.md](../AGENTS.md) for project-specific rules (for both humans and
agents).
