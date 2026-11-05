#!/usr/bin/env python3
"""Guard for K-9: UI strings do not name keys by hand.

Every key hint on screen comes from the keymap (`hoocode-code-tui-keybindings`,
`hints.rs`: `key_hint_text`, `key_display_text`, ...), so a rebound key shows the
new key. This lint reads the string literals of the TUI crates and fails on a
key name written into one of them, such as `"ctrl+x m"`, `"esc stop"` or
`"↑↓ navigate"`. Comments, `#[cfg(test)]` modules and `*_tests.rs` files are not
read. Legitimate uses (key tables, parsers, the terminal's own byte sequences)
are listed in `no_literal_keys.allow`.

Scans: crates/hoocode-code-tui-*/src/**/*.rs and crates/hoocode-tui-*/src/**/*.rs.
Exit 0 when clean, 1 with `file:line` for each new literal. No dependencies.

Run it by hand: python3 scripts/ci/no_literal_keys.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ALLOW_FILE = Path(__file__).resolve().parent / "no_literal_keys.allow"
GLOBS = ("crates/hoocode-code-tui-*/src/**/*.rs", "crates/hoocode-tui-*/src/**/*.rs")

MODIFIERS = r"(?:ctrl|alt|shift|super|cmd|option)"
# Where a hint token starts: the start of the text, after a separator (`·`, `|`,
# `(`, `[`), or after "press", "or" or "and". Prose such as "pressing Escape twice"
# or "terminal tab bar" does not start a hint, so it is not flagged.
HINT_START = r"(?:^|[·|(\[]\s*|\b(?:[Pp]ress|[Oo]r|[Aa]nd)\s+)"
NAMED = (
    r"(?:esc|escape|enter|return|tab|backspace|pgup|pgdn|pageup|pagedown|space|"
    r"Esc|Escape|Enter|Return|Tab|Backspace|PgUp|PgDn|PageUp|PageDown|Space)"
)
ARROW = r"(?:[↑↓←→]|\\u\{21(?:90|91|92|93)\})"
PATTERNS = [
    # A chord: ctrl+x, Ctrl+X, Shift+Tab, alt+enter.
    ("chord", re.compile(r"(?<![\w.])" + MODIFIERS + r"\+\S", re.IGNORECASE)),
    # A named key starting a hint: "esc stop", "Enter a value", "press esc to cancel".
    (
        "named key",
        re.compile(
            r"(?<![\w.\-])" + HINT_START + NAMED + r"(?=\s+[a-z]|\s*$|\s*/|/|\s*\)|\s*[·|])"
        ),
    ),
    # A function key: F1, f12.
    ("function key", re.compile(r"(?<![\w.])[Ff](?:[1-9]|1[0-2])(?![\w])")),
    # Arrow glyphs used as key hints: "↑↓ navigate", "←/→", "\u{2191}\u{2193} line".
    ("arrow key", re.compile(ARROW + r"\s*/?\s*" + ARROW)),
    # A bracketed key: "[Esc]", "(enter)".
    ("bracketed key", re.compile(r"[\[(](?:esc|enter|tab|ctrl|alt|shift)[\])]", re.IGNORECASE)),
    # A letter key answering a dialog: "y yes", "n no", "a always".
    (
        "letter key",
        re.compile(
            r"(?<![\w./])" + HINT_START + r"[yanedqs] (?:yes|no|always|edit|next|prev|close|"
            r"cancel|find|back|stop|open|pick|select|search|help)\b"
        ),
    ),
]

CFG_TEST_MOD = re.compile(r"#\[cfg\(test\)\]\s*(?:pub\s+)?mod\b")
CHAR_LIT = re.compile(r"'(?:\\.[^'\n]{0,8}|[^\\'\n])'")
RAW_START = re.compile(r"r(#*)\"")


def code_and_strings(src: str):
    """Yields ('str', line, text) for string literals outside comments and test modules."""
    i, n, line = 0, len(src), 1
    skip_depth = 0  # > 0 inside a #[cfg(test)] module
    pending_test = False
    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            i += 1
            continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("/*", i):
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif src.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    line += src[i] == "\n"
                    i += 1
            continue
        if c == "#" and skip_depth == 0 and src.startswith("#[cfg(test)]", i):
            if CFG_TEST_MOD.match(src, i):
                pending_test = True
        if c == "'":
            m = CHAR_LIT.match(src, i)
            if m:
                i = m.end()
                continue
        if c == "r" and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            m = RAW_START.match(src, i)
            if m:
                close = '"' + m.group(1)
                j = src.find(close, m.end())
                j = n if j < 0 else j
                text = src[m.end() : j]
                if skip_depth == 0:
                    yield ("str", line, text)
                line += text.count("\n")
                i = j + len(close)
                continue
        if c == '"':
            start, j, buf = line, i + 1, []
            while j < n and src[j] != '"':
                if src[j] == "\\":
                    buf.append(src[j : j + 2])
                    j += 2
                    continue
                line += src[j] == "\n"
                buf.append(src[j])
                j += 1
            if skip_depth == 0:
                yield ("str", start, "".join(buf))
            i = j + 1
            continue
        if c == "{":
            if skip_depth:
                skip_depth += 1
            elif pending_test:
                skip_depth, pending_test = 1, False
        elif c == "}" and skip_depth:
            skip_depth -= 1
        i += 1


def load_allow():
    """Entries: `path | needle | reason`. A needle of `*` allows the whole file."""
    entries = []
    if not ALLOW_FILE.exists():
        return entries
    for raw in ALLOW_FILE.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = [p.strip() for p in line.split("|")]
        if len(parts) != 3 or not parts[2]:
            sys.exit(f"{ALLOW_FILE}: each entry is `path | needle | reason`: {raw!r}")
        entries.append(tuple(parts))
    return entries


def allowed(rel: str, source_line: str, entries) -> bool:
    for path, needle, _ in entries:
        if path != rel:
            continue
        if needle == "*" or needle in source_line:
            return True
    return False


def scan_file(path: Path, entries):
    rel = path.relative_to(ROOT).as_posix()
    if path.name.endswith("_tests.rs"):
        return []
    src = path.read_text(encoding="utf-8")
    src_lines = src.splitlines()
    hits = []
    for _, line, text in code_and_strings(src):
        for name, pattern in PATTERNS:
            if pattern.search(text):
                source_line = src_lines[line - 1] if 0 < line <= len(src_lines) else ""
                if allowed(rel, source_line, entries):
                    break
                hits.append((rel, line, name, text.strip()))
                break
    return hits


def main() -> int:
    entries = load_allow()
    files = sorted({p for g in GLOBS for p in ROOT.glob(g)})
    hits = []
    for path in files:
        hits.extend(scan_file(path, entries))
    for rel, line, name, text in hits:
        shown = text if len(text) <= 90 else text[:87] + "..."
        print(f"{rel}:{line}: literal {name} in a UI string: {shown!r}")
    if hits:
        print(
            f"no_literal_keys: FAIL: {len(hits)} literal key(s). Build the hint from the keymap "
            "(hoocode-code-tui-keybindings hints.rs), or allow it in scripts/ci/no_literal_keys.allow",
            file=sys.stderr,
        )
        return 1
    print(f"no_literal_keys: OK ({len(files)} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
