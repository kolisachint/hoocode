#!/usr/bin/env python3
"""Review bundles for TUI golden changes (T0.4).

Reads the output of ``goldens.py run`` (``target/tui-goldens/<scenario>/``). For each
snapshot whose screen differs from its golden in ``tests/golden/tui/<scenario>/`` (or
has no golden yet), writes ``target/tui-review/<scenario>/<snap>/``:

* ``before.txt``  the golden (omitted for a new snapshot)
* ``after.txt``   the run output
* ``diff.txt``    unified diff, golden to run output
* ``legend.txt``  each style key used in ``after.txt`` with a short colour description
* ``after.png``   only with ``--png``: the screen rendered via ``render_png.mjs``

It also writes ``target/tui-review/index.md`` listing every bundle.

Usage::

    review_bundle.py [scenario|all] [--png] [--snap NAME]

Run ``scripts/tui/goldens.py run <scenario|all>`` first. This tool does not run the app.
It clears ``target/tui-review/`` at start. It is a review aid, not a gate: it exits 0
after writing bundles (including when there are none) and 2 when there is no run output.

``--png`` needs ``node`` with the ``playwright`` package (found through ``npm root -g``
unless ``NODE_PATH`` is set) and a Chromium it can launch. Set ``PLAYWRIGHT_CHROMIUM``
to a browser executable if the default one is missing. Without a working node or
browser the PNG is skipped with a warning. Snapshot text is parsed from the ``«key»text``
format, so a literal ``«`` or ``»`` in a screen will be read as a style marker.
"""

from __future__ import annotations

import argparse
import colorsys
import difflib
import html
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import goldens as G  # noqa: E402  (shares the output/golden paths and golden loader)

REVIEW = G.ROOT / "target" / "tui-review"
RENDER = G.TUI_DIR / "render_png.mjs"
# Files in a run's output directory that are not snapshots.
NOT_SNAPSHOTS = {"error.txt", "last-screen.txt"}
MARKER = re.compile(r"«([^»]*)»")
HEX = re.compile(r"^#[0-9a-fA-F]{6}$")
ATTR_WORDS = {"bold", "dim", "italic", "underline", "blink", "inverse", "hidden", "strike"}
# Hue boundaries in degrees, first match wins.
HUES = [(15, "red"), (45, "orange"), (70, "yellow"), (165, "green"), (195, "teal"),
        (255, "blue"), (290, "purple"), (345, "magenta"), (361, "red")]


# ---------------------------------------------------------------------------
# Parsing and style legend
# ---------------------------------------------------------------------------


def parse_styled(text: str) -> list[list[tuple[str, str]]]:
    """The goldens' styled grid format (goldens.grid_styled) back into (char, key) cells."""
    grid = []
    for line in text.rstrip("\n").split("\n"):
        parts = MARKER.split(line)  # ['', key, text, key, text, ...]
        cells: list[tuple[str, str]] = []
        for key, chunk in zip(parts[1::2], parts[2::2]):
            cells.extend((ch, key) for ch in chunk)
        grid.append(cells)
    return grid


def color_name(value: str) -> str:
    if not HEX.match(value):
        return value
    r, g, b = (int(value[i:i + 2], 16) / 255 for i in (1, 3, 5))
    h, l, s = colorsys.rgb_to_hls(r, g, b)
    if s < 0.15:
        for limit, name in ((0.12, "black"), (0.4, "dark gray"), (0.75, "gray"), (0.92, "light gray")):
            if l < limit:
                return name
        return "white"
    deg = h * 360
    base = next(name for limit, name in HUES if deg < limit)
    if l < 0.3:
        return "dark " + base
    if l > 0.8:
        return "light " + base
    return base


def describe_key(key: str) -> tuple[str, str]:
    """A style key ("bold;fg=#5cc8bb") as (style, meaning): ("fg=#5cc8bb bold", "teal bold").

    Colours come first on both sides; the key's own order (sorted by name) is kept
    for the rest."""
    if not key:
        return "(default)", "default colours"
    items = key.split(";")
    items.sort(key=lambda item: 0 if item.split("=", 1)[0] in ("fg", "bg", "ul") else 1)
    words = []
    for item in items:
        name, _, value = item.partition("=")
        if name == "fg":
            words.append(color_name(value))
        elif name == "bg":
            words.append("on " + color_name(value))
        elif name == "ul":
            words.append("underline colour " + color_name(value))
        elif name in ATTR_WORDS:
            words.append(name)
        elif item == "<masked>":
            words.append("masked (random in the run, normalised)")
        else:
            words.append(item)
    return " ".join(items), " ".join(words)


