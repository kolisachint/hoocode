#!/usr/bin/env python3
"""Render hoocode TUI golden snapshots to coloured HTML and build a before/after page.

Two golden formats are auto-detected per file:
  inline     «fg=#5cc8bb;bold»text«»more   a marker replaces the active style, «» resets
  component  plain rows, then a "--- styles ---" line, then
             "r3 c61-65 fg=#d58181 bold inverse" (row, column or column range, attributes)

Public API:
  load_grid(path)           -> rows, each a list of (char, style) cells
  to_html(path, width)      -> <pre> block for one golden
  compose(parts, width)     -> <pre> block stacking goldens vertically
  build_rows(parts, width)  -> the rows behind compose(), for counting

parts items: a path (whole golden), (path, n) for the first n rows, (path, [i, j, ...])
for chosen rows, an int for that many blank rows, or a list of rows.
Rows are padded to `width` and trailing blank rows are dropped.

Usage:
  python3 render.py          # writes ../compare.html
  python3 render.py --png    # also writes ../compare.png via headless Chrome (optional)
Re-run after the mockups change; missing mockup files show "pending".
"""

from __future__ import annotations

import argparse
import html
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
GOLDEN = ROOT / "tests/golden"
MOCKUPS = HERE.parent / "mockups"
OUT_HTML = HERE.parent / "compare.html"
OUT_PNG = HERE.parent / "compare.png"
CHROME_DEFAULT = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

WIDTH = 100
DEFAULT_BG = "#14141c"
DEFAULT_FG = "#e5e5e7"
# Light theme: default terminal bg/fg of the light mockups (light/ dir).
LIGHT_BG = "#f8f8fb"
LIGHT_FG = "#2b2f36"
DIM_FG_WEIGHT = 0.6  # dim text = 60% fg, 40% bg
ROW_PX = 17

# Style = (fg, bg, bold, italic, dim, inverse, underline); fg/bg are "#rrggbb" or None.
Style = tuple
PLAIN: Style = (None, None, False, False, False, False, False)
FLAG_INDEX = {"bold": 2, "italic": 3, "dim": 4, "inverse": 5, "reverse": 5, "underline": 6}

INLINE_RE = re.compile(r"«([^»]*)»")
ANSI16 = [
    "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5",
    "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
]
COMPONENT_MARK = "--- styles ---"


class GoldenError(Exception):
    """A golden could not be read or parsed."""


def warn(msg: str) -> None:
    print(f"warning: {msg}", file=sys.stderr)


# ---------------------------------------------------------------- styles


