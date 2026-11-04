#!/usr/bin/env python3
"""NO_COLOR versions of the dark mockups (v2.9): no fg/bg at all, only bold/dim/italic/reverse.
Panes get full light borders (dim) with the title in the top border, split panes a │ rule
in the gutter, the pane input row a ├─┤ rule above it; the selected row is ` › ` + bold.
User messages and the prompt lose their fill; a user message keeps a bold ` › `."""
import os, re

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "mockups", "mono")
SRC = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "mockups", "dark")
W = 100
DIM, PANE, DETAIL, SEL, SURF = "#989898", "#1e1e2a", "#292b40", "#233239", "#2a2a36"
CHIPS = {"#5cc8bb", "#f0c674"}
PANEISH = {PANE, DETAIL, SEL} | CHIPS
MARK = re.compile(r"«([^»]*)»")
JOBS = [("screens", "screens", ["peek", "working", "failed"]),
        ("dialogs", "dialogs",
         ["permission", "question", "models", "settings", "sessions", "tasks", "slash"]),
        ("notices", "notices", ["b_warning"])]


def parse(line):
    """-> list of [ch, fg, bg, bold, italic]"""
    cells, st = [], (None, None, False, False)
    parts = MARK.split(line)
    for i, p in enumerate(parts):
        if i % 2:
            kv = dict(a.split("=") for a in p.split(";") if "=" in a)
            fl = p.split(";")
            st = (kv.get("fg"), kv.get("bg"), "bold" in fl, "italic" in fl)
        else:
            cells += [[c, *st] for c in p]
    return cells


def mono(ch, dim=False, bold=False, italic=False):
    return (ch, (bold, dim and not bold, italic))


def plainify(cells, selected=False):
    # the selected row (pane selection bg) turns bold; only its own cells, not the detail
    return [mono(c, dim=(fg == DIM), bold=b or (bg == SEL and c != " "), italic=i)
            for c, fg, bg, b, i in cells]


def border(left, right, width, title=None, tee=None, tee_ch="┬"):
    """A horizontal rule `width` cols wide (incl. corners); title = list of mono cells."""
    mid = [mono("─", dim=True) for _ in range(width - 2)]
    if title:
        t = [mono("─", dim=True), mono(" ")] + title + [mono(" ")]
        mid[:len(t)] = t
    if tee is not None and (not title or tee > len(title) + 3):
        mid[tee - 1] = mono(tee_ch, dim=True)
    return [mono(left, dim=True)] + mid + [mono(right, dim=True)]


def title_cells(cells):
    """chip text bold, the rest keeps dim/bold; joined by ' ─ ' after the chip."""
    chip = [c for c in cells if c[2] in CHIPS]
    k = max(i for i, c in enumerate(cells) if c[2] in CHIPS) + 1
    rest = cells[k:]
    while rest and rest[-1][0] == " ":
        rest.pop()
    while rest and rest[0][0] == " ":
        rest.pop(0)
    name = "".join(c[0] for c in chip).strip()
    t = [mono(ch, bold=True) for ch in name]
    if rest:
        t += [mono(" "), mono("─", dim=True), mono(" ")] + plainify(rest)
    return t


def convert(lines):
    rows = [parse(l) for l in lines]
    bgs = [{c[2] for c in r if c[2]} for r in rows]
    n = len(rows)
    inpane = [bool(b & PANEISH) for b in bgs]
    for i in range(n):   # pane input row: surface row followed by a pane row
        if bgs[i] == {SURF} and i + 1 < n and inpane[i + 1]:
            inpane[i] = "input"
    out, i = [], 0
    while i < n:
        if not inpane[i]:
            r = rows[i]
            if bgs[i] == {SURF} and i != n - 2:      # user message band: bold ›
                cells = plainify(r)
                if "".join(c for c, _ in cells[:3]) == " › ":
                    cells[1] = mono("›", bold=True)
                out.append(cells)
            else:
                out.append(plainify(r))
            i += 1
            continue
        j = i
        while j < n and inpane[j]:
            j += 1
        block = rows[i:j]
        cols = [k for r in block for k, c in enumerate(r) if c[2]]
        c0, c1 = min(cols), max(cols) + 1
        full = c0 == 0 and c1 == W
        # split gutter: pane cell right before the first detail cell
        gut = None
        for r in block:
            d = [k for k, c in enumerate(r) if c[2] == DETAIL]
            if d:
                gut = d[0] - 1
                break
        first = block[0]
        has_title = any(c[2] in CHIPS for c in first)
        if full:
            width = W                  # borders at 0 and 99
        else:
            width = c1 - c0 + 2
            assert c0 >= 1 and c1 <= W - 1
        split_drop = None
        if gut is not None and full:
            srows = [r for r in block if any(c[2] == DETAIL for c in r)]
            # prefer two blanks at the end of the list column, so the detail keeps its pad
            two = all(r[gut - 1][0] == " " and r[gut - 2][0] == " " for r in srows)
            split_drop = {gut - 2, gut - 1} if two else {gut - 1, W - 1}
        g = None if gut is None else (gut - (1 if split_drop == {gut - 2, gut - 1} else 0)
                                      if full else gut - c0 + 1)
        def boxrow(cells, selected=False, split=False):
            m = plainify(cells, selected)
            if full:
                # drop 2 blank cells to make room for the borders: the last column, and on
                # split rows the list column's last cell (so the detail keeps its width)
                drop = split_drop if (split and gut is not None) else {W - 2, W - 1}
                assert all(m[k][0] == " " for k in drop), "".join(c for c, _ in m)
                inner = [c for k, c in enumerate(m) if k not in drop]
            else:
                inner = m[c0:c1]
            row = [mono("│", dim=True)] + inner + [mono("│", dim=True)]
            if split and g is not None:
                row[g] = mono("│", dim=True)
            return ([] if full else [mono(" ")] * (c0 - 1)) + row
        lead = [] if full else [mono(" ")] * (c0 - 1)
        body = block[1:] if has_title else block
        splitrow = lambda r: any(c[2] == DETAIL for c in r)
        top_tee = g if (g is not None and body and splitrow(body[0])) else None
        out.append(lead + border("┌", "┐", width, title_cells(first) if has_title else None, top_tee))
        prev_split = False
        for k, r in enumerate(body):
            if inpane[i + (1 if has_title else 0) + k] == "input":
                out.append(lead + border("├", "┤", width, None, g if prev_split else None, "┴"))
                prev_split = False
            sp = splitrow(r)
            out.append(boxrow(r, selected=any(c[2] == SEL for c in r[:3]), split=sp))
            prev_split = sp
        out.append(lead + border("└", "┘", width, None, g if prev_split else None, "┴"))
        i = j
    return out


def fmt(row):
    s, cur = [], (False, False, False)
    for ch, st in row:
        if st != cur:
            s.append("«" + ";".join(nm for nm, f in zip(("bold", "dim", "italic"), st) if f) + "»")
            cur = st
        s.append(ch)
    if cur != (False, False, False):
        s.append("«»")
    return "".join(s).rstrip(" ") if not any(c == "│" for c, _ in row[-1:]) else "".join(s)


for src, sub, names in JOBS:
    for nm in names:
        lines = open(os.path.join(SRC, src, nm + ".txt")).read().splitlines()
        rows = convert(lines)
        open(os.path.join(HERE, sub, nm + ".txt"), "w").write("\n".join(fmt(r) for r in rows) + "\n")
        print("wrote", f"{sub}/{nm}.txt")
