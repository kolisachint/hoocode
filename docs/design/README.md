# Design docs

Status (2026-11-06): Start here. Six area docs, one plan. Engineering rules below are in force. Open items: EN-1 to EN-13.

## What hoocode is and how these docs work

hoocode is a coding agent for the terminal: a Rust CLI with a full-screen TUI, print and JSON modes, an rpc mode and an app server. These docs say what is built, what to build next, and the rules that must hold. Each area doc holds its principles, a short "How it works", its decisions and its open items.

| File | Covers | Read it for |
|---|---|---|
| `README.md` (this file) | How we work, design philosophy, standards, engineering: build, CI, tests, release, naming, licence | The rules and the build |
| [roadmap.md](roadmap.md) | The one plan: built summary per area, next steps in order, deferred items with triggers, not doing | What to build next |
| [runtime.md](runtime.md) | Concurrency, lanes, caps, memory limits, reliability, performance | Threads, limits, speed |
| [agent.md](agent.md) | Agent loop and tools, models, scheduler and `/loop`, extras, web tools, DocSearch, MCP, plugins and skills, extensions, app server and rpc | The agent and its integrations |
| [subagents.md](subagents.md) | Subagents, orchestration, evals | Child agents |
| [tui.md](tui.md) | TUI: declutter (the current look), screen anatomy, themes, keybindings | The terminal UI |
| [../maps/packages.md](../maps/packages.md) | Crate map | Where code lives |
| [../maps/ui.md](../maps/ui.md) | Screens, pickers, slash commands | Where UI code lives |
| [../user/](../user/) | User pages: flags, commands, paths, install | What users see |

Rules for these docs:

- **Design first.** A design session changes docs only. No code until the design is agreed.
- **Decisions go in the area doc.** Add a dated row to its Decisions table. There are no dated decision pages. Remove a row when it stops holding.
- **Ask before you build.** When the user asks to implement, ask which roadmap item to start with first.
- **Code wins.** Where a doc and the code disagree, the code is what is built. Record the built behaviour as the decision.
- **Maps and user docs move with the code.** Change `docs/maps/` and `docs/user/` in the same commit as any crate, screen, command, flag, path or install change.

## How we work (user preferences)

- **Recommendation first.** Put open questions to the user as choices, recommended option first. Give the trade-offs and the risk each answer creates.
- **Standards first, no bloat.** Prefer open standards (see the table below). Use a vendor format only where the standard cannot cover it. Build for where the standards are going, not for legacy. Example: drop deprecated MCP transports.
- **Reliability before features.**
- **Never drop an idea to narrow scope. Tier it** as Now, Next or Later, each with its trigger. Verified non-issues go in Not doing, with the reason.
- **No new always-on token cost.** A tool that adds a schema is opt-in, or appears only when used.
- **Licences: MIT only for copied code.** Never copy code, schemas or generated types from non-MIT projects (for example `../codex`, Apache-2.0). Running such tools in tests is fine. Depending on a permissively licensed crate is fine. The user's own repos (`webtools`, `embeddingsearchtools`, `voicetools`, `hooteams`, `drawio-canvas`) are MIT and fair to use.
- **TUI work is done by Haiku subagents** (user, 2026-10-09). The main session briefs, checks and merges. Haiku subagents review, implement and test. Visual review is a text grid plus a style legend. Use a PNG only for a changed or uncertain screen.
- **Style.** Plain words. Short sentences. Tables over prose. No history or background essays (git keeps old drafts). No marketing.

Area doc template:

```
# <Area>
Status (YYYY-MM-DD): one or two lines.
## Principles   durable rules and invariants, with real numbers and names
## How it works what is built: terse facts, crates, files, config keys, formats
## Decisions    table Date | Decision; only decisions still in force
## Open         table ID | Item | Size | Trigger or gate | Design notes
## Not doing    one bullet list
```

Sizes: S is about a session, M a few, L many. Open IDs use the area prefix (EN- here).

## Standards

