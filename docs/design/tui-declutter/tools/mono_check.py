#!/usr/bin/env python3
"""Checks the NO_COLOR mockups: no colour attributes at all (only bold/dim/italic/reverse),
≤100 cols, bordered pane rows exactly 100 (full-width panes), boxes closed, no ◐."""
import glob, os, re, sys
HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "mockups", "mono")
MARK = re.compile(r"«([^»]*)»")
OK = {"bold", "dim", "italic", "reverse"}
bad = 0
for f in sorted(glob.glob(os.path.join(HERE, "*", "*.txt"))):
    name, fb = os.path.relpath(f, HERE), 0
    lines = open(f).read().splitlines()
    for i, l in enumerate(lines, 1):
        errs = []
        for spec in MARK.findall(l):
            for a in filter(None, spec.split(";")):
                if a not in OK:
                    errs.append(f"attribute {a!r}")
        p = MARK.sub("", l)
        if len(p) > 100: errs.append(f"width {len(p)}")
        if p[:1] in "┌│├└" and p[:1] and len(p) != 100: errs.append(f"full-width box row {len(p)}")
        if p[:1] in "┌│├└" and p[:1] and p[-1] not in "┐│┤┘": errs.append("box row not closed")
        if "◐" in p: errs.append("◐")
        for e in errs:
            print(f"FAIL {name}:{i} {e}"); fb += 1
    tops = sum(MARK.sub("", l).lstrip().startswith("┌") for l in lines)
    bots = sum(MARK.sub("", l).lstrip().startswith("└") for l in lines)
    if tops != bots: print(f"FAIL {name} {tops} tops vs {bots} bottoms"); fb += 1
    bad += fb
    print(("ok  " if not fb else "BAD ") + f"{name:24} {len(lines)} rows")
print("failures:", bad)
sys.exit(bad > 0)
