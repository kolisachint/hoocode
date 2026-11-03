# Install

hoocode runs on **macOS and Linux, on x64 and arm64**. The Linux builds are static
(musl), so one binary runs on any distribution. There is no Windows build yet, and
hoocode is not on crates.io or Homebrew.

## Install script

No root, no Node:

```bash
curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh
```

It downloads the release for your platform, checks it against the release's
`SHA256SUMS`, installs `~/.hoocode/bin/hoocode` with a `hoo` alias, and adds
`~/.hoocode/bin` to your `PATH`.

| Flag | Environment variable | What it does |
|---|---|---|
| `--version <tag>` | `HOOCODE_VERSION` | Install a specific release (default: latest) |
| `--dir <path>` | `HOOCODE_INSTALL_DIR` | Install root (default: `~/.hoocode`) |
| `--no-modify-path` | `HOOCODE_NO_MODIFY_PATH=1` | Leave your shell rc files alone |
| `-h`, `--help` | | Show the options |
| | `HOOCODE_RELEASE_BASE_URL` | Fetch archives from a mirror instead of GitHub |

Pass flags through `sh -s --`:

```bash
curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh -s -- --no-modify-path
```

The script is [`install/install.sh`](../../install/install.sh) in the
repository, and the copy served at that URL is identical.

## First run

```bash
cd your-project
hoocode                       # asks before Shell, Edit and Write
hoocode --skip-permissions    # runs every tool without asking
```

On its first start hoocode writes `~/.hoocode/hoo-config.json` with the default mode
rules. It starts in **build** mode: Shell, Edit and Write ask before they run, and
read-only commands such as `ls`, `cat`, `rg` and `git status` run without asking.

**Skip permissions.** `hoocode --skip-permissions` runs every tool without asking; the
header shows SKIP PERMISSIONS while it is on. To make it the default, set
`"skip_permissions": true` in `~/.hoocode/hoo-config.json` (or export
`HOOCODE_SKIP_PERMISSIONS=1`). The mode rules still apply: ask and debug never edit
files. A project's own `.hoocode/hoo-config.json` cannot turn it on. See
[modes and approvals](configuration.md#modes-and-approvals).

Then sign in: run `/login` in the session, or set a provider key such as
`ANTHROPIC_API_KEY` or `OPENAI_API_KEY` before starting. See the
[quickstart](quickstart.md).

## npm or bun

The package pulls in only the native binary for your platform. Its `hoocode` and `hoo`
commands are a small `sh` script that runs that binary directly, so Node is not needed
to run hoocode and the installed command leaves no Node process running.

Supported: npm, pnpm, Yarn classic (1.x) and bun, on macOS and Linux (x64 and arm64).

- Yarn 2+ (Berry) is not supported. It runs package commands with `node`, which
  cannot run an `sh` script.
- Windows is not supported; use WSL. npm refuses the install there (`EBADPLATFORM`).

```bash
npm install -g hoocode     # or: bun add -g hoocode
```

## pip

Python 3.8 or newer. A thin launcher: on first run it downloads the release binary,
checks its sha256 and caches it under `~/.cache/hoocode/`.

```bash
pip install hoocode        # or: pipx install hoocode
```

## Release archives

Every [GitHub release](https://github.com/kolisachint/hoocode/releases) has
`hoocode-<target>.tar.gz` and a `SHA256SUMS` file, for these targets:

- `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`
- `x86_64-apple-darwin`, `aarch64-apple-darwin`

```bash
tar xzf hoocode-aarch64-apple-darwin.tar.gz
install -m 755 hoocode-aarch64-apple-darwin/hoocode ~/.local/bin/hoocode
```

## From source

Rust 1.88 or newer.

```bash
git clone https://github.com/kolisachint/hoocode
cd hoocode
cargo install --locked --path crates/hoocode-code-main --bin hoocode
```

## Updating

`hoocode update` prints the right update command for the way you installed it.
