<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="../assets/hoocode.svg">
    <img alt="HooCode" src="../assets/hoocode-light.svg" height="64">
  </picture>
</p>

<p align="center">Deterministic terminal coding agent.</p>

# Install

The full, user-facing install guide lives at
**[kolisachint.github.io/hoocode/install](https://kolisachint.github.io/hoocode/install)**
(source: [`packages/coding-agent/docs/install.md`](../packages/coding-agent/docs/install.md)).
It covers the one-click installers, npm, the standalone archives, offline and
container setups, and uninstalling.

This page is the short version, plus the bits that only matter if you are
working *on* HooCode rather than *with* it.

## One-click

```bash
# macOS and Linux
curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh
```

```powershell
# Windows
irm https://kolisachint.github.io/hoocode/install.ps1 | iex
```

The installer sources are in [`install/`](../install). They install into
`~/.hoocode`, need no root, and pre-seed the external Rust tools (`fd`, `rg`,
`embsearch`, `webtools`, `voicetools`) into `~/.hoocode/bin` — the same
directory HooCode downloads them to itself.

## From npm

Needs Node.js ≥ 20.

```bash
npm install -g @kolisachint/hoocode-agent
hoocode --help
```

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

## Build the release archives

```bash
./scripts/build-binaries.sh                          # every target
./scripts/build-binaries.sh --targets linux-x64      # just one
./scripts/build-binaries.sh --list                   # what targets exist
```

`bun build --compile` cross-compiles, so one host produces every platform's
archive. That is why the `binaries` job in
[`.github/workflows/release.yml`](../.github/workflows/release.yml) is a single
job rather than a runner matrix.

Targets: `linux-x64`, `linux-arm64`, `linux-x64-musl`, `linux-arm64-musl`,
`darwin-x64`, `darwin-arm64`, `windows-x64`. Each produces
`hoocode-<target>.tar.gz` (or `.zip` on Windows) plus a shared `checksums.txt`
that both installers verify against.

Adding a target means one row in `ALL_TARGETS` in
[`scripts/build-binaries.sh`](../scripts/build-binaries.sh) and, if it is a new
OS/arch pair, a matching branch in the platform detection in
[`install/install.sh`](../install/install.sh).

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for how to get a change in, and
[AGENTS.md](../AGENTS.md) for the project rules that apply to humans and agents
alike.
