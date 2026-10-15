#!/usr/bin/env bash
# Warm-up build for the SessionStart hook (.claude/settings.json). Implements
# the warm-up step of docs/design/build-speed.md §4.1: it starts
# `cargo nextest run --workspace --no-run` in the background so the first real
# build in a session is warm.
#
# - Returns at once. The build runs detached (setsid + nohup) and logs to
#   <target>/warmup.log.
# - Idempotent: a stamp file <target>/.warmup-stamp records the Cargo.lock and
#   rustc version it was built for. The stamp is trusted for
#   HOOCODE_WARMUP_MAX_AGE_DAYS (default 3, the same as prune_target.sh), so a
#   later session re-runs the build after prune removed old units. A re-run
#   against a warm target returns in seconds.
# - Does nothing when cargo is missing, when the disk is low, when another
#   warm-up holds the lock, or when HOOCODE_WARMUP=0.
# - Never fails the session: it always exits 0.
# - Respects CARGO_TARGET_DIR; defaults to <repo>/target.
#
# Usage: scripts/dev/warm_up.sh
#   Internal: scripts/dev/warm_up.sh --background (used by the detached job)
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$REPO/target}"
LOG="$TARGET/warmup.log"
STAMP="$TARGET/.warmup-stamp"
LOCK="$TARGET/.warmup.lock"
MIN_FREE_GB="${HOOCODE_WARMUP_MIN_FREE_GB:-8}"
MAX_AGE_DAYS="${HOOCODE_WARMUP_MAX_AGE_DAYS:-3}"

note() {
  echo "warm_up: $*"
}

# Key for the build state: Cargo.lock plus the rustc version.
state_key() {
  {
    cat "$REPO/Cargo.lock" 2>/dev/null
    rustc -V 2>/dev/null
  } | cksum
}

is_warm() {
  local key="$1"
  [[ -f "$STAMP" ]] || return 1
  [[ "$(cat "$STAMP" 2>/dev/null)" == "$key" ]] || return 1
  # Stamp is fresh enough: find prints the file only if it is younger than the limit.
  [[ -n "$(find "$STAMP" -mmin -$((MAX_AGE_DAYS * 1440)) 2>/dev/null)" ]]
}

free_gb() {
  local kb
  kb="$(df -P -k "$TARGET" 2>/dev/null | awk 'NR==2 {print $4}')" || return 1
  [[ -n "$kb" ]] || return 1
  echo $((kb / 1048576))
}

# Runs detached. Holds the lock for the whole build.
run_build() {
  mkdir -p "$TARGET" || exit 0
  if command -v flock >/dev/null 2>&1; then
    exec 9>"$LOCK"
    if ! flock -n 9; then
      note "another warm-up holds $LOCK, exiting"
      exit 0
    fi
  fi

  local key rc
  key="$(state_key)"
  note "start $(date -u +%FT%TZ) target=$TARGET"

  cd "$REPO" || exit 0
  export CARGO_TARGET_DIR="$TARGET"
  export CARGO_TERM_COLOR=never

  if cargo nextest --version >/dev/null 2>&1; then
    nice -n 10 cargo nextest run --workspace --locked --no-run
  else
    note "cargo-nextest missing, warming with cargo test --no-run"
    nice -n 10 cargo test --workspace --locked --no-run
  fi
  rc=$?

  if [[ $rc -eq 0 ]]; then
    printf '%s\n' "$key" >"$STAMP"
    note "done $(date -u +%FT%TZ), stamp written"
  else
    rm -f "$STAMP"
    note "failed with exit $rc $(date -u +%FT%TZ)"
  fi
  exit 0
}

main() {
  if [[ "${1:-}" == "--background" ]]; then
    run_build
  fi

  if [[ "${HOOCODE_WARMUP:-1}" == "0" ]]; then
    note "disabled by HOOCODE_WARMUP=0"
    exit 0
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    note "cargo not found, skipping"
    exit 0
  fi

  mkdir -p "$TARGET" 2>/dev/null || { note "cannot create $TARGET, skipping"; exit 0; }

  local key free
  key="$(state_key)"
  if is_warm "$key"; then
    note "target already warm (stamp fresh), skipping"
    exit 0
  fi

  free="$(free_gb)" || free=0
  if (( free < MIN_FREE_GB )); then
    note "only ${free} GB free under $TARGET (< ${MIN_FREE_GB} GB), skipping"
    exit 0
  fi

  if command -v flock >/dev/null 2>&1 && ! ( exec 9>"$LOCK"; flock -n 9 ); then
    note "warm-up already running, skipping"
    exit 0
  fi

  setsid nohup bash "${BASH_SOURCE[0]}" --background >>"$LOG" 2>&1 </dev/null &
  note "started in background (pid $!), log: $LOG"
  exit 0
}

main "$@"
