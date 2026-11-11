#!/usr/bin/env bash
# Install the Rust build as `hoocode` (and its `hoo` alias).
#
# Usage: scripts/install.sh [--prefix DIR]
#   --prefix DIR    install into DIR (default: ~/.hoocode/bin when the curl
#                   installer's install exists, else ~/.local/bin)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Update the curl install (install/install.sh: ~/.hoocode/bin, on PATH) in place
# rather than adding a second copy elsewhere that it would shadow.
CURL_BIN="${HOOCODE_INSTALL_DIR:-$HOME/.hoocode}/bin"
if [[ -x "$CURL_BIN/hoocode" ]]; then
  PREFIX="$CURL_BIN"
else
  PREFIX="$HOME/.local/bin"
fi
usage() {
  cat <<'EOF'
Usage: scripts/install.sh [--prefix DIR]

Build the release hoocode binary and install it as hoocode (and its hoo alias).

  --prefix DIR    install into DIR (default: ~/.hoocode/bin when the curl
                  installer's install exists, else ~/.local/bin)
  -h, --help      show this usage
EOF
}
# Argument errors exit 2 before anything is built.
die() { echo "error: $*" >&2; exit 2; }
while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix) [[ $# -ge 2 ]] || die "--prefix needs a value"; PREFIX="$2"; shift 2 ;;
    --prefix=*) [[ -n "${1#*=}" ]] || die "--prefix needs a value"; PREFIX="${1#*=}"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

cargo build --locked --release --manifest-path "$ROOT/Cargo.toml" --bin hoocode
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps --manifest-path "$ROOT/Cargo.toml" \
  | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')"

mkdir -p "$PREFIX"
# Copy then rename, so a running hoocode keeps its (old) file.
install -m 755 "$TARGET_DIR/release/hoocode" "$PREFIX/.hoocode.new"
mv -f "$PREFIX/.hoocode.new" "$PREFIX/hoocode"
[[ -e "$PREFIX/hoo" ]] || ln -s hoocode "$PREFIX/hoo"
echo "installed: $PREFIX/hoocode"
case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) echo "note: $PREFIX is not on PATH" >&2 ;;
esac
if [[ "$(command -v hoocode || true)" != "$PREFIX/hoocode" && -n "$(command -v hoocode || true)" ]]; then
  echo "note: another hoocode comes first on PATH: $(command -v hoocode)" >&2
  echo "      (an npm global install of the TS one?) put $PREFIX earlier on PATH" >&2
fi
