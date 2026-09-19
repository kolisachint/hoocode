# Install

HooCode ships two ways: a **standalone binary** with no runtime to install, and
an **npm package** for machines that already have Node. Both give you the same
`hoocode` (and `hoo`) command.

## One-click install

### macOS and Linux

```bash
curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh
```

### Windows

```powershell
irm https://kolisachint.github.io/hoocode/install.ps1 | iex
```

That is the whole thing. The installer:

1. Detects your platform — including **musl vs glibc**, which decides which
   Linux build you get. Alpine and `static` distroless images are supported.
2. Downloads the matching standalone binary from the latest GitHub release and
   verifies its SHA256 against the release's `checksums.txt`.
3. Installs into `~/.hoocode/lib/hoocode` and links `~/.hoocode/bin/{hoocode,hoo}`.
4. Pre-seeds the [external tools](#external-tools) into `~/.hoocode/bin`, which
   is exactly where HooCode looks for them — so your first run is fast and works
   offline.
5. Adds `~/.hoocode/bin` to your `PATH`.

Nothing needs root or administrator rights. Everything lands under your own home
directory, and re-running the installer upgrades in place.

### Installer options

Both installers take the same options, as flags or as environment variables.

| Flag | Environment variable | Effect |
|------|---------------------|--------|
| `--version v1.2.3` | `HOOCODE_VERSION` | Install a specific release (default: latest) |
| `--dir <path>` | `HOOCODE_INSTALL_DIR` | Install root (default: `~/.hoocode`) |
| `--no-tools` | `HOOCODE_SKIP_TOOLS=1` | Skip the external tools |
| `--tools fd,rg` | `HOOCODE_TOOLS` | Pre-seed only these tools |
| `--no-modify-path` | `HOOCODE_NO_MODIFY_PATH=1` | Leave shell rc files / `PATH` alone |
| — | `HOOCODE_RELEASE_BASE_URL` | Fetch archives from a mirror instead of GitHub |

`HOOCODE_RELEASE_BASE_URL` expects the same layout a release has —
`<base>/<tag>/hoocode-<target>.tar.gz` and `<base>/<tag>/checksums.txt` — so an
air-gapped or bandwidth-limited network can mirror a release once and point every
machine at it.

A piped script cannot take arguments, so to pass options either set the
environment variable:

```bash
HOOCODE_SKIP_TOOLS=1 sh -c "$(curl -fsSL https://kolisachint.github.io/hoocode/install.sh)"
```

or download the script first:

```bash
curl -fsSL https://kolisachint.github.io/hoocode/install.sh -o install.sh
sh install.sh --no-tools --version v1.2.3
```

```powershell
irm https://kolisachint.github.io/hoocode/install.ps1 -OutFile install.ps1
.\install.ps1 -NoTools
```

## Install from npm

Needs **Node.js ≥ 20**. Works on any platform Node runs on, including ones with
no prebuilt binary.

```bash
npm install -g @kolisachint/hoocode-agent
hoocode --help
```

`pnpm add -g`, `yarn global add`, and `bun install -g` all work too — HooCode
detects which one installed it and offers the matching self-update command.

## Download a binary by hand

Every release publishes a standalone archive per platform, plus a
`checksums.txt` covering all of them.

| Archive | Platform |
|---------|----------|
| `hoocode-linux-x64.tar.gz` | Linux x86_64, glibc |
| `hoocode-linux-arm64.tar.gz` | Linux aarch64, glibc |
| `hoocode-linux-x64-musl.tar.gz` | Linux x86_64, musl (Alpine) |
| `hoocode-linux-arm64-musl.tar.gz` | Linux aarch64, musl (Alpine) |
| `hoocode-darwin-x64.tar.gz` | macOS Intel |
| `hoocode-darwin-arm64.tar.gz` | macOS Apple Silicon |
| `hoocode-windows-x64.zip` | Windows x86_64 |

Windows on ARM runs the x64 build under emulation; there is no native arm64
Windows binary yet, because the compiler has no such target.

Grab them from the [releases page](https://github.com/kolisachint/hoocode/releases),
verify, and unpack:

```bash
sha256sum -c checksums.txt --ignore-missing
tar -xzf hoocode-linux-x64.tar.gz -C ~/.hoocode/lib/hoocode
```

The binary expects the rest of the archive (themes, docs, canvas SDK, examples)
to sit beside it, so unpack the whole thing rather than pulling out just the
executable.

## External tools

Five optional Rust binaries. HooCode works without every one of them — the
installer pre-seeds them so that is never the reason your first session is slow.

| Tool | What it adds | Without it |
|------|--------------|-----------|
| `rg` | Fast content search | A pure-JS scanner: same results, slower on large trees |
| `fd` | Fast filename search | A JS directory walker: same results, slower |
| `embsearch` | Semantic ranking in search and capability lookup | Search is lexical-only; nothing errors |
| `webtools` | `webfetch` and `websearch` | Those tools error when called (the group is off by default) |
| `voicetools` | Push-to-talk voice input | Voice never starts; typing is unaffected |

HooCode fetches whichever of these it needs on demand anyway, so `--no-tools` is
a valid choice — it only trades a slower first run.

Already have `fd` or `rg` from your package manager? HooCode uses whatever is on
`PATH` before it downloads anything.

## Restricted and offline environments

HooCode runs with no network access.

- **`HOOCODE_OFFLINE=1`** (or `--offline`) disables every startup network
  operation — no binary downloads, no version checks. Search and file
  autocomplete fall back to the built-in pure-JS implementations, so both keep
  working.
- **`HOOCODE_NATIVE_SEARCH=1`** forces the pure-JS path even when `fd`/`rg` are
  available. It also engages automatically when they are not.
- **Pre-seed the binaries** for native speed offline: install them from your OS
  package manager, or run the installer once on a connected machine and copy
  `~/.hoocode/bin` across.

The interactive UI never blocks on these downloads — it starts immediately and
wires `fd` in once resolved, so a slow or blocked network never delays launch.

### Containers and Kubernetes

HooCode runs in containers, including as `root` and with a read-only root
filesystem (config-directory writes fail silently rather than crashing). Three
prerequisites are inherent to running an LLM agent and are not things HooCode
can work around:

1. **A provider credential.** Set an API key (`ANTHROPIC_API_KEY`,
   `OPENAI_API_KEY`, …) or, for Copilot, the explicit `COPILOT_GITHUB_TOKEN`. A
   bare `GH_TOKEN`/`GITHUB_TOKEN` is *not* treated as an LLM credential.
2. **Network egress to the model.** The container's egress policy must allow the
   provider host (e.g. `api.anthropic.com`), or point HooCode at an in-cluster
   OpenAI-compatible endpoint. A fully air-gapped pod cannot reach a hosted LLM.
3. **A writable path for config and sessions.** With
   `readOnlyRootFilesystem: true`, mount a writable volume (an `emptyDir` will
   do) for `~/.hoocode` — or set `HOOCODE_CODING_AGENT_DIR` to one — and either
   run with `--no-session` or point `--session-dir` somewhere writable. Combine
   with `HOOCODE_OFFLINE=1` to skip all startup network operations.

Alpine and other musl images are supported by the `-musl` archives; the
installer picks the right one for you.

## Build from source

bun is the toolchain. It is pinned to the npm-compatible **hoisted** linker in
`bunfig.toml`, so it produces a flat `node_modules`. `bun.lock` is the
authoritative lockfile.

```bash
git clone https://github.com/kolisachint/hoocode.git
cd hoocode

bun install          # install all dependencies
bun run build        # build all packages
bun run check        # lint, format, and type check
./test.sh            # run tests (skips LLM-dependent tests without API keys)
```

To produce the release archives locally:

```bash
./scripts/build-binaries.sh                      # every target
./scripts/build-binaries.sh --targets linux-x64  # just one
./scripts/build-binaries.sh --list               # what targets exist
```

`bun build --compile` cross-compiles, so one machine builds every platform's
archive.

## Uninstall

```bash
rm -rf ~/.hoocode          # binary, tools, config, sessions and auth
```

Then drop the `~/.hoocode/bin` line the installer added to your shell rc file.
On Windows, remove `%USERPROFILE%\.hoocode` and the matching entry from your
user `PATH`.

If you installed from npm: `npm uninstall -g @kolisachint/hoocode-agent`.

---

Something here wrong or missing? [Open an
issue](https://github.com/kolisachint/hoocode/issues/new/choose) — install
problems are the most useful bug reports we get, because they are the ones that
stop people before they start.

And if HooCode earns its place in your terminal, [a
star](https://github.com/kolisachint/hoocode) is the cheapest way to say so. ★