| Standard | State | Used in |
|---|---|---|
| MCP (AAIF), spec 2026-07-28 | Stateless, `server/discover`, multi round-trip input, extensions. Deprecated: HTTP+SSE, Roots, Sampling, Logging, Dynamic Client Registration. The client is built on rmcp, the official SDK | agent.md |
| MCP Apps and Tasks extensions (AAIF) | Official. Deferred, not used | roadmap.md (only if needed) |
| MCP Skills extension (AAIF) | Dropped 2026-10-08 | not used |
| Agent Plugins 1.0.0 (agent-plugins.org) | Published 2026-08-06: `plugin.json`, `skills/`, `mcp.json`. hoocode writes Agent Plugins only; it reads Claude plugin formats and `marketplace.json` too | agent.md |
| Agent Skills (agentskills.io) | `SKILL.md` frontmatter rules | agent.md |
| AGENTS.md (AAIF) | Supported | agent.md |
| A2A (AAIF) v1.0 | No local or stdio story | not used |

## Engineering

Status (2026-11-06): Built: the done bar, the CI jobs and guards, the release pipeline and four shipped channels. Open: EN-1 to EN-13 below.

### Principles

- **Each step ends green.** After each step, `cargo fmt --all`, clippy with `-D warnings`, and nextest for the touched crates all pass. Steps are small, atomic and revertable.
- **Volatility isolation.** Volatile third-party crates are used only by the adapter crates listed in `scripts/ci/dep-firewall.json`. Adapters expose hoocode-owned types. The `firewall` CI job enforces it. Upgrading or swapping one of them touches only the listed crates. Never add to the `pending` list without a task ID.
- **Scope rule.** Keep what the standards cover and what is used. Drop vendor-only extras and code nothing calls.
- **One name: hoocode.** Crates are `hoocode-*`, the binary is `hoocode`, the user data is `~/.hoocode`, and project data is `<repo>/.hoocode/`. Environment variables use the `HOOCODE_` prefix only, with no aliases. Do not bring back the pre-rename app name: `scripts/ci/no_cortex.sh` fails the `guards` job.
- **Agents read `.agents`.** Read skills, commands, agents, AGENTS.md and MCP config from `~/.agents` and `<project>/.agents`. Write nothing there except `<project>/.agents/scheduled_tasks.json`.
- **Lockstep versions.** Every crate inherits the workspace version (`version.workspace = true`). A release bumps it once.
- **Errors are hand-written enums** that implement `std::error::Error`. No `anyhow`, no `thiserror`.
- **Code style.** Owned types, `Arc` and `.clone()` over lifetimes in structs. Plain `async fn`, also in traits. Avoid hand-written futures and deep combinator chains.
- **New dependency.** Check the resolved version in `Cargo.lock`. Read the API from `cargo doc` or the source of that exact version, not from memory.
- **Panics on user or model input are bugs.** Slice text through `text_slice` (`crates/hoocode-tui-util`). Rewrite only the `unwrap`s reachable from input.
- **macOS is a supported dev platform.** Its test failures are bugs.
- **Flaky tests are fixed, never quarantined.** Wait on final content, not on fixed quiet periods.
- **Measure before tuning.** Speed work starts with a measurement. Record numbers with the date and the change that moved them.
- **Disk.** Prune `target/` at session start. Never `cargo clean` unless the user asks.

### How it works

**Done bar and agent loop**

- Done means: `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo nextest run -p <touched crates>`. CI runs the full workspace.
- After an edit: `cargo check -p <crate> --message-format=short`. Build only to run tests.
- Lints: run `cargo clippy --fix --allow-dirty -p <crate>` first, then fix the rest by hand.
- Integration tests live in `crates/<crate>/tests/it/<name>.rs`, registered as `mod <name>;` in `tests/it/main.rs`. Each crate has one `it` test binary. Helpers go in `tests/it/{common,support}/`. The `test-layout` CI job fails on any other `crates/*/tests/*.rs`.
- Doctests: 75 of 76 crates set `doctest = false`. CI runs no doctest step (see EN-6).
- Subagent evals: `python3 scripts/eval/subagent_evals.py --include-slow` (real binary, mock LLM). Run them before a subagent change ships.

**Tests and goldens**

