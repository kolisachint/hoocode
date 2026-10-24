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

## npm or bun

Node 18 or newer. The package pulls in only the native binary for your platform.

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
