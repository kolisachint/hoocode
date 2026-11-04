#!/usr/bin/env python3
"""Map the dark declutter mockups to the light palette by role (fg and bg separately)."""
import glob, os, re
from light_palette import BASE, TEXT, DIM, TEAL, GREEN, RED, YEL, PANE, DETAIL, SEL, INPUT, CHIP, CHIPFG, WARN, WARNFG

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "mockups", "light")
SRC = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "mockups", "dark")
D_BASE, D_TEAL, D_YEL = "#14141c", "#5cc8bb", "#f0c674"
FG = {"#e5e5e7": TEXT, "#989898": DIM, D_TEAL: TEAL, "#b5bd68": GREEN, "#d58181": RED, D_YEL: YEL}
BG = {"#1e1e2a": PANE, "#292b40": DETAIL, "#233239": SEL, "#2a2a36": INPUT,
      D_TEAL: CHIP, D_YEL: WARN, D_BASE: BASE}
MARK = re.compile(r"«([^»]*)»")


def style(spec):
    toks = [t for t in spec.split(";") if t]
    kv = dict(t.split("=") for t in toks if "=" in t)
    bg = kv.get("bg")
    out = []
    for t in toks:
        if t.startswith("fg="):
            fg = t[3:]
            if fg == D_BASE:  # dark text on a chip
                fg = {D_TEAL: CHIPFG, D_YEL: WARNFG}[bg]
            else:
                fg = FG[fg]
            out.append("fg=" + fg)
        elif t.startswith("bg="):
            out.append("bg=" + BG[t[3:]])
        else:
            out.append(t)
    return "«" + ";".join(out) + "»"


def convert(text):
    return MARK.sub(lambda m: style(m.group(1)), text).replace("◐", "◉")


def theme_fix(text):
    """Light screen: `light` is the theme in use and selected; the detail names it."""
    rows = text.split("\n")
    i = next(n for n, r in enumerate(rows) if "»dark«" in r and " › " in r)
    sel, nxt = rows[i], rows[i + 1]
    # Swap names between the selected row and the next row, keeping each row's styling/width.
    sel = sel.replace("»dark«", "»light«", 1).replace(f"«bg={SEL}»             «", f"«bg={SEL}»            «", 1)
    nxt = nxt.replace("»light«", "»dark«", 1).replace(f"«bg={PANE}»            «", f"«bg={PANE}»             «", 1)
    tail = "  Default dark theme«" + f"bg={DETAIL}»"
    k = sel.index(tail) + len(tail)
    n = len(sel[k:]) - len(sel[k:].lstrip(" "))   # detail padding; "light" twice = 2 cols more
    sel = (sel[:k].replace("» dark«", "» light«").replace(tail, "  Default light theme«" + f"bg={DETAIL}»")
           + " " * (n - 2) + sel[k + n:])
    rows[i], rows[i + 1] = sel, nxt
    return "\n".join(rows)


JOBS = [("screens/*.txt", "screens"), ("dialogs/*.txt", "dialogs"),
        ("notices/b_*.txt", "notices")]
for pat, sub in JOBS:
    for f in sorted(glob.glob(os.path.join(SRC, pat))):
        t = convert(open(f).read())
        if os.path.basename(f) == "theme.txt":
            t = theme_fix(t)
        open(os.path.join(HERE, sub, os.path.basename(f)), "w").write(t)
        print("wrote", sub + "/" + os.path.basename(f))