- nextest profiles: `default` retries 2; `ci` retries 0, flags a test every 60 s and kills it after 5 min. Subagent tests retry 3.
- Component goldens run in nextest. `UPDATE_GOLDENS=1` accepts them.
- Screen goldens: about 10 scenarios, `python3 scripts/tui/goldens.py check all` (tmux, mock LLM). This is a pre-release check, not a per-change one. `python3 scripts/tui/goldens.py update <scenario>` accepts a deliberate change. Reports go to `target/tui-goldens/`. CI runs them nightly at 03:17 UTC (`tui-parity.yml`) and by hand.

**Build and profiles**

- The bottleneck is linking and running test binaries, not compiling. The linker is LLD, the Rust default on x86_64 Linux since 1.90.
- `rust-toolchain.toml` pins 1.94.1, so fmt and clippy match CI. `rust-version = "1.88"` in `Cargo.toml` is the MSRV. No CI job checks it (EN-7).
- Dev profile: `debug = false`. For one crate, add `[profile.dev.package.<crate>] debug = "line-tables-only"`. The regex crates (`regex-automata`, `regex-syntax`, `fancy-regex`) build at opt-level 3, because the markdown rules take about 1 s to build unoptimised.
- Release profile: `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`, `panic = "unwind"`. Unwind stays on: `run_blocking` turns a tool panic into an error.
- Environments: cloud sessions are Linux x86_64, 4 vCPU, 15 GB, ephemeral, with a fixed disk. Local is macOS arm64. CI is GitHub-hosted; the repo is public, so runner minutes are free.
- Local macOS setup (manual, once): allow the terminal in Gatekeeper developer mode; exclude `target/` in Spotlight; set rust-analyzer `targetDir = true`; keep the default linker.

**Session hooks (`.claude/settings.json`, SessionStart)**

1. `scripts/dev/ensure_nextest.sh` installs cargo-nextest if it is missing (binstall, then the prebuilt tarball, then `cargo install --locked`). It exits 0 on failure.
2. `scripts/dev/prune_target.sh --apply --if-over 15 --quiet`. When `target/` is over 15 GB, it deletes dependency units, fingerprints and build entries older than 3 days and stale units under the pre-rename names. An oversized incremental cache keeps only the newest session.
3. `scripts/dev/warm_up.sh` starts a detached `nextest --no-run` build and returns at once. It is skipped when a stamp (`Cargo.lock` plus `rustc -V`) younger than 3 days matches, when free disk is under 8 GB, or with `HOOCODE_WARMUP=0`. While it runs it holds cargo's build lock, so cargo commands wait.

The Stop hook (clippy per touched crate) and the PostToolUse hook (rustfmt) are not wired (EN-1).

**CI (`.github/workflows/ci.yml`)**

Runs on push and pull request to `main`, nightly at 04:17 UTC, and when `release.yml` calls it. Superseded PR runs are cancelled. Every cargo command uses `--locked`. `CARGO_INCREMENTAL=0`. The toolchain comes from `rust-toolchain.toml`.

| Job | Runs | On PRs |
|---|---|---|
| fmt | `cargo fmt --all -- --check` | yes |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | yes |
| test | `cargo nextest run -P ci --locked --workspace --no-fail-fast` | yes |
| test-macos | the same command on `macos-latest` | yes |
| doc | `cargo doc --locked --workspace --no-deps` | yes |
| test-layout | fails on any test file outside `tests/it/` | yes |
| guards | `scripts/ci/no_cortex.sh`; `scripts/ci/no_literal_keys.py` (key names in UI text come from the keymap); `scripts/maps/packages.py --check` (crate map) | yes |
| firewall | `scripts/ci/check_dep_firewall.py`; `install/install.sh` and the npm launcher parse under sh and dash; launcher tests run under sh and dash | yes |
| subagent-evals, subagent-evals-nightly | fast eval suite; nightly fault matrix and 10-minute soak | no: schedule or by hand |

Other workflows: `fuzz.yml` (nightly 03:37 UTC, 9 fuzz targets); `tui-parity.yml` (goldens, see above); `models-sync.yml` (Mondays 06:00 UTC, opens a PR that refreshes `models.json`; the bot merges nothing).

**Release**

Three ways to release:

