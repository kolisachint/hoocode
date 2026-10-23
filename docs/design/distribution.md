# Distribution: how the Rust hoocode reaches users

Status: **shipped** (updated 2026-10-22): archives, npm, PyPI, the curl installer and
the site docs. This page records what ships today and the work deliberately left
for later, so the scope of each step stays small.

## Names

| Command | Build | Repo | npm |
|---|---|---|---|
| `hoocode`, `hoo` | Rust (this repo, cargo binary `hoocode`) | kolisachint/hoocode | `hoocode` (primary), `@kolisachint/hoocode` (alias) |

The TypeScript build (`hoocode-ts`, `hoo-ts`) was retired on 2026-10-10; hoocode replaces it.

## Shipped

Every release made by `release.yml` (a merged PR with a `rust:*` label):

1. **Archives**, built by `binaries.yml`: `hoocode-<rust-target>.tar.gz` for
   `x86_64`/`aarch64` × `apple-darwin`/`unknown-linux-musl`. Linux builds are
   static, so one binary runs on every distro.
2. **`SHA256SUMS`** covering the archives.
3. **PyPI**: `hoocode`, a thin downloader wheel. It installs a `hoocode`
   console script that, on first run, downloads the archive matching the
   package version and platform, verifies its sha256 against the release
   `SHA256SUMS`, extracts the binary into a user cache dir, and execs it.
   Built by `scripts/pypi/build_pypi.py`, published with twine by
   `scripts/pypi/publish_pypi.py` using the `PYPI_TOKEN` / `PYPI_API_TOKEN`
   secret. Re-runs skip versions that already exist. After uploading, the
   script checks the version against the PyPI JSON API and fails the job if
   it has not turned up: a successful upload is installable only after
   index propagation, and without this a partial publish looks identical
   to a successful one.
4. **npm**: `hoocode` (primary install name; `@kolisachint/hoocode` is published too, as an alias with the same version) plus `@kolisachint/hoocode-{darwin,linux}-{arm64,x64}`.
   The main package is a small Node launcher (`npm/hoocode/bin/hoocode.js`);
   the platform packages are `optionalDependencies` gated by `os`/`cpu`, so
   npm and bun fetch only the one for the machine. No postinstall script.
   Built by `scripts/npm/build_packages.py`, published with provenance by
   `scripts/npm/publish_packages.py` using the `NPM_TOKEN` secret. Re-runs skip
   versions that already exist. After publishing, the script checks every
   package against the registry packument and fails the job if any has not
   turned up: a fresh publish is readable only after registry propagation
   (`v0.1.5`'s `darwin-x64` took ~11 minutes), and `npm publish` reporting
   success does not mean `npm install` can resolve it yet. Verify a published
   release with
   `python3 scripts/npm/publish_packages.py <dist-dir> --verify-only`.
5. **curl installer**: `install/install.sh`, served at
   `https://kolisachint.github.io/hoocode/install.sh`. The site repo
   (kolisachint.github.io) serves a verbatim copy in `public/hoocode/`, synced
   automatically from this repo by the site's `scripts/sync-docs.mjs`.
   It verifies `SHA256SUMS`, test-runs the binary before replacing anything,
   installs into `~/.hoocode/bin`, and removes links left by an old
   TypeScript install (`~/.hoocode/lib/hoocode`).

## Done since 2026-10-01

| Item | Resolution |
|---|---|
| **Site docs** | Resolved: the site at `/hoocode/` has short Rust docs written on the site, and no longer offers the hoocode-ts installers. |
| **Auto-sync of `install.sh`** | Done: the site's `scripts/sync-docs.mjs` copies `install/install.sh` from this repo into `public/hoocode/`. |

## Deferred

| Item | Why deferred | What it needs |
|---|---|---|
| **Windows** | The Windows leg was the slowest part of every release. There is no Windows build; the site offers none. | Add `x86_64-pc-windows-msvc` back to `binaries.yml`, a `.zip`, an `install.ps1`, a `win32-x64` npm package. |
| **crates.io** — **dropped 2026-10-08** | Not needed: users get archives, npm and curl. The umbrella crates are deleted. Was: 34 of 42 crates are unpublished; a first publish hits crates.io rate limits (run 29692981384). `release:crates` only leaves a notice. | A paced, resumable `scripts/publish_packages.py` run, then re-enable the `crates` job in `release.yml`. |
| **macOS signing** | Binaries carry only the linker's ad-hoc signature. curl, npm and `gh` downloads are not quarantined, so they run; a browser download triggers "Apple could not verify". | An Apple Developer ID, `codesign` + `notarytool` in `binaries.yml`. |
| **Self-update** | `hoocode update` does not know which of curl or npm installed it. | Detect the install method (path under `~/.hoocode/bin` vs `node_modules`) and print the matching command. |
