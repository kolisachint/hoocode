#!/usr/bin/env bash
# Prune Rust build output under target/. Cargo output only.
# Dry run by default: it prints what it would free and deletes nothing.
#
# Usage:
#   scripts/dev/prune_target.sh [options]
#
# Options:
#   --apply           delete (default is a dry run)
#   --days N          deps units, .fingerprint and build entries whose newest
#                     file is older than N days are removed (default 3)
#   --inc-cap-gb N    if incremental/ is over N GB, keep only the newest session
#                     per unit; if still over, delete all of incremental/
#                     (default 5)
#   --target DIR      target dir (default: $CARGO_TARGET_DIR or <repo>/target)
#   --if-over GB      do nothing unless target/ is over GB
#   --quiet           print only the summary table and errors
#   -h, --help        show this help
#
# Always removed: entries named cortex* (old crate names).
# Never touched: anything outside <profile>/{deps,.fingerprint,build,incremental},
#   and target/{tui-goldens,subagent-evals,perf,codex-schema}.
# GB means 1024 x 1024 x 1024 bytes. Sizes are disk blocks, as du reports them.
set -euo pipefail

usage() {
  cat <<'EOF'
Prune Rust build output under target/. Dry run unless --apply is given.

Usage: scripts/dev/prune_target.sh [options]

  --apply           delete (default is a dry run)
  --days N          remove deps units, .fingerprint and build entries whose
                    newest file is older than N days (default 3)
  --inc-cap-gb N    incremental/ over N GB keeps only the newest session per
                    unit; still over, delete all of incremental/ (default 5)
  --target DIR      target dir (default: $CARGO_TARGET_DIR or <repo>/target)
  --if-over GB      do nothing unless target/ is over GB
  --quiet           print only the summary table and errors
  -h, --help        show this help

Always removed: entries named cortex* (old crate names).
Never touched: other directories, and target/{tui-goldens,subagent-evals,
perf,codex-schema}.
GB means 1024^3 bytes.
EOF
}

APPLY=0
DAYS=3
INC_CAP_GB=5
TARGET=""
IF_OVER=""
QUIET=0

die() {
  echo "prune_target: $*" >&2
  exit 2
}

need_value() {
  [ $# -ge 2 ] && [ -n "$2" ] || die "$1 needs a value (see --help)"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1 ;;
    --days)
      need_value "$@"
      DAYS="$2"
      shift
      ;;
    --inc-cap-gb)
      need_value "$@"
      INC_CAP_GB="$2"
      shift
      ;;
    --target)
      need_value "$@"
      TARGET="$2"
      shift
      ;;
    --if-over)
      need_value "$@"
      IF_OVER="$2"
      shift
      ;;
    --quiet) QUIET=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "unknown option: $1 (see --help)" ;;
  esac
  shift
done

[[ "$DAYS" =~ ^[0-9]+$ ]] || die "--days must be a whole number"
[[ "$INC_CAP_GB" =~ ^[0-9]+([.][0-9]+)?$ ]] || die "--inc-cap-gb must be a number"
if [ -n "$IF_OVER" ]; then
  [[ "$IF_OVER" =~ ^[0-9]+([.][0-9]+)?$ ]] || die "--if-over must be a number"
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [ -z "$TARGET" ]; then
  TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
fi

if [ ! -d "$TARGET" ]; then
  [ "$QUIET" -eq 1 ] || echo "No target dir at $TARGET. Nothing to do."
  exit 0
fi
TARGET="$(cd "$TARGET" && pwd)"

exec python3 - "$TARGET" "$DAYS" "$INC_CAP_GB" "$APPLY" "$QUIET" "$IF_OVER" <<'PY'
import os
import re
import shutil
import stat
import sys
import time

target = sys.argv[1]
days = int(sys.argv[2])
cap_text = sys.argv[3]
inc_cap_gb = float(cap_text)
apply_mode = sys.argv[4] == "1"
quiet = sys.argv[5] == "1"
if_over = sys.argv[6]

GIB = 1024 ** 3
EXCLUDED = {"tui-goldens", "subagent-evals", "perf", "codex-schema"}
AREAS = ("deps", ".fingerprint", "build", "incremental")
# deps file: [lib]<crate>-<16 hex>[.suffix]; the crate name has no hyphens.
UNIT_RE = re.compile(r"^(?:lib)?([A-Za-z0-9_]+)-([0-9a-f]{16})(?=[.\-]|$)")
# .fingerprint and build entry: <package>-<16 hex>; package names may have hyphens.
DIR_RE = re.compile(r"^(.+)-([0-9a-f]{16})$")