def legend_text(after: str) -> str:
    keys = {key for row in parse_styled(after) for _, key in row}
    lines = ["# style key -> meaning (colour names are approximate: from hue and lightness)"]
    for key in sorted(keys):
        style, meaning = describe_key(key)
        lines.append(f"{style} -> {meaning}")
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------
# PNG (copy of harness.grid_to_html, plus a page wrapper)
# ---------------------------------------------------------------------------


def grid_to_html(grid: list[list[tuple[str, str]]]) -> str:
    rows = []
    for line in grid:
        parts, cur, buf = [], None, []

        def flush() -> None:
            if not buf:
                return
            st = dict(kv.split("=", 1) if "=" in kv else (kv, True) for kv in cur.split(";") if kv) if cur else {}
            fg, bg = st.get("fg"), st.get("bg")
            if st.get("inverse"):
                fg, bg = bg or "#1e1e1e", fg or "#d4d4d4"
            css = [f"color:{fg}"] if fg else []
            css += [f"background:{bg}"] if bg else []
            css += ["font-weight:bold"] if st.get("bold") else []
            css += ["opacity:.6"] if st.get("dim") else []
            css += ["font-style:italic"] if st.get("italic") else []
            css += ["text-decoration:underline"] if st.get("underline") else []
            css += ["text-decoration:line-through"] if st.get("strike") else []
            text = html.escape("".join(buf))
            parts.append(f'<span style="{";".join(css)}">{text}</span>' if css else text)
            buf.clear()

        for ch, key in line:
            if key != cur:
                flush()
                cur = key
            buf.append(ch)
        flush()
        rows.append("".join(parts))
    return "\n".join(rows)


PAGE = """<!doctype html><html><head><meta charset=utf-8><title>{title}</title>
<style>
body{{background:#111;color:#ddd;font-family:system-ui,sans-serif;margin:16px}}
.status{{font-size:16px;color:#9cf;margin-bottom:8px}}
pre.term{{background:#1e1e1e;color:#d4d4d4;font:13px/1.25 'DejaVu Sans Mono',Menlo,monospace;padding:8px;margin:0;border:1px solid #333;display:inline-block}}
</style></head><body><div class=status>{title}</div><pre class=term>{body}</pre></body></html>
"""


class PngRenderer:
    """Renders screens with render_png.mjs. Checks node and playwright once."""

    def __init__(self) -> None:
        self.disabled_reason: str | None = None
        self.env = dict(os.environ)
        self.tmp = Path(tempfile.mkdtemp(prefix="review-png-"))
        if shutil.which("node") is None:
            self.disabled_reason = "node is not on PATH"
            return
        if "NODE_PATH" not in self.env and shutil.which("npm"):
            root = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip()
            if root:
                self.env["NODE_PATH"] = root

    def render(self, after: str, png: Path, title: str) -> str | None:
        """Write the PNG. Returns None on success, else the reason it was skipped."""
        if self.disabled_reason:
            return self.disabled_reason
        html_path = self.tmp / "screen.html"
        html_path.write_text(PAGE.format(title=html.escape(title), body=grid_to_html(parse_styled(after))))
        res = subprocess.run(["node", str(RENDER), str(html_path), str(png)], env=self.env,
                             capture_output=True, text=True, timeout=180)
        if res.returncode != 0 or not png.exists():
            if "Executable doesn't exist" in res.stderr:
                # No browser to launch: skip every later PNG too.
                self.disabled_reason = "no Chromium for playwright (set PLAYWRIGHT_CHROMIUM to a browser path)"
                return self.disabled_reason
            errors = [line.strip() for line in res.stderr.splitlines() if "Error" in line]
            return (errors[0] if errors else f"render_png.mjs exited {res.returncode}")[:300]
        return None

    def close(self) -> None:
        shutil.rmtree(self.tmp, ignore_errors=True)


# ---------------------------------------------------------------------------
# Bundles and index
# ---------------------------------------------------------------------------


def run_output_snapshots(scenario_dir: Path) -> dict[str, str]:
    snaps = {}
    for p in sorted(scenario_dir.glob("*.txt")):
        if p.name.endswith(".plain.txt") or p.name in NOT_SNAPSHOTS:
            continue
        snaps[p.stem] = p.read_text()
    return snaps