1. A merged PR with label `rust:patch`, `rust:minor` or `rust:major`. `merge-release.yml` (on push to main) calls `release.yml`: gates (the CI workflow; skippable), then bump, `cargo update --workspace`, tag, GitHub release. Then `binaries.yml` builds, then `SHA256SUMS`, npm and PyPI.
2. A `vX.Y.Z` tag pushed by hand on a `main` commit whose workspace version matches. `release-tag.yml` runs. Its manual run is a rehearsal by default (`dry_run` is on).
3. `release.yml` run by hand (`workflow_dispatch`).

Binaries are built only for labelled releases and tags. There are no nightly or per-merge builds. Each archive is `hoocode-<target>.tar.gz` and holds `hoocode`; `hoo` is a link.

| Target | Note |
|---|---|
| `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` | Static. One binary runs on every distro |
| `apple-darwin` (aarch64 and x86_64) | Both arches built on one arm64 runner |

| Channel | What | Checks |
|---|---|---|
| Archives | Four targets from `binaries.yml`, plus `SHA256SUMS` | sha256 list |
| curl installer | `install/install.sh`, served from the site and synced from this repo. Installs to `~/.hoocode/bin` | Verifies `SHA256SUMS`; test-runs the new binary before it replaces anything |
| npm | `hoocode` (primary); `@kolisachint/hoocode` (alias, same version); `@kolisachint/hoocode-{darwin,linux}-{arm64,x64}` | POSIX sh launcher `exec`s the platform binary, so no Node process stays up. No postinstall. Platform packages gated by `os` and `cpu`. Published with provenance |
| PyPI | `hoocode`: a thin downloader that fetches the matching archive on first run | Checks sha256 against `SHA256SUMS`; caches the binary |

The npm and PyPI publish scripts check the registry after upload and fail the job if the version is not visible yet (propagation can take about 11 minutes). Re-runs skip versions that already exist. Verify a release with `python3 scripts/npm/publish_packages.py <dist-dir> --verify-only`.

Not shipped: Windows (no build; the npm main package is `os`-gated), crates.io (dropped), signed macOS builds (ad-hoc signature only). Yarn Berry 2+ is unsupported: it runs bins with `node`.

### Decisions

| Date | Decision |
|---|---|
| 2026-09-24 | Volatility isolation: volatile crates sit behind hoocode-owned adapter crates, enforced by the dependency firewall |
| 2026-09-29 | Errors are hand-written enums. No `anyhow`, no `thiserror` |
| 2026-09-29 | No new crates for build speed. More crates mean more test binaries |
| 2026-10-01 | Binaries are built only for labelled releases or hand-pushed tags. No nightly or per-merge builds |
| 2026-10-01 | A release runs the CI gates first, then tags, then builds binaries |
| 2026-10-02 | One integration-test binary per crate, under `tests/it/`. The `test-layout` job enforces it |
| 2026-10-08 | Scope rule: keep what standards cover and what is used; drop vendor-only extras and code nothing calls |
| 2026-10-08 | crates.io publishing dropped. Users get archives, npm, PyPI and the curl installer |
| 2026-10-08 | The WASM plugin host crate is deleted. wasmtime is in no build |
| 2026-10-10 | `[profile.dev] debug = false`. Panic file and line still come from `Location::caller` |
| 2026-10-10 | Prune `target/` at session start. Never `cargo clean` unless asked |
| 2026-10-14 | Rename to hoocode is done: `HOOCODE_` env only, no aliases, `~/.hoocode` data, no pre-rename names |
| 2026-10-14 | The pre-rename data folders were merged once into `~/.hoocode`, by copy. Nothing reads them now. hoocode never deletes them |
| 2026-10-14 | `hoocode-agent-mcp` is kept as the home of the MCP client |
| 2026-10-14 | `.agents` is read for skills, commands, agents, AGENTS.md and MCP config. hoocode writes only the scheduled-task store there |
| 2026-10-14 | Flaky tests are fixed, never quarantined |
| 2026-10-14 | The macOS test job runs on pull requests and blocks them |
| 2026-10-15 | SessionStart runs the nextest install, the prune and the warm-up. `HOOCODE_WARMUP=0` opts out |
| 2026-10-19 | CI wiring applied: the `guards` job, the goldens workflow and the macOS PR job |
| 2026-10-29 | Release profile: thin LTO, one codegen unit, stripped symbols, unwind kept (built 2026-10-30) |
| 2026-11-04 | Plain words only in TUI text. `scripts/ci/plain_words.sh` is run by hand; it is not in CI yet (EN-8) |