errors = []
plan = []


def gb(nbytes):
    return f"{nbytes / GIB:.1f}"


def walk(path, st):
    """Return (disk bytes, newest mtime) for a path. Never follows symlinks."""
    size = st.st_blocks * 512
    newest = st.st_mtime
    if stat.S_ISDIR(st.st_mode):
        try:
            it = os.scandir(path)
        except OSError as ex:
            errors.append(f"cannot read {path}: {ex.strerror}")
            return size, newest
        with it:
            for e in it:
                try:
                    cst = e.stat(follow_symlinks=False)
                except OSError as ex:
                    errors.append(f"cannot stat {e.path}: {ex.strerror}")
                    continue
                s, m = walk(e.path, cst)
                size += s
                if m > newest:
                    newest = m
    return size, newest


def add(profile, area, reason, paths, size, count):
    plan.append({
        "profile": profile, "area": area, "reason": reason,
        "paths": paths, "size": size, "count": count, "ok": False,
    })


def scan_deps(path, profile, cutoff):
    total = 0
    units = {}
    for e in os.scandir(path):
        try:
            st = e.stat(follow_symlinks=False)
        except OSError as ex:
            errors.append(f"cannot stat {e.path}: {ex.strerror}")
            continue
        size, newest = walk(e.path, st)
        total += size
        m = UNIT_RE.match(e.name)
        if not m:
            continue  # not a cargo unit file: leave it alone
        u = units.setdefault(m.group(2), {"name": m.group(1), "paths": [], "size": 0, "newest": 0.0})
        u["paths"].append(e.path)
        u["size"] += size
        if newest > u["newest"]:
            u["newest"] = newest
    for u in units.values():
        if u["name"].startswith("cortex"):
            reason = "cortex"
        elif u["newest"] < cutoff:
            reason = "old"
        else:
            continue
        add(profile, "deps", reason, u["paths"], u["size"], 1)
    return total


def scan_named_dirs(path, profile, area, cutoff):
    total = 0
    for e in os.scandir(path):
        try:
            st = e.stat(follow_symlinks=False)
        except OSError as ex:
            errors.append(f"cannot stat {e.path}: {ex.strerror}")
            continue
        size, newest = walk(e.path, st)
        total += size
        if not stat.S_ISDIR(st.st_mode):
            continue
        m = DIR_RE.match(e.name)
        if not m:
            continue
        if m.group(1).startswith("cortex"):
            reason = "cortex"
        elif newest < cutoff:
            reason = "old"
        else:
            continue
        add(profile, area, reason, [e.path], size, 1)
    return total


def scan_incremental(path, profile, cap):
    # Each incremental/<crate>-<hash> unit keeps its newest session.
    units = []
    total = 0
    for u in os.scandir(path):
        try:
            st = u.stat(follow_symlinks=False)
        except OSError as ex:
            errors.append(f"cannot stat {u.path}: {ex.strerror}")
            continue
        size, _ = walk(u.path, st)
        total += size
        if not stat.S_ISDIR(st.st_mode):
            continue
        sessions = []
        for s in os.scandir(u.path):
            try:
                sst = s.stat(follow_symlinks=False)
            except OSError as ex:
                errors.append(f"cannot stat {s.path}: {ex.strerror}")
                continue
            if stat.S_ISDIR(sst.st_mode):
                ssize, snewest = walk(s.path, sst)
                sessions.append((snewest, ssize, s.path))
        units.append(sessions)
    if total <= cap:
        return total
    per_session = []
    freed = 0
    for sessions in units:
        sessions.sort(reverse=True)  # newest first
        for _, ssize, spath in sessions[1:]:
            per_session.append((ssize, spath))
            freed += ssize
    if total - freed > cap:
        count = sum(len(s) for s in units)
        add(profile, "incremental", "whole", [path], total, count)
    else:
        for ssize, spath in per_session:
            add(profile, "incremental", "old-session", [spath], ssize, 1)
    return total


def scan_profile(pdir, cutoff, cap):
    profile = os.path.basename(pdir)
    before = 0
    for e in os.scandir(pdir):
        try:
            st = e.stat(follow_symlinks=False)
        except OSError as ex:
            errors.append(f"cannot stat {e.path}: {ex.strerror}")
            continue
        is_area = stat.S_ISDIR(st.st_mode) and e.name in AREAS
        if is_area and e.name == "deps":
            before += scan_deps(e.path, profile, cutoff)
        elif is_area and e.name == "incremental":
            before += scan_incremental(e.path, profile, cap)
        elif is_area:
            before += scan_named_dirs(e.path, profile, e.name, cutoff)
        else:
            size, _ = walk(e.path, st)
            before += size
    return before


