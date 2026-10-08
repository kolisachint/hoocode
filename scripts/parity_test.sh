#!/bin/bash
# Parity smoke test (migration task 13.3).
#
# The real parity gates are:
#   Level 1  cargo test -p hoocode-code-main --test it replay
#            (hoocode vs fixtures recorded from the pinned hoocode; runs in CI via
#            `cargo test --workspace`)
#   Level 2  python3 migration/tui-parity/harness.py run all
#            (rendered TUI, real hoocode vs real hoocode in tmux; manual/nightly
#            workflow staged in migration/ci/tui-parity.yml)
#
# This script is only a quick smoke: the binary starts, answers --version/--help,
# the Level-1 replay passes, and (when the pinned hoocode is built, see
# migration/tui-parity/setup_hoocode.sh) one Level-2 scenario passes.
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
if [ -f target/hoocode-pin/packages/coding-agent/dist/cli.js ] && command -v tmux >/dev/null; then
    check "Level-2 print-basic (hoocode-ts vs hoocode in tmux)" python3 migration/tui-parity/harness.py run print-basic
else
    echo "SKIP  Level-2 print-basic (run migration/tui-parity/setup_hoocode.sh; needs tmux)"
fi

exit $failed
