#!/usr/bin/env python3
"""Regenerate the "All crates" tables in docs/maps/packages.md.

Usage:
    python3 scripts/maps/packages.py           # rewrite the generated part
    python3 scripts/maps/packages.py --check   # exit 1 with a diff if the page is stale

Only the lines between the BEGIN and END generated markers are written. The
hand-written prose around them is never touched.

Columns:
    Crate     Package name without the workspace prefix (the prefix shared by the
              most workspace packages, so it works before and after a rename).
    Does      Cargo.toml `description`.
    Used by   Workspace packages with a normal (not dev or build) dependency on it.
    src/tests Lines in .rs files under src/ and tests/.
    Status    Hand-curated. Kept from the existing table, keyed by crate name. A
              crate with no row gets "keep" and a note on stderr.

Needs only Python 3 and cargo. Keep this file free of the old app name so it works
before and after the rename.
"""

import argparse
import collections
import difflib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PAGE = ROOT / "docs" / "maps" / "packages.md"
BEGIN = "<!-- BEGIN generated: packages -->"
END = "<!-- END generated -->"
HEADER = "| Crate | Does | Used by | src / tests lines | Status |"
RULE = "|---|---|---|---|---|"
DEFAULT_STATUS = "keep"

# (heading, predicate on the display name). Order matters: first match wins.
GROUPS = [
    ("Runtime (`runtime`)", lambda n: n == "runtime"),
    ("Umbrella (all to delete)", lambda n: "-" not in n),
    ("AI: models, providers, logins (`ai-*`)", lambda n: n.startswith("ai-")),
    ("Agent runtime (`agent-*`)", lambda n: n.startswith("agent-")),
    (
        "Coding agent (`code-*`, `app-server*`)",
        lambda n: (n.startswith("code-") and not n.startswith("code-tui-"))
        or n.startswith("app-server"),
    ),
    ("Coding agent UI (`code-tui-*`)", lambda n: n.startswith("code-tui-")),
    ("TUI library (`tui-*`)", lambda n: n.startswith("tui-")),
    ("Other", lambda n: True),
]

ROW_RE = re.compile(r"^\| `([^`]+)` \| (.*) \| (\d+) / (\d+) \| (.*) \|$")


def cargo_metadata():
    proc = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        sys.exit(2)
    return json.loads(proc.stdout)


def workspace_prefix(names):
    """The first hyphen-separated part shared by most package names."""
    firsts = collections.Counter(n.split("-", 1)[0] for n in names)
    prefix, count = firsts.most_common(1)[0]
    if count != len(names):
        print(
            f"note: {len(names) - count} package(s) do not share prefix '{prefix}'",
            file=sys.stderr,
        )
    return prefix


def display_name(name, prefix):
    if name.startswith(prefix + "-"):
        return name[len(prefix) + 1 :]
    return name


def count_rs_lines(pkg_dir, sub):
    total = 0
    base = pkg_dir / sub
    if base.is_dir():
        for path in base.rglob("*.rs"):
            total += path.read_bytes().count(b"\n")
    return total


def existing_status(lines):
    """Map crate display name -> status cell, read from the generated region."""
    status = {}
    for line in lines:
        m = ROW_RE.match(line)
        if m:
            status[m.group(1)] = m.group(5)
    return status


def cell(text):
    return " ".join(text.split()).replace("|", "\\|")


def build_rows(meta, status_by_name):
    packages = meta["packages"]
    names = [p["name"] for p in packages]
    prefix = workspace_prefix(names)
    workspace = set(names)

    used_by = collections.Counter()
    for p in packages:
        deps = {d["name"] for d in p["dependencies"] if d["kind"] is None}
        for dep in deps & workspace:
            if dep != p["name"]:
                used_by[dep] += 1

    rows = {}
    for p in packages:
        disp = display_name(p["name"], prefix)
        pkg_dir = Path(p["manifest_path"]).parent
        status = status_by_name.get(disp)
        if status is None:
            status = DEFAULT_STATUS
            print(f"note: no status for '{disp}'; using '{DEFAULT_STATUS}'", file=sys.stderr)
        rows[disp] = (
            f"| `{disp}` | {cell(p.get('description') or '')} | {used_by[p['name']]} | "
            f"{count_rs_lines(pkg_dir, 'src')} / {count_rs_lines(pkg_dir, 'tests')} | "
            f"{status} |"
        )

    for gone in sorted(set(status_by_name) - set(rows)):
        print(f"note: dropped '{gone}' (not in the workspace)", file=sys.stderr)
    return rows, prefix


def render_body(rows, prefix):
    blocks = []
    placed = set()
    # Within a group: the root crate (named just the prefix) first, then by name.
    order = lambda n: (n != prefix, n)
    for heading, pred in GROUPS:
        names = sorted((n for n in rows if n not in placed and pred(n)), key=order)
        placed.update(names)
        if not names:
            continue
        table = [f"### {heading}", "", HEADER, RULE] + [rows[n] for n in names]
        blocks.append("\n".join(table))
    return "\n\n".join(blocks).split("\n")


def generate(text, meta):
    lines = text.split("\n")
    try:
        b = lines.index(BEGIN)
        e = lines.index(END, b + 1)
    except ValueError:
        print(f"error: {PAGE} needs '{BEGIN}' and '{END}' markers", file=sys.stderr)
        sys.exit(2)
    status = existing_status(lines[b + 1 : e])
    rows, prefix = build_rows(meta, status)
    region = render_body(rows, prefix) + [""]
    return "\n".join(lines[: b + 1] + region + lines[e:])


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="do not write; exit 1 with a diff if the page is stale",
    )
    args = parser.parse_args()

    current = PAGE.read_text(encoding="utf-8")
    updated = generate(current, cargo_metadata())

    if args.check:
        if updated == current:
            return 0
        diff = difflib.unified_diff(
            current.splitlines(keepends=True),
            updated.splitlines(keepends=True),
            fromfile=f"{PAGE} (committed)",
            tofile=f"{PAGE} (generated)",
        )
        sys.stdout.writelines(diff)
        print(
            "error: docs/maps/packages.md is stale; run: python3 scripts/maps/packages.py",
            file=sys.stderr,
        )
        return 1

    if updated != current:
        PAGE.write_text(updated, encoding="utf-8")
        print(f"wrote {PAGE.relative_to(ROOT)}")
    else:
        print(f"{PAGE.relative_to(ROOT)} already up to date")
    return 0


if __name__ == "__main__":
    sys.exit(main())
