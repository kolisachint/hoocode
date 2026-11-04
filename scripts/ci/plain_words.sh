#!/usr/bin/env bash
# Guard for the plain-words rule of the TUI declutter (docs/design/tui-declutter.md):
# the TUI, its help, tips, slash command descriptions, settings text and notices use
# plain words, not the old space metaphor. Fails when a banned word appears inside a
# string literal in the source of the TUI crates.
#
# Checked: `crates/*/src` of the TUI crates (code-tui-*, tui-*, code-cli, code-subagents),
# lines that hold a string literal. Skipped: comment lines, test files (`tests/`,
# `*_tests.rs`, `tests.rs`) and test assertions (lines with `assert`).
# Banned: the uppercase words SHIP FUEL CR LANDED HIT M1 CLEARANCE TRANSMISSION HANGAR
# DOCKING MISSION SYSTEMS RESOURCES (case-sensitive, whole word) and the phrases
# "flight path", "end shift", "satellite", "jettison", "comms", "hull", "mission control".
# Docs, archive and design pages are not checked (they record the old names).
#
# Status: add it to the CI `guards` job next to no_cortex.sh (the workflow file needs a
# token with workflow scope). Run it by hand too: scripts/ci/plain_words.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

CRATES=(
  'crates/hoocode-code-tui-*/src/*'
  'crates/hoocode-tui-*/src/*'
  'crates/hoocode-code-cli/src/*'
  'crates/hoocode-code-subagents/src/*'
)

UPPER='\b(SHIP|FUEL|CR|LANDED|HIT|M1|CLEARANCE|TRANSMISSION|HANGAR|DOCKING|MISSION|SYSTEMS|RESOURCES)\b'
PHRASES='[Ff]light path|[Ee]nd shift|[Ee]nd-of-shift|[Ss]atellites?\b|[Jj]ettison|\b[Cc]omms\b|\b[Hh]ull\b|[Mm]ission [Cc]ontrol'

# git grep exits 1 when nothing matches, which is the pass case.
set +e
raw=$(git grep -n -I -E -e "$UPPER" -e "$PHRASES" -- "${CRATES[@]}" \
  ':(exclude)*/tests/*' ':(exclude)*_tests.rs' ':(exclude)*/tests.rs' \
  ':(exclude)*/data/*' ':(exclude)*.json')
status=$?
set -e

hits=""
if [ "$status" -eq 0 ]; then
  while IFS= read -r line; do
    body=${line#*:*:}
    case "$body" in
      *'"'*) ;;                                  # no string literal on the line
      *) continue ;;
    esac
    trimmed=${body#"${body%%[![:space:]]*}"}
    case "$trimmed" in
      //*|'#['*|*assert*) continue ;;            # comments, attributes, test assertions
    esac
    # Only count a banned word that sits inside the quotes.
    if printf '%s\n' "$body" | grep -q -E "\"[^\"]*(($UPPER)|($PHRASES))[^\"]*\""; then
      hits="${hits}${line}"$'\n'
    fi
  done <<< "$raw"
  hits="${hits%$'\n'}"
fi

if [ -n "$hits" ]; then
  printf '%s\n' "$hits" | sed -n '1,200p'
  echo "plain_words: FAIL: user-visible text uses the old space-metaphor words (see scripts/ci/plain_words.sh)" >&2
  exit 1
fi
echo "plain_words: OK"
