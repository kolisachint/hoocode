#!/usr/bin/env bash
#
# Run the test suite.
#
#   ./test.sh                 every package
#   ./test.sh coding-agent    one package
#   ./test.sh ai tui          several
#   ./test.sh -- test/tips.test.ts
#                             pass the rest through to vitest (single package only)
#
# Why this exists rather than `bun run test` at the root: the root script is
# `npm run test --workspaces --if-present`, and under bun that re-invokes itself
# and appends its own flags until it runs out of memory. AGENTS.md says to run
# tests from the package root, and this is that, in the right order.
#
# The order matters. packages/agent and packages/coding-agent import
# @kolisachint/hoocode-ai by package name, which vite resolves through its
# `exports` to dist/ -- so a clean checkout fails with "Failed to resolve entry
# for package" until ai, agent and tui have been built once. CI builds them for
# the same reason.
#
# LLM-dependent tests skip themselves when the matching API key is absent, so a
# run with no credentials is expected to be green, not to be skipped wholesale.

set -euo pipefail

cd "$(dirname "$0")"

ALL_PACKAGES=(ai agent tui coding-agent)
PACKAGES=()
PASSTHROUGH=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --)
            shift
            PASSTHROUGH=("$@")
            break
            ;;
        -h|--help)
            sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            PACKAGES+=("$1")
            shift
            ;;
    esac
done

if [[ ${#PACKAGES[@]} -eq 0 ]]; then
    PACKAGES=("${ALL_PACKAGES[@]}")
fi

for want in "${PACKAGES[@]}"; do
    if [[ ! -d "packages/$want" ]]; then
        echo "Unknown package: $want" >&2
        echo "Known: ${ALL_PACKAGES[*]}" >&2
        exit 1
    fi
done

if [[ ${#PASSTHROUGH[@]} -gt 0 && ${#PACKAGES[@]} -ne 1 ]]; then
    echo "Arguments after -- need exactly one package: ./test.sh coding-agent -- test/foo.test.ts" >&2
    exit 1
fi

echo "==> Building the packages the tests resolve through dist/..."
for pkg in ai agent tui; do
    (cd "packages/$pkg" && bun run build >/dev/null)
done

FAILED=()
for pkg in "${PACKAGES[@]}"; do
    echo ""
    echo "==> packages/$pkg"
    if [[ ${#PASSTHROUGH[@]} -gt 0 ]]; then
        (cd "packages/$pkg" && bun run test "${PASSTHROUGH[@]}") || FAILED+=("$pkg")
    else
        (cd "packages/$pkg" && bun run test) || FAILED+=("$pkg")
    fi
done

echo ""
if [[ ${#FAILED[@]} -gt 0 ]]; then
    echo "FAILED: ${FAILED[*]}"
    exit 1
fi
echo "All tests passed (${PACKAGES[*]})."
