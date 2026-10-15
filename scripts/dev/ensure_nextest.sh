#!/usr/bin/env bash
# Ensure `cargo nextest` is installed. Used by the SessionStart hook
# (.claude/settings.json) so the test loop (`cargo nextest run -p <crate>`,
# CLAUDE.md) works in cloud sessions.
#
# - Does nothing when `cargo nextest --version` already works.
# - Otherwise installs, in order: cargo-binstall (if present), the official
#   prebuilt tarball from get.nexte.st, then `cargo install --locked`.
# - Never fails the session: on any failure it prints one warning line and
#   exits 0. TLS verification is never disabled; curl and cargo use the
#   proxy from HTTPS_PROXY and the CA bundle set up by the environment.
#
# Usage: scripts/dev/ensure_nextest.sh
set -euo pipefail

BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"

warn() {
  echo "ensure_nextest: warning: $* (tests fall back to cargo test)" >&2
  exit 0
}

# Prints the first line of `cargo nextest --version`, or nothing on failure.
nextest_version() {
  local out
  out="$(cargo nextest --version 2>/dev/null)" || return 0
  printf '%s\n' "$out" | head -n 1
}

has_nextest() {
  [[ -n "$(nextest_version)" ]]
}

install_binstall() {
  command -v cargo-binstall >/dev/null 2>&1 || return 1
  cargo binstall -y --no-confirm cargo-nextest >/dev/null 2>&1
}

install_tarball() {
  local url tmp
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) url="https://get.nexte.st/latest/linux" ;;
    Linux-aarch64) url="https://get.nexte.st/latest/linux-arm" ;;
    Darwin-*) url="https://get.nexte.st/latest/mac" ;;
    *) return 1 ;;
  esac
  command -v curl >/dev/null 2>&1 || return 1
  command -v tar >/dev/null 2>&1 || return 1

  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" RETURN
  curl -fsSL --retry 3 --proto '=https' --tlsv1.2 -o "$tmp/nextest.tar.gz" "$url" || return 1
  tar -xzf "$tmp/nextest.tar.gz" -C "$tmp" || return 1
  [[ -f "$tmp/cargo-nextest" ]] || return 1
  mkdir -p "$BIN_DIR"
  install -m 0755 "$tmp/cargo-nextest" "$BIN_DIR/cargo-nextest"
}

install_cargo() {
  command -v cargo >/dev/null 2>&1 || return 1
  cargo install --locked --quiet cargo-nextest
}

main() {
  # Outside Claude Code cloud sessions the hook still runs, but this check
  # returns at once when nextest is already there.
  if [[ -n "${CLAUDE_CODE_REMOTE:-}" && "${CLAUDE_CODE_REMOTE}" != "true" ]]; then
    exit 0
  fi

  if has_nextest; then
    echo "ensure_nextest: $(nextest_version)"
    exit 0
  fi

  command -v cargo >/dev/null 2>&1 || warn "cargo not found; cannot install cargo-nextest"

  # PATH may not yet include the install dir in this shell.
  export PATH="$BIN_DIR:$PATH"

  echo "ensure_nextest: cargo-nextest missing, installing" >&2
  if install_binstall && has_nextest; then
    :
  elif install_tarball && has_nextest; then
    :
  elif install_cargo && has_nextest; then
    :
  else
    warn "could not install cargo-nextest (binstall, tarball and cargo install all failed)"
  fi

  echo "ensure_nextest: $(nextest_version)"
}

main "$@"
