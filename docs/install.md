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

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines and
[AGENTS.md](../AGENTS.md) for project-specific rules (for both humans and
agents).
