#!/usr/bin/env python3
"""Render a hoocode TUI golden text file to a PNG.

The golden format is the styled grid text produced by scripts/tui/goldens.py:
    «fg=#5cc8bb;bold»hello«»world

This script converts those style runs to inline HTML styles, renders the grid
in a monospace terminal-like page, and uses headless Chrome to produce a PNG.

Usage:
    python3 scripts/tui/golden_to_png.py <golden.txt> <out.png>
    python3 scripts/tui/golden_to_png.py target/tui-goldens/chat-basic/startup.txt /tmp/startup.png

Environment:
    CHROME_BIN  path to Chrome/Chromium (defaults to macOS Google Chrome)
"""

from __future__ import annotations

import argparse
import html
import os
import re
import subprocess
import sys
from pathlib import Path

DEFAULT_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
CELL_W = 8   # px, approximate for 13px monospace
CELL_H = 17  # px, line height used by the design CSS

STYLE_RE = re.compile(r"«((?:[^»]|»)*)»")


def parse_runs(line: str) -> list[tuple[str, str]]:
    """Parse a golden line into (style, text) runs. Style may be empty."""
    runs: list[tuple[str, str]] = []
    pos = 0
    current_style = ""
    for m in STYLE_RE.finditer(line):
        # Text before the marker inherits the current style.
        if m.start() > pos:
            runs.append((current_style, line[pos : m.start()]))
        style_spec = m.group(1)
        # Update style: each «...» replaces the active style.
        current_style = style_spec
        pos = m.end()
    if pos < len(line):
        runs.append((current_style, line[pos:]))
    return runs


def style_to_css(style: str) -> str:
    """Convert a golden style spec to CSS declarations."""
    if style.startswith("<") and style.endswith(">"):
        # Masked metadata like «<masked>»
        return ""
    parts = [p.strip() for p in style.split(";") if p.strip()]
    decls: list[str] = []
    for part in parts:
        if part == "bold":
            decls.append("font-weight:700")
        elif part == "italic":
            decls.append("font-style:italic")
        elif part == "underline":
            decls.append("text-decoration:underline")
        elif part == "inverse":
            decls.append("color:#070b18;background:#e6ecff")
        elif part.startswith("fg=#"):
            decls.append(f"color:{part[3:]}")
        elif part.startswith("bg=#"):
            decls.append(f"background:{part[3:]}")
    return ";".join(decls)


def golden_to_html(path: Path) -> str:
    text = path.read_text()
    rows_html: list[str] = []
    for raw_line in text.splitlines():
        # Replace literal spaces with non-breaking spaces so trailing spaces
        # and runs of spaces are preserved in HTML.
        line = raw_line.replace("\t", "    ")
        runs = parse_runs(line)
        spans: list[str] = []
        for style, content in runs:
            if not content:
                continue
            css = style_to_css(style)
            # Preserve spaces exactly.
            safe = html.escape(content).replace(" ", "&#160;")
            if css:
                spans.append(f'<span style="{css}">{safe}</span>')
            else:
                spans.append(f"<span>{safe}</span>")
        rows_html.append(f'<div class="r">{ "".join(spans) }</div>')

    row_count = len(rows_html)
    height = max(row_count * CELL_H + 28, 200)
    width = 100 * CELL_W + 32  # 100-col terminal + padding

    return f"""<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
html,body {{ margin:0; padding:0; background:#070b18; }}
.term {{
  display:block;
  width:fit-content;
  max-width:100%;
  overflow-x:auto;
  background:#070b18;
  border:1px solid #1a2340;
  border-radius:10px;
  box-shadow:0 0 0 1px #ffffff05 inset,0 40px 80px -40px #5ce1e630;
  margin:16px;
}}
.scr {{
  box-sizing:content-box;
  padding:14px 16px;
  font:13px/17px "JetBrains Mono",ui-monospace,Menlo,Consolas,monospace;
  color:#e6ecff;
  font-variant-ligatures:none;
  font-variant-emoji:text;
}}
.r {{ display:block; white-space:pre; height:17px; line-height:17px; }}
.r span {{ display:inline-block; vertical-align:top; height:17px; }}
</style>
</head>
<body>
<div class="term"><div class="scr" style="width:100ch">
{chr(10).join(rows_html)}
</div></div>
</body>
</html>"""


def render_png(html: str, out: Path, chrome: str) -> None:
    html_path = out.with_suffix(".html")
    html_path.write_text(html)
    subprocess.run(
        [chrome, "--headless", "--disable-gpu", f"--screenshot={out}",
         "--window-size=1100,900", "--hide-scrollbars", f"file://{html_path}"],
        check=True,
        capture_output=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Render a TUI golden to PNG")
    parser.add_argument("golden", type=Path, help="Path to the .txt golden file")
    parser.add_argument("out", type=Path, help="Path for the output PNG")
    parser.add_argument("--chrome", default=os.environ.get("CHROME_BIN", DEFAULT_CHROME),
                        help="Path to Chrome/Chromium binary")
    args = parser.parse_args()

    if not args.golden.exists():
        print(f"error: golden file not found: {args.golden}", file=sys.stderr)
        return 1

    html = golden_to_html(args.golden)
    render_png(html, args.out, args.chrome)
    print(args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
