#!/usr/bin/env bash
# Guard for the rename to hoocode (naming-and-paths.md §1): fails if the old app
# name appears in any tracked text file outside the allowlist below. Matching is
# case-insensitive and binary files are skipped.
#
# Allowlist (paths are git pathspecs):
#   - the naming card and the dated decision pages, which must name the old names
#   - this script
#   - the merge code that reads the old ~/.cortexcode and <repo>/.cortexcode folders
#     (add its path here when it lands in step 1.2)
#
# Status: not wired into CI yet. CI wiring happens after step 0c (the rename), since
# the guard fails on the tree until then. Run it by hand: scripts/ci/no_cortex.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

ALLOW=(
  ':(exclude)docs/design/naming-and-paths.md'
  ':(exclude)docs/design/decisions-2026-10-07.md'
  ':(exclude)docs/design/decisions-2026-10-08.md'
  ':(exclude)scripts/ci/no_cortex.sh'
  ':(exclude)scripts/rename/rename_to_hoocode.py'  # the one-shot step 0c tool names the old names by design
  ':(exclude)Cargo.lock'                            # regenerated from the crate names
  ':(exclude)migration/ci/rename-to-hoocode.patch'  # the staged CI rename: it removes the old names
  ':(exclude)CLAUDE.md'                             # names the guard's target in its rule ("don't add new cortex names")
)

# Tokens that are not this rename's to change (naming-and-paths.md §2-4, milestone 1.2) and
# the other project pycortex. They are stripped before the check, so a line that also has a
# real old name still fails.
#   .cortexcode, CORTEX_* / CORTEXCODE_* env names, cortex-debug (milestone 1.2)
#   pycortex (another project, the Python migration this one follows)
EXEMPT='s/pycortex//Ig; s/\.cortexcode([^A-Za-z0-9_]|$)/\1/g; s/CORTEX(CODE)?_[A-Z0-9_]*//g; s/cortex-debug//g'

# git grep exits 1 when nothing matches, which is the pass case.
set +e
raw=$(git grep -n -I -i -e 'cortex' -- . "${ALLOW[@]}")
status=$?
set -e
hits=""
if [ "$status" -eq 0 ]; then
  while IFS= read -r line; do
    if printf '%s\n' "$line" | sed -E "$EXEMPT" | grep -q -i 'cortex'; then
      hits="${hits}${line}"$'\n'
    fi
  done <<< "$raw"
  hits="${hits%$'\n'}"
  # Exempt-only hits are not failures.
  [ -n "$hits" ] || status=1
fi

if [ "$status" -eq 0 ]; then
  printf '%s\n' "$hits" | sed -n '1,200p'
  files=$(printf '%s\n' "$hits" | cut -d: -f1 | sort -u | wc -l | tr -d ' ')
  lines=$(printf '%s\n' "$hits" | wc -l | tr -d ' ')
  echo "no_cortex: FAIL: $lines line(s) in $files file(s) use the old name" >&2
  exit 1
elif [ "$status" -eq 1 ]; then
  echo "no_cortex: OK"
  exit 0
else
  echo "no_cortex: git grep failed (exit $status)" >&2
  exit "$status"
fi
