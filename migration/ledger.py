#!/usr/bin/env python3
"""Migration ledger, read-only (frozen 2026-10-09, TUI plan T0.6).

    ledger.py status              # progress per phase + current task
    ledger.py next                # the task to work on now (in_progress, else first ready todo)
    ledger.py show <id>           # full task card: hoocode sources at the pin, TS tests, gates
    ledger.py check               # validate ledger structure (CI)

The ledger no longer changes through this script. ``start``, ``note``, ``block``, ``move``
and ``verify`` exit non-zero. The hoocode-ts parity gate (Level 2) is retired: the done bar
is now Level 1 plus ``python3 scripts/tui/goldens.py check all``. New work is tracked in the
phase tables of docs/design/tui-activity.md, not in this ledger.

Migration is paused (CLAUDE.md). ``migration/ledger.json`` is kept as history.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEDGER = ROOT / "migration" / "ledger.json"
# Screen scenarios moved from migration/tui-parity/ to scripts/tui/ in T0.6.
SCENARIOS = ROOT / "scripts" / "tui" / "scenarios"
# "moved" tasks are closed in the ledger: their work now lives in a design card
# (moved_to). Dependents of a moved task are ready, like dependents of a done one.
DONEISH = {"done", "l1_done", "moved"}
STATUSES = {"todo", "in_progress", "l1_done", "done", "blocked", "deferred", "moved"}
MUTATING = {"start", "note", "block", "move", "verify"}
FROZEN = (
    "ledger.py is read-only since 2026-10-09 (TUI plan T0.6). '{cmd}' would change the ledger and is refused.\n"
    "New work is tracked in the phase tables of docs/design/tui-activity.md.\n"
    "The done bar is L1 plus `python3 scripts/tui/goldens.py check all`."
)
FROZEN_FOOTER = "(ledger.py is read-only since 2026-10-09; see docs/design/tui-activity.md)"


def load() -> dict:
    return json.loads(LEDGER.read_text())


def task(led: dict, tid: str) -> dict:
    for t in led["tasks"]:
        if t["id"] == tid:
            return t
    sys.exit(f"unknown task {tid}")


def ready(led: dict, t: dict) -> bool:
    by_id = {x["id"]: x for x in led["tasks"]}
    return all(by_id[d]["status"] in DONEISH for d in t["depends"])


# ---------------------------------------------------------------------------


def cmd_status(led: dict) -> None:
    phases: dict[int, dict[str, int]] = {}
    for t in led["tasks"]:
        phases.setdefault(t["phase"], {}).setdefault(t["status"], 0)
        phases[t["phase"]][t["status"]] += 1
    print(f"hoocode pin {led['pin']['hoocode_version']} ({led['pin']['hoocode_commit'][:8]})")
    for ph in sorted(phases):
        counts = phases[ph]
        total = sum(counts.values())
        done = counts.get("done", 0)
        parts = " ".join(f"{k}={v}" for k, v in sorted(counts.items()))
        print(f"  phase {ph:>2}: {done}/{total} done   {parts}")
    cur = [t for t in led["tasks"] if t["status"] == "in_progress"]
    for t in cur:
        print(f"in progress: {t['id']} {t['title']}")
    l1 = [t["id"] for t in led["tasks"] if t["status"] == "l1_done"]
    if l1:
        print(f"l1_done (L2 retired, not closed): {', '.join(l1)}")


def cmd_next(led: dict) -> None:
    cur = [t for t in led["tasks"] if t["status"] == "in_progress"]
    if cur:
        print(f"CONTINUE {cur[0]['id']}")
        show(cur[0])
    else:
        for t in led["tasks"]:
            if t["status"] == "todo" and ready(led, t):
                print(f"NEXT {t['id']} (not started: the ledger is read-only)")
                show(t)
                break
        else:
            print("nothing ready: all remaining tasks are blocked/deferred or waiting on dependencies")
    print(FROZEN_FOOTER)


def show(t: dict) -> None:
    pin_dir = ROOT / "target" / "hoocode-pin"
    print(f"\n[{t['id']}] {t['title']}\n  phase {t['phase']} · status {t['status']} · depends {t['depends'] or '-'}")
    if t["status"] == "moved":
        print(f"  moved to: {t.get('moved_to', '?')}")
    print(f"  crates: {', '.join(t['crates']) or '-'}")
    print("  hoocode sources (at pin, in target/hoocode-pin):")
    for s in t["sources"]:
        mark = "" if (pin_dir / s).exists() or not pin_dir.exists() else "  (missing at pin!)"
        print(f"    {s}{mark}")
    if t["ts_tests"]:
        print("  TS tests to port: " + ", ".join(t["ts_tests"]))
    if t["l1_cmds"]:
        print("  extra L1 commands: " + "; ".join(t["l1_cmds"]))
    l2 = t["l2"]
    if isinstance(l2, list):
        for s in l2:
            exists = (SCENARIOS / f"{s}.json").exists()
            print(f"  L2 scenario: {s}{'' if exists else '  (not written yet)'}")
    else:
        print(f"  L2: {l2}")
    if t["notes"]:
        print(f"  notes: {t['notes']}")
    for entry in t.get("log", [])[-5:]:
        print(f"  log {entry['at']} {entry['commit']} [{entry['kind']}] {entry['text']}")


def cmd_check(led: dict) -> int:
    ids = [t["id"] for t in led["tasks"]]
    errs = []
    if len(ids) != len(set(ids)):
        errs.append("duplicate task ids")
    for t in led["tasks"]:
        if t["status"] not in STATUSES:
            errs.append(f"{t['id']}: bad status {t['status']}")
        for d in t["depends"]:
            if d not in ids:
                errs.append(f"{t['id']}: unknown dependency {d}")
        if t["status"] == "done" and isinstance(t["l2"], list):
            for s in t["l2"]:
                if not (SCENARIOS / f"{s}.json").exists():
                    errs.append(f"{t['id']}: done but L2 scenario {s} does not exist")
        if t["status"] == "moved":
            target = t.get("moved_to", "")
            if not target or not (ROOT / target.split("#")[0]).exists():
                errs.append(f"{t['id']}: moved needs moved_to pointing at an existing doc")
        if isinstance(t["l2"], str) and not t["l2"].startswith("n/a"):
            errs.append(f"{t['id']}: l2 must be a scenario list or 'n/a: <reason>'")
    if sum(t["status"] == "in_progress" for t in led["tasks"]) > 1:
        errs.append("more than one task in_progress")
    for e in errs:
        print(f"error: {e}", file=sys.stderr)
    print(f"ledger ok: {len(ids)} tasks" if not errs else f"{len(errs)} error(s)")
    return 1 if errs else 0


def main() -> int:
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    cmd = args[0]
    if cmd in MUTATING:
        print(FROZEN.format(cmd=cmd), file=sys.stderr)
        return 2
    led = load()
    if cmd == "status":
        cmd_status(led)
    elif cmd == "next":
        cmd_next(led)
    elif cmd == "show":
        if len(args) < 2:
            sys.exit("usage: ledger.py show <id>")
        show(task(led, args[1]))
    elif cmd == "check":
        return cmd_check(led)
    else:
        sys.exit(f"unknown command {cmd}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
