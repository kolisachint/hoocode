# Fuzzing

Fuzz targets for the parsers that read bytes from outside the process: provider
streams, session and settings files, RPC framing, terminal input and markdown. Plan
step 1.6 ([reliability.md](../docs/design/reliability.md), item 5).

This directory is its own cargo workspace (`exclude = ["fuzz"]` in the root
`Cargo.toml`). The main build never touches it, and it needs **nightly** plus
`cargo-fuzz`.

## Layout

| Path | What |
|---|---|
| `fuzz_targets/<name>.rs` | libFuzzer entry point (one binary per target) |
| `targets/<name>.rs` | the target body, `pub fn run(data: &[u8])`, shared with the stable tests |
| `smoke.rs` | stable driver: seeds, their prefixes, deterministic mutations |
| `corpus/<name>/` | checked-in seed inputs (cargo-fuzz also writes new findings here) |

Each target body is compiled twice: into the fuzz binary, and via `#[path]` into a
`fuzz_smoke` test of the crate that owns the parser (`crates/*/tests/**/fuzz_smoke.rs`).
So `cargo test --workspace fuzz_smoke` runs every target body on stable Rust on each
push, with no nightly.

| Target | Parser | Owning crate |
|---|---|---|
| `sse` | SSE decoder, and the same events for any chunk split | `hoocode-ai-sse` |
| `rpc_jsonl` | RPC LF framing, chunk invariance, JSON round trip | `hoocode-code-rpc` |
| `session_jsonl` | session file load and entry migration | `hoocode-code-session` |
| `settings_json` | global and project `settings.json` | `hoocode-code-settings` |
| `models_json` | custom `models.json` (comments, trailing commas) | `hoocode-code-models` |
| `tui_keys` | legacy, Kitty and modifyOtherKeys key input | `hoocode-tui-keys` |
| `js_regex` | JS RegExp emulation used by the markdown lexer | `hoocode-tui-util` |
| `ansi_wrap` | ANSI-aware wrap, truncate and strip | `hoocode-tui-util` |
| `markdown` | markdown lexer and renderer (list items included) | `hoocode-tui-components` |

## Run

```bash
rustup toolchain install nightly
cargo install cargo-fuzz --locked                    # once
cargo +nightly fuzz run sse                          # run until stopped
cargo +nightly fuzz run js_regex -- -max_total_time=300
cargo +nightly fuzz list                             # all targets
```

Run from the repository root; `cargo fuzz` finds `fuzz/` itself.

Stable smoke tests (no nightly):

```bash
cargo test --workspace fuzz_smoke
```

## Findings

A crash is saved under `fuzz/artifacts/<target>/`. Reproduce it with
`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>`, then add the input
as a seed in `corpus/<target>/` and a regression test in the owning crate.

- `js_regex`: a `\u` escape with fewer than four characters after it, and `\x`, `\p`
  or `\u{` without room or a closing brace, indexed past the end of the pattern.
  Fixed in `hoocode-tui-util` (`translate` treats them as identity escapes, as
  JS Annex B does). Regression: `truncated_escapes_are_identity_escapes_not_panics`.

## Adding a target

1. Write `targets/<name>.rs` with `pub fn run(data: &[u8])`. It must not panic on any
   input, and it may assert round trips and invariants.
2. Add `fuzz_targets/<name>.rs` (copy an existing one), a `[[bin]]` in `Cargo.toml`,
   and a `corpus/<name>/` seed directory with a few small valid and broken inputs.
3. Add `crates/<owner>/tests/.../fuzz_smoke.rs` with a `#[test]` that calls
   `smoke::run("<name>", <name>::run)`, and register the module in that crate's test
   entry point. Add the target to `archive/migration/ci/fuzz.patch` as well.
4. Update the table above and `docs/maps/packages.md` if a crate changed.
