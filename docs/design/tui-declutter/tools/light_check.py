#!/usr/bin/env python3
"""Checks the light mockups: widths, fill rows = 100, light palette only, WCAG ≥4.5, no ◐."""
import glob, os, re, sys
from light_palette import BASE, TEXT, DIM, TEAL, GREEN, RED, YEL, PANE, DETAIL, SEL, INPUT, CHIP, CHIPFG, WARN, WARNFG, cr

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "mockups", "light")
FGS = {TEXT, DIM, TEAL, GREEN, RED, YEL, CHIPFG, WARNFG}
BGS = {PANE, DETAIL, SEL, INPUT, CHIP, WARN, BASE}
MARK = re.compile(r"«([^»]*)»")
bad = 0
for f in sorted(glob.glob(os.path.join(HERE, "*", "*.txt"))):
    name = os.path.relpath(f, HERE)
    fb = 0
    for i, l in enumerate(open(f).read().splitlines(), 1):
        parts = MARK.split(l)
        runs = [(parts[0], {})] + [(parts[k + 1], dict(a.split("=") for a in parts[k].split(";") if "=" in a))
                                   for k in range(1, len(parts), 2)]
        n = sum(len(t) for t, _ in runs)
        errs = []
        if n > 100: errs.append(f"width {n}>100")
        if any("bg" in a for _, a in runs) and n != 100: errs.append(f"fill row width {n}")
        for t, a in runs:
            if a.get("fg") and a["fg"] not in FGS: errs.append(f"fg {a['fg']} not light")
            if a.get("bg") and a["bg"] not in BGS: errs.append(f"bg {a['bg']} not light")
            if t.strip():
                c = cr(a.get("fg", TEXT), a.get("bg", BASE))
                if c < 4.5: errs.append(f"contrast {c:.2f} {t.strip()[:20]!r}")
        if "◐" in l: errs.append("◐ glyph")
        for e in errs:
            print(f"FAIL {name}:{i} {e}"); fb += 1
    bad += fb
    print(("ok  " if not fb else "BAD ") + name)
print("failures:", bad)
sys.exit(bad > 0)
