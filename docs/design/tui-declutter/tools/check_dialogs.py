#!/usr/bin/env python3
"""Checks the v3 mockups: widths, palette, prompt row rule (v2.2), text contrast (WCAG)."""
import glob
import os
import re
import sys

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "mockups", "dark", "dialogs")
BASE = "#14141c"
FG = {"#e5e5e7", "#989898", "#5cc8bb", "#b5bd68", "#d58181", "#f0c674", BASE}
BG = {"#1e1e2a",  # pane
      "#292b40",  # detail column (v2.9)
      "#233239",  # pane selection
      "#2a2a36",  # surface: prompt row, pane input row, user-message band in theme preview
      "#5cc8bb", "#f0c674",  # title chips
      BASE}  # theme live-preview window
SURFACE = "#2a2a36"
REPLACING = {"permission", "question", "quit", "models", "settings", "sessions", "theme",
             "tree", "fork", "login", "scoped_models", "keys", "first_run_login"}
MARK = re.compile(r"«([^»]*)»")


def lum(h):
    c = [int(h[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    c = [x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4 for x in c]
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]


def contrast(a, b):
    a, b = lum(a), lum(b)
    return (max(a, b) + 0.05) / (min(a, b) + 0.05)


def spans(line):
    """(text, fg, bg) for each styled run."""
    parts = MARK.split(line)
    out = []
    for i in range(1, len(parts), 2):
        attrs = dict(a.split("=") for a in parts[i].split(";") if "=" in a)
        out.append((parts[i + 1], attrs.get("fg"), attrs.get("bg")))
    return out


bad = 0
for f in sorted(glob.glob(os.path.join(HERE, "*.txt"))):
    lines = open(f).read().splitlines()
    fb = 0
    name = os.path.basename(f)[:-4]
    for i, l in enumerate(lines, 1):
        runs = spans(l)
        n = sum(len(t) for t, _, _ in runs)
        fgs = {fg for _, fg, _ in runs if fg}
        bgs = {bg for _, _, bg in runs if bg}
        errs = []
        if n > 100:
            errs.append(f"width {n}>100")
        if bgs and n != 100:
            errs.append(f"pane/surface row width {n} (must be 100)")
        if fgs - FG:
            errs.append(f"fg not in palette {fgs - FG}")
        if bgs - BG:
            errs.append(f"bg not in palette {bgs - BG}")
        for t, fg, bg in runs:
            if t.strip():
                c = contrast(fg or "#e5e5e7", bg or BASE)
                if c < 4.5:
                    errs.append(f"contrast {c:.2f} {fg} on {bg or BASE}: {t.strip()[:20]!r}")
        for e in errs:
            print(f"FAIL {name}:{i} {e}")
            fb += 1
    has_prompt = {bg for _, _, bg in spans(lines[-2]) if bg} == {SURFACE}
    if (name in REPLACING) == has_prompt:
        print(f"FAIL {name} prompt row {'present' if has_prompt else 'missing'}")
        fb += 1
    if lines[0] != "«»" and not name.startswith("first_run"):  # start screen opens with its header
        print(f"FAIL {name} first row not blank")
        fb += 1
    bad += fb
    print(("ok  " if not fb else "BAD ") + f"{name + '.txt':28} {len(lines)} rows")
print("failures:", bad)
sys.exit(bad > 0)
