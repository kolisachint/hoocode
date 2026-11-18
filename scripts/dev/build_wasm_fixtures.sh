#!/usr/bin/env bash
# Build the WASM test fixtures for crates/hoocode-code-wasm and copy the built
# .wasm into tests/fixtures/. The fixture sources live under fixtures/ (not
# tests/, see the CI test-layout guard). They are separate crates (excluded from
# the host workspace) built for wasm32-wasip2, which already emits a component.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fixtures="$root/crates/hoocode-code-wasm/tests/fixtures"
src="$root/crates/hoocode-code-wasm/fixtures"
target_dir="$root/target/wasm-fixtures"

cargo build \
    --manifest-path "$src/echo-ext/Cargo.toml" \
    --target wasm32-wasip2 \
    --release \
    --target-dir "$target_dir"

cp "$target_dir/wasm32-wasip2/release/echo_ext.wasm" "$fixtures/echo-ext.wasm"
echo "wrote $fixtures/echo-ext.wasm ($(wc -c < "$fixtures/echo-ext.wasm") bytes)"