### Open

| ID | Item | Size | Trigger or gate | Design notes |
|---|---|---|---|---|
| EN-1 | Wire the Stop hook and the PostToolUse hook | S | None | Stop: map `git diff --name-only HEAD` plus untracked files to `crates/<name>/`, run `cargo clippy -p <each> --all-targets -- -D warnings`, exit 2 on failure with the short errors; skip when no `.rs` changed; bound the timeout. PostToolUse: `rustfmt --edition 2021 <file>` on the edited `*.rs` only, no `cargo check` |
| EN-2 | `[workspace.lints]` | S | None | `rust.unused_must_use = "deny"`; `clippy.all = { level = "warn", priority = -1 }`; `[lints] workspace = true` in all 76 crates. No pedantic groups. CI keeps `-D warnings` |
| EN-3 | Hygiene job: `cargo machete` and `cargo deny check` | S | None | Add `deny.toml`. Neither exists today |
| EN-4 | Highlight opt-level experiment | S | Measure with EN-5 | `[profile.dev.package.hoocode-tui-highlight] opt-level = 1`. `highlight_gold` took 10.8 s in debug. Keep only if the test gain beats the slower rebuild of that crate. No blanket `"*"` override |
| EN-5 | Re-measure the build and decide on mold | M | After EN-2 and EN-4 | Measure a clean `cargo check --workspace --all-targets --timings`, a clean `cargo nextest run --workspace --no-run`, and a base-crate edit followed by both. Adopt mold only if linking is over 30% of an incremental `nextest --no-run`. Linux only, never macOS |
| EN-6 | Doctests: CI runs none | S | User picks: recommend adding a doctest step for the crates that have doctests | The earlier CI plan put `cargo test --doc` in CI; `ci.yml` `test` runs nextest only. Either add the step or drop the rule and keep `doctest = false` |
| EN-7 | MSRV job | S | None | Build with Rust 1.88, `--locked`, the way the other jobs run. Nothing checks `rust-version` today |
| EN-8 | Run `scripts/ci/plain_words.sh` in the `guards` job | S | None | The plain-words rule is enforced by hand only. Check the script's skip list after the cleanup |
| EN-9 | Self-update prints the right command for the install method | S | User asks for `hoocode update` to be fixed | Under `~/.hoocode/bin`: the curl command. Under `node_modules`: the npm command. PyPI runs from the user cache dir: the pip command. Today `hoocode-code-update` does not detect the method |
| EN-10 | Remove the dead `publish_crates` input from `release.yml` | S | Next edit to `release.yml` | The input does nothing; the job prints a notice. Or re-enable it with a paced publish script |
| EN-11 | Windows build | M | User asks for Windows | Add `x86_64-pc-windows-msvc` to `binaries.yml`, a `.zip`, an `install.ps1` and a `win32-x64` npm package. The old Windows leg was the slowest part of a release |
| EN-12 | macOS signing | M | Users hit the browser warning, and the user has an Apple Developer ID | `codesign` and `notarytool` in `binaries.yml`. Only browser downloads are quarantined; curl, npm and `gh` downloads run without it |
| EN-13 | Subagent fast evals on PRs | S | User decides | Today they run nightly and by hand only. Run them on PRs that touch `hoocode-code-subagents` |

### Not doing

- Copying code, schemas or generated types from non-MIT projects.
- `anyhow` or `thiserror`.
- sccache, until there is a remote bucket and its credentials are set as environment secrets.
- cargo-hakari (no measured thrash), bacon in the agent loop (it contends for the build lock), rust-analyzer in `rust-toolchain.toml`, the Cranelift backend, nightly `-Z threads`, a `CARGO_BUILD_JOBS` cap (until OOM during linking).
- wasmtime or any `wasm` feature in default builds.
- More crate splits for build speed.
- A blanket `[profile.dev.package."*"]` override.
- Per-merge or nightly binaries. npm postinstall scripts. Yarn Berry 2+ support.
- crates.io publishing. `fd` and `rg` as external search binaries.
- Env aliases for the pre-rename prefixes. hoocode does not edit `.gitignore` files.
- Quarantining flaky tests.
