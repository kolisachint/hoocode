#!/bin/bash
# Parity smoke test (migration task 13.3).
#
# The real parity gates are:
#   Level 1  cargo test -p hoocode-code-main --test it replay
#            (hoocode vs fixtures recorded from the pinned hoocode; runs in CI via
#            `cargo test --workspace`)
#   Level 2  python3 scripts/tui/goldens.py check all
#            (rendered TUI: real hoocode in tmux against the mock LLM, diffed with
#            tests/golden/tui/; manual/nightly workflow staged in archive/migration/ci/tui-parity.yml)
#
# This script is only a quick smoke: the binary starts, answers --version/--help,
# the Level-1 replay passes, and (when tmux is installed) the startup golden passes.
set -euo pipefail
cd "$(dirname "$0")/.."

failed=0
check() {
    local name="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        echo "PASS  $name"
    else
        echo "FAIL  $name"
        failed=1
    fi
}

cargo build -q -p hoocode-code-main --bin hoocode
bin=target/debug/hoocode

check "--version prints the version" sh -c "$bin --version </dev/null 2>&1 | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+'"
check "--help prints usage" sh -c "$bin --help </dev/null 2>&1 | grep -q 'Usage:'"
check "Level-1 replay (hoocode-ts vs hoocode fixtures)" cargo test -q -p hoocode-code-main --test it replay
if command -v tmux >/dev/null; then
    check "Level-2 startup golden (tests/golden/tui)" python3 scripts/tui/goldens.py check startup
else
    echo "SKIP  Level-2 startup golden (needs tmux)"
fi

exit $failed