def write_bundle(scenario: str, snap: str, before: str | None, after: str, png: PngRenderer | None,
                 want_png: bool) -> dict:
    d = REVIEW / scenario / snap
    d.mkdir(parents=True)
    files = []
    if before is not None:
        (d / "before.txt").write_text(before)
        files.append("before.txt")
    (d / "after.txt").write_text(after)
    files.append("after.txt")

    old_label = f"golden/{scenario}/{snap}.txt" if before is not None else "(no golden)"
    diff = "".join(difflib.unified_diff(
        (before or "").splitlines(keepends=True), after.splitlines(keepends=True),
        old_label, f"run/{scenario}/{snap}.txt"))
    (d / "diff.txt").write_text(diff)
    files.append("diff.txt")
    lines = diff.splitlines()
    plus = sum(1 for x in lines if x.startswith("+") and not x.startswith("+++"))
    minus = sum(1 for x in lines if x.startswith("-") and not x.startswith("---"))

    (d / "legend.txt").write_text(legend_text(after))
    files.append("legend.txt")

    png_note = ""
    if want_png and png is not None:
        reason = png.render(after, d / "after.png", f"{scenario} / {snap}")
        if reason is None:
            files.append("after.png")
        else:
            png_note = reason
            print(f"  warning: no PNG for {scenario}/{snap}: {reason}")
    return {"scenario": scenario, "snap": snap, "status": "changed" if before is not None else "new",
            "plus": plus, "minus": minus, "files": files, "png_note": png_note}


def write_index(entries: list[dict], missing: dict[str, list[str]], scenarios: list[str]) -> None:
    changed = sum(1 for e in entries if e["status"] == "changed")
    new = len(entries) - changed
    out = ["# TUI review bundles", "",
           f"{len(entries)} bundle(s): {changed} changed, {new} new. Scenarios read: "
           f"{', '.join(scenarios) or 'none'}.", "",
           "Regenerate: `python3 scripts/tui/goldens.py run <scenario|all>` then "
           "`python3 scripts/tui/review_bundle.py <scenario|all> [--png]`.", ""]
    if entries:
        out += ["| Scenario | Snapshot | Status | +/- lines | Bundle | Files |",
                "|---|---|---|---|---|---|"]
        for e in entries:
            base = f"{e['scenario']}/{e['snap']}"
            links = " · ".join(f"[{f.split('.')[0]}]({base}/{f})" for f in e["files"])
            out.append(f"| {e['scenario']} | {e['snap']} | {e['status']} | +{e['plus']} / -{e['minus']} | "
                       f"`{base}/` | {links} |")
        out.append("")
    else:
        out += ["No changed or new screens.", ""]
    if missing:
        out += ["## Golden snapshots missing from the run", ""]
        out += [f"- {scen}: {', '.join(snaps)}" for scen, snaps in missing.items()]
        out.append("")
    (REVIEW / "index.md").write_text("\n".join(out))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("scenario", nargs="?", default="all", help="a scenario name, or 'all' (default)")
    ap.add_argument("--png", action="store_true", help="also render after.png (needs node + playwright)")
    ap.add_argument("--snap", metavar="NAME", help="only this snapshot name")
    args = ap.parse_args()

    if not G.OUT.is_dir():
        print(f"no run output in {G.OUT}; run: python3 scripts/tui/goldens.py run {args.scenario}",
              file=sys.stderr)
        return 2
    if args.scenario == "all":
        scenarios = sorted(p.name for p in G.OUT.iterdir() if p.is_dir())
    else:
        scenarios = [args.scenario]
    for scen in scenarios:
        if not (G.OUT / scen).is_dir():
            print(f"no run output for {scen} in {G.OUT / scen}; run: python3 scripts/tui/goldens.py run {scen}",
                  file=sys.stderr)
            return 2

    if REVIEW.exists():
        shutil.rmtree(REVIEW)
    REVIEW.mkdir(parents=True)

    renderer = PngRenderer() if args.png else None
    entries: list[dict] = []
    missing: dict[str, list[str]] = {}
    seen_snap = False
    try:
        for scen in scenarios:
            run_dir = G.OUT / scen
            if (run_dir / "error.txt").exists():
                print(f"warning: {scen} failed to run; bundling the snapshots it reached "
                      f"(see {run_dir / 'error.txt'})")
            golden = G.golden_files(scen)
            after_snaps = run_output_snapshots(run_dir)
            for snap, after in after_snaps.items():
                if args.snap and snap != args.snap:
                    continue
                seen_snap = True
                before = golden.get(snap)
                if before == after:
                    continue
                entries.append(write_bundle(scen, snap, before, after, renderer, args.png))
            if not args.snap and not (run_dir / "error.txt").exists():
                gone = sorted(set(golden) - set(after_snaps))
                if gone:
                    missing[scen] = gone
    finally:
        if renderer:
            renderer.close()

    if args.snap and not seen_snap:
        print(f"no snapshot named {args.snap!r} in the run output for {', '.join(scenarios)}", file=sys.stderr)
    write_index(entries, missing, scenarios)
    changed = sum(1 for e in entries if e["status"] == "changed")
    print(f"wrote {len(entries)} bundle(s) ({changed} changed, {len(entries) - changed} new) to {REVIEW}")
    print(f"index: {REVIEW / 'index.md'}")
    for scen, snaps in missing.items():
        print(f"note: golden snapshots not in the run for {scen}: {', '.join(snaps)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