def safe_path(path):
    rel = os.path.relpath(path, target)
    parts = rel.split(os.sep)
    if ".." in parts or len(parts) < 2:
        return False
    if parts[0] in EXCLUDED or parts[1] not in AREAS:
        return False
    if len(parts) >= 3:
        return True
    return len(parts) == 2 and parts[1] == "incremental"


def main():
    cutoff = time.time() - days * 86400
    cap = inc_cap_gb * GIB
    if not quiet:
        print("Scanning target/ (this can take a little while)...", file=sys.stderr)

    other = 0
    profiles = []
    for e in sorted(os.scandir(target), key=lambda x: x.name):
        try:
            st = e.stat(follow_symlinks=False)
        except OSError as ex:
            errors.append(f"cannot stat {e.path}: {ex.strerror}")
            continue
        is_profile = (
            stat.S_ISDIR(st.st_mode)
            and e.name not in EXCLUDED
            and os.path.isdir(os.path.join(e.path, "deps"))
        )
        if is_profile:
            profiles.append((e.name, scan_profile(e.path, cutoff, cap)))
        else:
            size, _ = walk(e.path, st)
            other += size

    total = other + sum(b for _, b in profiles)
    if if_over != "" and total <= float(if_over) * GIB:
        if not quiet:
            print(f"target/ is {gb(total)} GB, at or under {if_over} GB. Nothing to do.")
        return 0

    if not quiet:
        print(f"Target: {target}")
        if apply_mode:
            print("Mode: APPLY. Deleting.")
        else:
            print("Mode: DRY RUN. Nothing is deleted. Add --apply to delete.")
        print(f"Rules: older than {days} days; cortex*; incremental cap {cap_text} GB")
        print()
        groups = {}
        for a in plan:
            key = (a["profile"], a["area"], a["reason"])
            g = groups.setdefault(key, [0, 0])
            g[0] += a["count"]
            g[1] += a["size"]
        labels = {
            "old": f"older than {days} days",
            "cortex": "cortex* (old crate name)",
            "old-session": "old session (over cap)",
            "whole": "whole dir (still over cap)",
        }
        if not groups:
            print("Nothing matches the rules.")
        for (profile, area, reason), (n, s) in sorted(groups.items()):
            print(f"{profile}/{area}: {n} x {labels[reason]}, {gb(s)} GB")
        print()

    for a in plan:
        if not apply_mode:
            a["ok"] = True
            continue
        ok = True
        for p in a["paths"]:
            if not safe_path(p):
                errors.append(f"refused unsafe path: {p}")
                ok = False
                break
            try:
                if os.path.isdir(p) and not os.path.islink(p):
                    shutil.rmtree(p)
                else:
                    os.remove(p)
            except OSError as ex:
                errors.append(f"could not remove {p}: {ex.strerror}")
                ok = False
        a["ok"] = ok

    header = f"{'Profile':<12}{'Before GB':>11}{'Freed GB':>11}{'After GB':>11}{'Removed':>10}"
    rule = "-" * len(header)
    rows = []
    t_before = t_freed = t_removed = 0
    for name, before in profiles:
        freed = sum(a["size"] for a in plan if a["ok"] and a["profile"] == name)
        removed = sum(a["count"] for a in plan if a["ok"] and a["profile"] == name)
        rows.append((name, before, freed, removed))
        t_before += before
        t_freed += freed
        t_removed += removed
    rows.append(("other", other, 0, 0))
    t_before += other

    print("Summary" + (" (would free)" if not apply_mode else ""))
    print(header)
    print(rule)
    for name, before, freed, removed in rows:
        after = before - freed
        print(f"{name:<12}{gb(before):>11}{gb(freed):>11}{gb(after):>11}{removed:>10}")
    print(rule)
    print(f"{'TOTAL':<12}{gb(t_before):>11}{gb(t_freed):>11}{gb(t_before - t_freed):>11}{t_removed:>10}")
    print()
    print("Removed = deps units, .fingerprint/build dirs, incremental sessions.")
    print("'other' = everything not pruned. It is never touched.")
    if apply_mode:
        failed = sum(1 for a in plan if not a["ok"])
        print(f"Removed {t_removed} entries. Failed: {failed}.")

    for msg in errors:
        print(f"error: {msg}", file=sys.stderr)
    return 1 if errors else 0


sys.exit(main())
PY