def palette_hex(n: int) -> str:
    """xterm 256-colour index -> hex."""
    if n < 16:
        return ANSI16[n]
    if n < 232:
        n -= 16
        levels = [0, 95, 135, 175, 215, 255]
        r, g, b = levels[n // 36], levels[(n // 6) % 6], levels[n % 6]
        return f"#{r:02x}{g:02x}{b:02x}"
    v = 8 + 10 * (n - 232)
    return f"#{v:02x}{v:02x}{v:02x}"


def parse_color(value: str) -> str | None:
    value = value.strip().lower()
    if re.fullmatch(r"#[0-9a-f]{6}", value):
        return value
    if re.fullmatch(r"#[0-9a-f]{3}", value):
        return "#" + "".join(ch * 2 for ch in value[1:])
    if value.isdigit() and int(value) < 256:
        return palette_hex(int(value))
    return None


def apply_token(style: Style, token: str) -> Style | None:
    """Return style with one attribute token applied, or None if the token is unknown."""
    t = token.strip().lower()
    s = list(style)
    if t in FLAG_INDEX:
        s[FLAG_INDEX[t]] = True
    elif t.startswith("fg=") or t.startswith("bg="):
        color = parse_color(t[3:])
        if color is None:
            return None
        s[0 if t.startswith("fg=") else 1] = color
    else:
        return None
    return tuple(s)


def parse_inline_style(spec: str, where: str) -> Style:
    spec = spec.strip()
    # Empty marker is a reset; «<masked: ...>» is golden metadata and renders unstyled.
    if not spec or (spec.startswith("<") and spec.endswith(">")):
        return PLAIN
    style = PLAIN
    for token in spec.split(";"):
        if not token.strip():
            continue
        new = apply_token(style, token)
        if new is None:
            warn(f"{where}: unknown style token {token!r}")
        else:
            style = new
    return style


# ---------------------------------------------------------------- parsing


def _parse_inline_line(line: str, where: str) -> list:
    row: list = []
    style = PLAIN
    pos = 0
    for m in INLINE_RE.finditer(line):
        row.extend((ch, style) for ch in line[pos : m.start()])
        style = parse_inline_style(m.group(1), where)
        pos = m.end()
    row.extend((ch, style) for ch in line[pos:])
    return row


def _apply_component_line(rows: list, line: str, where: str) -> None:
    parts = line.split()
    if not parts:
        return
    m_r = re.fullmatch(r"r(\d+)", parts[0])
    m_c = re.fullmatch(r"c(\d+)(?:-(\d+))?", parts[1]) if len(parts) > 1 else None
    if not (m_r and m_c):
        raise GoldenError(f"{where}: bad style line {line!r}")
    r = int(m_r.group(1))
    c0 = int(m_c.group(1))
    c1 = int(m_c.group(2) or c0)
    if c1 < c0:
        raise GoldenError(f"{where}: bad column range in {line!r}")
    valid = []
    for tok in parts[2:]:
        if apply_token(PLAIN, tok) is None:
            warn(f"{where}: unknown style token {tok!r} in {line!r}")
        else:
            valid.append(tok)
    while len(rows) <= r:
        rows.append([])
    row = rows[r]
    while len(row) <= c1:
        row.append((" ", PLAIN))
    for c in range(c0, c1 + 1):
        ch, st = row[c]
        for tok in valid:
            st = apply_token(st, tok)
        row[c] = (ch, st)


def load_grid(path) -> list:
    """Parse one golden file into rows of (char, style) cells."""
    path = Path(path)
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        raise GoldenError(f"{path}: {exc}") from exc
    lines = [ln[:-1] if ln.endswith("\r") else ln for ln in text.split("\n")]
    if lines and lines[-1] == "":
        lines.pop()
    where = path.name
    if COMPONENT_MARK in lines:
        idx = lines.index(COMPONENT_MARK)
        rows = [[(ch, PLAIN) for ch in line] for line in lines[:idx]]
        for line in lines[idx + 1 :]:
            _apply_component_line(rows, line, where)
        return rows
    return [_parse_inline_line(line, where) for line in lines]


# ---------------------------------------------------------------- composition


def fit_row(row: list, width: int, where: str) -> list:
    if len(row) > width:
        warn(f"{where}: row has {len(row)} cells, truncated to {width}")
        row = row[:width]
    if len(row) < width:
        row = row + [(" ", PLAIN)] * (width - len(row))
    return row


def is_blank(row: list) -> bool:
    return all(ch == " " and st[1] is None and not st[5] and not st[6] for ch, st in row)


class Right:
    """A golden pushed to the right edge: its rows end at the last column of the screen."""

    def __init__(self, path) -> None:
        self.path = Path(path)


def _part_rows(part, width: int = WIDTH) -> tuple[str, list]:
    if isinstance(part, Right):
        src_rows = load_grid(part.path)
        pad = width - max((len(r) for r in src_rows), default=0)
        return part.path.name, [[(" ", PLAIN)] * pad + r for r in src_rows]
    if isinstance(part, int):
        return "blank", [[] for _ in range(part)]
    if isinstance(part, list):
        return "grid", part
    if isinstance(part, tuple):
        path, sel = part
    else:
        path, sel = part, None
    name = Path(path).name
    src = load_grid(path)
    if sel is None:
        return name, src
    if isinstance(sel, int):
        return name, src[:sel]
    picked = []
    for i in sel:
        if not 0 <= i < len(src):
            raise GoldenError(f"{name}: row {i} out of range (golden has {len(src)} rows)")
        picked.append(src[i])
    return name, picked


def build_rows(parts, width: int = WIDTH) -> list:
    rows: list = []
    for part in parts:
        name, part_rows = _part_rows(part, width)
        rows.extend(fit_row(r, width, name) for r in part_rows)
    while rows and is_blank(rows[-1]):
        rows.pop()
    return rows


# ---------------------------------------------------------------- HTML


def mix(a: str, b: str, weight: float) -> str:
    """Blend hex colour a (weight) with b (1 - weight)."""
    ca = [int(a[i : i + 2], 16) for i in (1, 3, 5)]
    cb = [int(b[i : i + 2], 16) for i in (1, 3, 5)]
    return "#" + "".join(f"{round(weight * x + (1 - weight) * y):02x}" for x, y in zip(ca, cb))


def style_css(style: Style, base_bg: str = DEFAULT_BG, base_fg: str = DEFAULT_FG) -> str:
    """CSS for one style. Cells without their own colour take the screen's base_bg / base_fg."""
    fg, bg, bold, italic, dim, inverse, underline = style
    if inverse:
        fg, bg = bg or base_bg, fg or base_fg
    decl = []
    if fg or dim or inverse:
        color = fg or base_fg
        if dim:
            color = mix(color, bg or base_bg, DIM_FG_WEIGHT)
        decl.append(f"color:{color}")
    if bg or inverse:
        decl.append(f"background:{bg or base_bg}")
    if bold:
        decl.append("font-weight:700")
    if italic:
        decl.append("font-style:italic")
    if underline:
        decl.append("text-decoration:underline")
    return ";".join(decl)


def row_html(row: list, base_bg: str = DEFAULT_BG, base_fg: str = DEFAULT_FG) -> str:
    out = []
    i = 0
    while i < len(row):
        st = row[i][1]
        j = i
        while j < len(row) and row[j][1] == st:
            j += 1
        text = html.escape("".join(ch for ch, _ in row[i:j]), quote=False)
        css = style_css(st, base_bg, base_fg)
        out.append(f'<span style="{css}">{text}</span>' if css else text)
        i = j
    return "".join(out)


def render_pre(rows: list, width: int = WIDTH, base_bg: str = DEFAULT_BG,
               base_fg: str = DEFAULT_FG) -> str:
    body = "\n".join(row_html(r, base_bg, base_fg) for r in rows)
    return (
        f'<pre class="tui" style="width:{width}ch;margin:0;background:{base_bg};'
        f'color:{base_fg};font:13px/{ROW_PX}px ui-monospace,Menlo,Consolas,monospace;'
        f'white-space:pre">{body}</pre>'
    )


def to_html(path, width: int = WIDTH) -> str:
    """One golden as a <pre> block, padded to `width` columns."""
    return render_pre(build_rows([path], width), width)


def compose(parts, width: int = WIDTH) -> str:
    """Several goldens (and blank rows) stacked vertically as one <pre> block."""
    return render_pre(build_rows(parts, width), width)


# ---------------------------------------------------------------- screens

CORE = GOLDEN / "hoocode-code-tui-app"
SELECTORS = GOLDEN / "hoocode-code-tui-selectors"
WIDGETS = GOLDEN / "hoocode-code-tui-widgets"
COMPONENTS = GOLDEN / "hoocode-tui-components"
HUD = CORE / "hud_row_100x1.txt"
COMMS = CORE / "comms_ready_100x3.txt"
START_ROWS = [0, 16, 17, 18, 19, 20]  # HUD, banner, resource line of answer.txt
DARK_SCREENS = MOCKUPS / "dark/screens"
DARK_DIALOGS = MOCKUPS / "dark/dialogs"
LIGHT = MOCKUPS / "light"  # light variants of the after columns


# Dialogs that take the prompt's slot (show_in_editor_slot): no comms frame below them.
REPLACING = {"permission", "question", "quit", "models", "settings", "sessions", "theme",
             "tree", "fork", "login", "scoped_models", "keys"}


def layers(name: str) -> Path:
    return CORE / f"layers_{name}_80.txt"


BEFORE = {
    # Spec said "first 5 rows", but answer.txt rows 1-4 are blank; its banner and
    # resource line are at rows 16-20. Use those so the start screen shows its content.
    "start": [(GOLDEN / "tui/chat-basic/answer.txt", START_ROWS), COMMS],
    "peek": [HUD, 1, layers("landed_peek"), 1, COMMS],
    "outline": [HUD, 1, layers("landed_radar"), 1, COMMS],
    "full": [HUD, 1, layers("landed_full"), 1, COMMS],
    "failed": [HUD, 1, layers("failed_peek"), 1, COMMS],
    "agent": [HUD, 1, layers("agent_radar"), 1, COMMS],
    "working": [GOLDEN / "tui/message-queue/queued.txt"],
}

SCREEN_TITLES = [
    ("start", "Start screen"),
    ("peek", "Peek view"),
    ("outline", "Outline view (was radar)"),
    ("full", "Full view"),
    ("failed", "Failed turn (peek)"),
    ("agent", "Subagent turn (outline)"),
    ("working", "Working state"),
]

# (key, title, dialog golden). BEFORE = HUD, blank, dialog, then blank + comms unless in REPLACING.
DIALOGS = [
    ("permission", "Permission", CORE / "clearance_shell_100.txt"),
    ("question", "Question", SELECTORS / "ask_card_100.txt"),
    ("tasks", "Tasks panel (unfocused)", WIDGETS / "task_panel_tabs_unfocused_80.txt"),
    # Focused = cursor row and "move · tab · enter open" hints (flight_plan_shell_80 is the alt).
    ("tasks_focused", "Tasks panel (focused)", WIDGETS / "flight_plan_agents_80.txt"),
    ("quit", "Quit with jobs", CORE / "end_shift_jobs_100.txt"),
    ("models", "Models (/model)", SELECTORS / "hangar_100.txt"),
    ("settings", "Settings", SELECTORS / "systems_100.txt"),
    ("sessions", "Sessions (/resume)", SELECTORS / "mission_log_100.txt"),
    ("theme", "Theme", SELECTORS / "hull_100.txt"),
    ("tree", "Tree", SELECTORS / "flight_path_100.txt"),
    ("fork", "Fork", SELECTORS / "stage_separation_100.txt"),
    ("login", "Login", SELECTORS / "docking_provider_100.txt"),
    ("scoped_models", "Scoped models", SELECTORS / "scoped_models_picker_100.txt"),
    ("slash", "Slash dropdown", COMPONENTS / "slash_dropdown_100.txt"),
    ("files", "File dropdown", COMPONENTS / "files_dropdown_100.txt"),
    ("keys", "Keys", WIDGETS / "controls_100.txt"),
]


DARK_NOTICES = MOCKUPS / "dark/notices"
COLOR = GOLDEN / "tui/color-chrome"
TOAST = CORE / "transmission_toast_46.txt"

# (anchor id, title, before parts, file stem in after-notices-v2, before note)
NOTICES = [
    ("n-info-undo", "Info with undo", [COLOR / "chrome-compact.txt"], "info_undo", ""),
    ("n-info-value", "Info with value", [COLOR / "dial.txt"], "info_value", ""),
    ("n-warning", "Warning (sign in)", [HUD, 1, Right(TOAST)], "warning",
     "approximation: real text differs (no screen golden for this notice; "
     "transmission toast composed onto the HUD row)"),
    ("n-usage", "Usage message", [COLOR / "usage.txt"], "usage", ""),
]


def _sources(parts) -> str:
    names = []
    for part in parts:
        if isinstance(part, Right):
            part = part.path
        if isinstance(part, tuple):
            part = part[0]
        if isinstance(part, (str, Path)):
            p = Path(part)
            try:
                names.append(str(p.relative_to(GOLDEN)))
            except ValueError:
                names.append(str(p))
    return ", ".join(names)


def mockup_src(path: Path) -> str:
    return f"{path.parent.parent.name}/{path.parent.name}/{path.name}"


def light_src(path: Path) -> str:
    return f"light/{path.parent.name}/{path.name}"


def mockup(path: Path):
    """Parts for a mockup file, or None while it is not written yet."""
    return [path] if path.exists() else None


def column(label: str, src: str, body: str, css_class: str, mode: str | None = None) -> str:
    theme = f" theme-{mode}" if mode else ""
    return (
        f'<div class="col {css_class}{theme}"><div class="label">{label}'
        f'<span class="src">{html.escape(src)}</span></div>'
        f'<div class="box">{body}</div></div>'
    )


def pane(label: str, css_class: str, src: str, parts, mode: str | None = None) -> tuple[str, int, str]:
    """One column: (html, rows, row-count text).

    parts=None means the file is not written yet; a str is a placeholder message.
    mode "dark" or "light" shows the column only in that theme (None = always, dark).
    """
    if parts is None:
        body = f'<div class="pending">pending: {html.escape(src)} not written yet</div>'
        return column(label, src, body, css_class, mode), 0, "pending"
    if isinstance(parts, str):
        body = f'<div class="pending">{html.escape(parts)}</div>'
        return column(label, src, body, css_class, mode), 0, parts
    base_bg, base_fg = (LIGHT_BG, LIGHT_FG) if mode == "light" else (DEFAULT_BG, DEFAULT_FG)
    try:
        rows = build_rows(parts, WIDTH)
    except GoldenError as exc:
        warn(f"{label}: {exc}")
        body = f'<div class="err">{html.escape(str(exc))}</div>'
        return column(label, src, body, css_class, mode), 0, "error"
    body = render_pre(rows, WIDTH, base_bg, base_fg)
    return column(label, src, body, css_class, mode), len(rows), f"{len(rows)} rows"


def section(item: dict, heights: list, counts: list) -> str:
    cols, texts, row_counts = [], [], []
    for label, css_class, src, parts, mode in item["panes"]:
        col_html, rows, text = pane(label, css_class, src, parts, mode)
        cols.append(col_html)
        texts.append(f"{label}: {text}")
        row_counts.append(rows)
    heights.append(max(row_counts + [1]))
    summary = " · ".join(texts)
    counts.append((item["title"], summary))
    note = f'<p class="note">{html.escape(item["note"])}</p>' if item.get("note") else ""
    pair_class = "pair tri" if item.get("tri") else "pair"
    return f"""<section id="{item["anchor"]}" class="{item.get("cls", "")}">
  <h3>{html.escape(item["title"])}</h3>
  {note}
  <div class="{pair_class}">
    {"".join(cols)}
  </div>
  <p class="counts">{html.escape(summary)}</p>
</section>"""


def after_pair(label: str, css_class: str, dark_path: Path, light_path: Path | None) -> list:
    """An after column as two panes: the dark file in dark mode, the light file in light mode.

    light_path=None means no light version exists: the light pane says "dark only".
    """
    dark = (f"{label} · dark", css_class, mockup_src(dark_path), mockup(dark_path), "dark")
    if light_path is None:
        light = (f"{label} · light", css_class, "no light version", "dark only", "light")
    else:
        light = (f"{label} · light", css_class, light_src(light_path),
                 mockup(light_path), "light")
    return [dark, light]


MONO = MOCKUPS / "mono"  # NO_COLOR mockups, same names as the dark files
MONO_DARK = {"screens": DARK_SCREENS, "dialogs": DARK_DIALOGS, "notices": DARK_NOTICES}
FIRST_RUN_NOTE = "approximation: today a timed toast"


def mono_sections() -> list[dict]:
    """One section per mono file: the dark after file next to its no-colour version."""
    titles = dict(SCREEN_TITLES)
    titles.update({key: title for key, title, _ in DIALOGS})
    titles.update({f"b_{stem}": title for _, title, _, stem, _ in NOTICES})
    items = []
    for group in ("screens", "dialogs", "notices"):
        for path in sorted((MONO / group).glob("*.txt")):
            dark = MONO_DARK[group] / path.name
            items.append({
                "anchor": f"m-{group}-{path.stem}",
                "title": titles.get(path.stem, path.stem),
                "panes": [
                    ("Colour (dark)", "after", mockup_src(dark), mockup(dark), None),
                    ("No colour", "mono", mockup_src(path), mockup(path), None),
                ],
            })
    return items


def all_parts() -> list[tuple[str, str, list[dict]]]:
    """(part id, part title, [section items])."""
    screens = [
        {"anchor": f"s-{key}", "title": title,
         "panes": [("Before", "before", _sources(BEFORE[key]), BEFORE[key], None)]
         + after_pair("after", "v2", DARK_SCREENS / f"{key}.txt", LIGHT / "screens" / f"{key}.txt")}
        for key, title in SCREEN_TITLES
    ]
    dialogs = [
        {"anchor": f"d-{key}", "title": title, "cls": "dialog",
         "panes": [("Before", "before", _sources([HUD, 1, golden, 1, COMMS]),
                    [HUD, 1, golden, 1, COMMS], None)]
         + after_pair("after", "v3", DARK_DIALOGS / f"{key}.txt", LIGHT / "dialogs" / f"{key}.txt")}
        for key, title, golden in DIALOGS
    ]
    notices = [
        {"anchor": anchor, "title": title, "note": note,
         "panes": [("Before", "before", _sources(before_parts), before_parts, None)]
         + after_pair("after", "v2b", DARK_NOTICES / f"b_{stem}.txt",
                      LIGHT / "notices" / f"b_{stem}.txt")}
        for anchor, title, before_parts, stem, note in NOTICES
    ]
    first_run_before = [HUD, 1, Right(TOAST)]
    first_run = [
        {"anchor": anchor, "title": title, "note": FIRST_RUN_NOTE, "panes": [("Before", "before", _sources(first_run_before), first_run_before, None)]
         + after_pair("after", "v3", DARK_DIALOGS / f"{stem}.txt",
                      LIGHT / "dialogs" / f"{stem}.txt")}
        for anchor, title, stem in [
            ("fr-login", "Log in opens on its own", "first_run_login"),
            ("fr-dismissed", "After esc", "first_run_dismissed"),
        ]
    ]
    return [
        ("screens", "Screens", screens),
        ("dialogs", "Dialogs", dialogs),
        ("notices", "Notices", notices),
        ("first-run", "First run", first_run),
        ("no-colour", "No colour (NO_COLOR)", mono_sections()),
    ]


CSS = """
html, body { margin: 0; background: #0e0e14; color: #c9c9d1; }
body { font-family: -apple-system, BlinkMacSystemFont, "Helvetica Neue", Helvetica, Arial, sans-serif;
  padding: 24px 20px 48px; }
header h1 { font-size: 22px; font-weight: 600; color: #e5e5e7; margin: 0 0 6px; }
header p { font-size: 13px; color: #8a8a96; margin: 0 0 16px; }
.toolbar { margin: 0 0 20px; }
.toolbar label { display: inline-flex; align-items: center; gap: 8px; cursor: pointer;
  font-size: 13px; color: #c9c9d1; padding: 6px 12px; background: #14141c;
  border: 1px solid #23232e; border-radius: 999px; user-select: none; }
.toolbar input { margin: 0; accent-color: #9aa4ff; }
nav { font-size: 13px; margin: 0 0 8px; padding: 12px 14px; background: #14141c;
  border: 1px solid #23232e; border-radius: 12px; }
nav .grp { display: block; font-size: 11px; font-weight: 600; letter-spacing: 0.06em;
  text-transform: uppercase; color: #6b6b76; margin: 6px 0 6px; }
nav .grp:first-child { margin-top: 0; }
nav a { color: #9aa4ff; text-decoration: none; margin-right: 14px; white-space: nowrap; }
nav a:hover { text-decoration: underline; }
h2.part { font-size: 20px; font-weight: 600; color: #e5e5e7; margin: 40px 0 18px;
  padding-top: 18px; border-top: 1px solid #23232e; }
section { margin: 0 0 40px; scroll-margin-top: 12px; }
section h3 { font-size: 16px; font-weight: 600; color: #e5e5e7; margin: 0 0 12px; }
.note { font-size: 13px; color: #8a8a96; margin: -4px 0 12px; }
.pair { display: grid; grid-template-columns: minmax(0, 1fr); gap: 20px; align-items: start; }
@media (min-width: 1680px) { .pair { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
@media (min-width: 2540px) { .pair.tri { grid-template-columns: repeat(3, minmax(0, 1fr)); } }
.col { min-width: 0; }
.label { font-size: 12px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase;
  color: #9a9aa6; margin: 0 0 8px 4px; }
.pair.tri .label { font-size: 13px; letter-spacing: 0; text-transform: none; }
.src { font-weight: 400; letter-spacing: 0; text-transform: none; color: #6b6b76; margin-left: 10px; }
.box { background: #14141c; border: 1px solid #23232e; border-radius: 14px; padding: 12px 14px;
  overflow-x: auto; }
.pending { color: #6b6b76; font-size: 14px; font-style: italic; padding: 40px 16px; text-align: center;
  background: #14141c; border: 1px dashed #2c2c38; border-radius: 14px; }
.err { color: #ff8a8a; font: 13px/17px ui-monospace, Menlo, Consolas, monospace; white-space: pre-wrap; }
.counts { font-size: 12px; color: #8a8a96; margin: 10px 0 0 4px; }
.seg { display: inline-flex; background: #14141c; border: 1px solid #23232e; border-radius: 999px;
  overflow: hidden; margin-right: 12px; vertical-align: middle; }
.seg button { font: inherit; font-size: 13px; color: #c9c9d1; background: none; border: 0;
  padding: 6px 14px; cursor: pointer; }
.seg button[aria-pressed="true"] { background: #23232e; color: #e5e5e7; }
.theme-light { display: none; }
body.light .theme-dark { display: none; }
body.light .theme-light { display: block; }
.theme-light .box { background: #f8f8fb; border-color: #d4d4de; }
.theme-light .pending { background: transparent; border-color: #c4c4d0; color: #555864; }
"""

SCRIPT = """
(function () {
  function setTheme(light) {
    document.body.classList.toggle('light', light);
    document.getElementById('theme-dark').setAttribute('aria-pressed', String(!light));
    document.getElementById('theme-light').setAttribute('aria-pressed', String(light));
  }
  document.getElementById('theme-dark').addEventListener('click', function () { setTheme(false); });
  document.getElementById('theme-light').addEventListener('click', function () { setTheme(true); });
  setTheme(false);
})();
"""


def page(body_html: str, nav_html: str) -> str:
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>hoocode TUI declutter: before / after</title>
<style>{CSS}</style>
</head>
<body>
<header>
  <h1>hoocode TUI declutter: before / after</h1>
  <p>Every screen 100 columns. Before = current goldens (tests/golden); after = docs/design/tui-declutter/mockups. Generated by tools/render.py.
  Dark | Light switches the after columns to the light/ variants.
  Notices show the status-line option. First run: the login and esc states, before = today's timed toast. No colour: NO_COLOR mockups beside their dark after file; the theme switch does not affect them.</p>
</header>
<div class="toolbar"><span class="seg" role="group" aria-label="Theme"><button type="button" id="theme-dark" aria-pressed="true">Dark</button><button type="button" id="theme-light" aria-pressed="false">Light</button></span></div>
<nav>
{nav_html}
</nav>
{body_html}
<script>{SCRIPT}</script>
</body>
</html>
"""


# ---------------------------------------------------------------- PNG


def write_png(height: int) -> bool:
    chrome = os.environ.get("CHROME_BIN") or CHROME_DEFAULT
    cmd = [
        chrome, "--headless", "--disable-gpu", "--hide-scrollbars",
        f"--window-size=1700,{height}", f"--screenshot={OUT_PNG}", OUT_HTML.as_uri(),
    ]
    try:
        subprocess.run(cmd, check=True, capture_output=True, timeout=120)
    except (OSError, subprocess.SubprocessError) as exc:
        warn(f"PNG skipped, headless Chrome failed ({chrome}): {exc}")
        return False
    if not OUT_PNG.exists():
        warn("PNG skipped, Chrome exited cleanly but wrote no file")
        return False
    return True


# ---------------------------------------------------------------- main


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--png", action="store_true", help="also write index.png via headless Chrome")
    args = ap.parse_args(argv)

    heights: list[int] = []
    counts: list[tuple[str, str]] = []
    body: list[str] = []
    nav: list[str] = []
    for part_id, part_title, items in all_parts():
        body.append(f'<h2 class="part" id="{part_id}">{html.escape(part_title)}</h2>')
        links = []
        for item in items:
            body.append(section(item, heights, counts))
            links.append(f'<a href="#{item["anchor"]}">{html.escape(item["title"])}</a>')
        nav.append(f'<span class="grp"><a href="#{part_id}">{html.escape(part_title)}</a></span>'
                   + "".join(links))

    OUT_HTML.write_text(page("\n".join(body), "\n".join(nav)), encoding="utf-8")
    print(f"wrote {OUT_HTML}")
    width = max(len(t) for t, _ in counts)
    for title, summary in counts:
        print(f"  {title:<{width}}  {summary}")

    if args.png:
        # header + per section (rows * row height + heading/caption/padding) + slack
        page_h = 260 + sum(h * ROW_PX + 150 for h in heights) + 120
        if write_png(page_h):
            print(f"wrote {OUT_PNG}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
