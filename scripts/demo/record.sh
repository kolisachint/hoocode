#!/usr/bin/env bash
# Re-record the README demo (docs/assets/demo.gif) with the current build.
# See scripts/demo/record.py for what it does and what it needs.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cargo build --quiet --release --manifest-path "$ROOT/Cargo.toml" -p hoocode-code-main --bin hoocode
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps --manifest-path "$ROOT/Cargo.toml" \
  | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')"
HOOCODE_BIN="${HOOCODE_BIN:-$TARGET_DIR/release/hoocode}" exec python3 "$ROOT/scripts/demo/record.py" "$@"
