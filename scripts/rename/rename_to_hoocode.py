#!/usr/bin/env python3
"""Rename cortexcode/cortex to hoocode (plan step 0c; archive/docs/naming-and-paths.md §1).

    python3 scripts/rename/rename_to_hoocode.py           # --dry-run (default): print the plan, write nothing
    python3 scripts/rename/rename_to_hoocode.py --apply   # git mv the paths, rewrite the text, git add it

Run it once, on a clean tree, after the 11 crate deletions have landed. It renames whatever
crates exist. Stdlib only; needs git. Nothing is committed: review `git diff --cached`, then commit.

Renamed: tracked paths (crates/cortexcode-* -> crates/hoocode-*, the migration doc ->
ts-to-rust-migration.md) and the text of every tracked file, by the RULES below in order.
CORTEX_BIN -> HOOCODE_BIN is applied first. CortexCode and Cortex (brand) -> HooCode.
cortexcode and cortex, as words or prefixes (cortex_*, cortex-*, [package.metadata.cortex],
the cargo bin), -> hoocode.

Not touched:
  * Cargo.lock (cargo regenerates it), binary and non-UTF-8 files, symlinks;
  * the naming card and the two decision pages (they must keep the old names);
  * this script;
  * what §2-4 owns, which stays as it is: any ".cortexcode" (config dirs), CORTEX*/CORTEXCODE*
    env names (except CORTEX_BIN), and cortex-debug.log;
  * pycortex (another project).

Idempotent: a second --apply finds nothing to change. Lines that a text rename gets wrong
(two quoted names collapsing into one, "X -> X") and files that need a hand edit are printed
under MANUAL; the script does not try to fix semantics.
"""

import argparse
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict

# Files whose content must keep the old names, and files that are never rewritten.
SKIP_CONTENT = {
    "Cargo.lock",
    "archive/docs/naming-and-paths.md",
    "archive/docs/decisions-2026-10-07.md",
    "archive/docs/decisions-2026-10-08.md",
    "scripts/rename/rename_to_hoocode.py",
}

# Applied first. Each is (label, regex, replacement).
BEFORE = [
    ("CORTEX_BIN -> HOOCODE_BIN (§1)",
     re.compile(r"(?<![A-Za-z0-9])CORTEX_BIN(?![A-Za-z0-9])"), "HOOCODE_BIN"),
]

# Held back from RULES: left unchanged, counted, reported as out of scope (§2-4).
HOLD = [
    ("§2-3 .cortexcode config dir",
     re.compile(r"\.cortexcode(?![A-Za-z0-9])")),
    ("§2 env var CORTEX* / CORTEXCODE*",
     re.compile(r"(?<![A-Za-z0-9])CORTEX(?:CODE)?_[A-Z0-9_]*")),
    ("§2 debug log name cortex-debug",
     re.compile(r"cortex-debug")),
]

# A word start: not after a letter or digit, or right after a regex \b (normalize.json).
WORD_START = r"(?:(?<![A-Za-z0-9])|(?<=\\b))"

# Applied in order to paths and text. Each is (label, regex, replacement).
RULES = [
    ("hoocode-to-cortexcode-migration -> ts-to-rust-migration",
     re.compile(r"hoocode-to-cortexcode-migration"), "ts-to-rust-migration"),
    ("CortexCode -> HooCode (brand)",
     re.compile(r"CortexCode"), "HooCode"),
    ("cortexcode -> hoocode (crates, Rust paths, URLs, names)",
     re.compile(r"cortexcode"), "hoocode"),
    ("Cortex -> HooCode (brand, APP_TITLE, regex \\bCortex)",
     re.compile(WORD_START + r"Cortex(?![A-Za-z0-9])"), "HooCode"),
    ("cortex -> hoocode (bin, APP_NAME, metadata, thread names, identifiers, regex \\bcortex)",
     re.compile(WORD_START + r"cortex(?![A-Za-z0-9])"), "hoocode"),
]

# Files that need more than a text rename. Printed when the file changes.
MANUAL = {
    "scripts/install.sh":
        "install stops renaming: the cargo bin is hoocode. The text rename turns "
        "`ln -s hoocode \"$PREFIX/cortex\"` into a link to itself; rewrite the install step "
        "and the --also-cortex option by hand.",
    "migration/tui-parity/harness.py":
        "APPS, CONFIG_DIRS and the diff labels expected ('hoocode', 'cortex'); both sides now "
        "say hoocode. Label them ts and rust (naming card §1) and check CONFIG_DIRS.",
    "crates/cortexcode-code-resources/src/package_discovery.rs":
        "a list of names (cortexcode, hoocode, pi) now holds hoocode twice; drop the duplicate.",
    "archive/migration/tools/gen_help_text.py":
        "the branding map (hoocode -> cortex, HooCode -> Cortex) is now a no-op; delete it or "
        "point it at the Rust names.",
    "migration/tui-parity/normalize.json":
        "branding normalizers for HooCode/Cortex may now be no-ops; review.",
    ".github/workflows/binaries.yml":
        "builds and copies the hoocode bin; check nothing renames at install.",
    "scripts/shims/hoocode-ts":
        "behaviour unchanged; only the comment names the bin differently.",
    "CLAUDE.md":
        "the line \"Until the rename lands, code still says cortexcode\" is stale; delete it.",
    "LICENSE":
        "the project name in the licence text changes; confirm that is wanted.",
}

FLAG_HOO_ARROW = re.compile(r"(?i)hoocode\s*(?:→|->)\s*hoocode")
FLAG_QUOTED_HOO = re.compile(r"""(["'])hoocode\1""")
FLAG_QUOTED_CORTEX = re.compile(r"""(["'])cortex(?:code)?\1""")


def transform(text):
    """Return (new_text, rule_counts, hold_counts) for one string."""
    rule_counts = Counter()
    hold_counts = Counter()
    for label, rx, rep in BEFORE:
        text, n = rx.subn(rep, text)
        if n:
            rule_counts[label] += n
    holds = []
    for label, rx in HOLD:
        def hide(m, label=label):
            hold_counts[label] += 1
            holds.append(m.group(0))
            return "\x00%d\x00" % (len(holds) - 1)
        text = rx.sub(hide, text)
    for label, rx, rep in RULES:
        text, n = rx.subn(rep, text)
        if n:
            rule_counts[label] += n
    text = re.sub(r"\x00(\d+)\x00", lambda m: holds[int(m.group(1))], text)
    return text, rule_counts, hold_counts


def line_flags(old_text, new_text):
    """Lines a text rename likely got wrong: 'X -> X', or a quoted name now twice."""
    flags = []
    old_lines = old_text.split("\n")
    new_lines = new_text.split("\n")
    for i, (o, n) in enumerate(zip(old_lines, new_lines), 1):
        if o == n:
            continue
        if FLAG_HOO_ARROW.search(n):
            flags.append((i, "reads 'X -> X' after the rename", n.strip()))
        elif FLAG_QUOTED_HOO.search(o) and FLAG_QUOTED_CORTEX.search(o):
            flags.append((i, "a line had both a quoted hoocode and a quoted cortex name; they now collide", n.strip()))
    return flags


def git(root, *args):
    return subprocess.run(["git", *args], cwd=root, check=True,
                          capture_output=True, text=True).stdout


def tracked_files(root):
    return [p for p in git(root, "ls-files", "-z").split("\0") if p]


def plan(root):
    """Compute path moves and content rewrites for the current tree (no writes)."""
    files = tracked_files(root)
    moves = []
    for p in files:
        new, _, _ = transform(p)
        if new != p:
            moves.append((p, new))

    rule_totals = Counter()
    rule_files = Counter()
    hold_totals = Counter()
    rewrites = []   # (path, new_text, flags, file_rule_counts)
    skipped = []
    binary = []
    for p in files:
        if p in SKIP_CONTENT:
            skipped.append(p)
            continue
        full = os.path.join(root, p)
        if os.path.islink(full) or not os.path.isfile(full):
            continue
        with open(full, "rb") as f:
            data = f.read()
        if b"\0" in data[:8192]:
            binary.append(p)
            continue
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            binary.append(p)
            continue
        new_text, rc, hc = transform(text)
        rule_totals.update(rc)
        hold_totals.update(hc)
        for label in rc:
            rule_files[label] += 1
        if new_text != text:
            rewrites.append((p, new_text, line_flags(text, new_text), rc))
    return {
        "files": files, "moves": moves, "rewrites": rewrites, "skipped": skipped,
        "binary": binary, "rule_totals": rule_totals, "rule_files": rule_files,
        "hold_totals": hold_totals,
    }


def crate_key(path):
    parts = path.split("/")
    return "/".join(parts[:2]) if parts[0] == "crates" else path


def print_plan(root, pl, verbose):
    moves = pl["moves"]
    print(f"repo: {root}")
    print(f"tracked files: {len(pl['files'])}; skipped by name: {len(pl['skipped'])}; "
          f"binary or non-UTF-8: {len(pl['binary'])}" + "".join(f"\n  binary: {p}" for p in pl["binary"]))

    groups = defaultdict(list)
    for old, new in moves:
        groups[crate_key(old)].append((old, new))
    print(f"\nPATH MOVES: {len(moves)} files under {len(groups)} top-level entries")
    for key in sorted(groups):
        items = groups[key]
        new_key = crate_key(items[0][1])
        print(f"  {key} -> {new_key}  ({len(items)} files)")
        if verbose:
            for old, new in items:
                print(f"      {old} -> {new}")

    print(f"\nTEXT REWRITES: {len(pl['rewrites'])} files changed")
    for label in sorted(pl["rule_totals"], key=lambda k: -pl["rule_totals"][k]):
        print(f"  {pl['rule_totals'][label]:6d} occurrences in {pl['rule_files'][label]:4d} files  {label}")

    print("\nHELD (§2-4, left as they are):")
    for label, _ in HOLD:
        print(f"  {pl['hold_totals'][label]:6d}  {label}")

    print("\nSKIPPED BY NAME (no content rewrite):")
    for p in pl["skipped"]:
        print(f"  {p}")

    manual = []
    for p, _, flags, _ in pl["rewrites"]:
        note = MANUAL.get(p, "")
        if note or flags:
            manual.append((p, note, flags))
    print(f"\nMANUAL ({len(manual)} files):")
    for p, note, flags in manual:
        if note:
            print(f"  {p}: {note}")
        for lineno, why, text in flags[:20]:
            print(f"  {p}:{lineno}: {why}: {text[:120]}")


def check_clean(root):
    out = git(root, "status", "--porcelain", "--untracked-files=no")
    return out.strip() == ""


def apply_moves(root, moves):
    tracked = set(tracked_files(root))
    targets = [new for _, new in moves]
    if len(set(targets)) != len(targets):
        sys.exit("refusing: two paths map to the same new name")
    for old, new in moves:
        if new in tracked or os.path.lexists(os.path.join(root, new)):
            sys.exit(f"refusing: target already exists: {new}")
    touched_dirs = set()
    for old, new in moves:
        os.makedirs(os.path.dirname(os.path.join(root, new)) or root, exist_ok=True)
        subprocess.run(["git", "mv", old, new], cwd=root, check=True)
        touched_dirs.add(os.path.dirname(os.path.join(root, old)))
    for d in sorted(touched_dirs, key=len, reverse=True):
        while d and os.path.abspath(d) != os.path.abspath(root):
            try:
                os.rmdir(d)
            except OSError:
                break
            d = os.path.dirname(d)


def apply_rewrites(root, rewrites):
    for p, new_text, _, _ in rewrites:
        with open(os.path.join(root, p), "w", encoding="utf-8", newline="") as f:
            f.write(new_text)


def stage(root, paths):
    for i in range(0, len(paths), 400):
        subprocess.run(["git", "add", "--", *paths[i:i + 400]], cwd=root, check=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", action="store_true", help="print the plan (default)")
    mode.add_argument("--apply", action="store_true", help="do the rename and stage it")
    ap.add_argument("--verbose", action="store_true", help="list every path move")
    args = ap.parse_args()

    root = git(os.getcwd(), "rev-parse", "--show-toplevel").strip()

    if args.apply:
        if not check_clean(root):
            sys.exit("refusing: tracked files have uncommitted changes; commit or set them aside first")
        pl = plan(root)
        print_plan(root, pl, args.verbose)
        apply_moves(root, pl["moves"])
        pl = plan(root)
        apply_rewrites(root, pl["rewrites"])
        stage(root, sorted(p for p, _, _, _ in pl["rewrites"]))
        print(f"\napplied: {len(pl['rewrites'])} files rewritten; staged. Review with git diff --cached.")
        after = plan(root)
        print(f"re-check: {len(after['moves'])} path moves and {len(after['rewrites'])} rewrites left "
              "(0 and 0 means the rename is complete and idempotent).")
    else:
        if not check_clean(root):
            print("note: tracked files have uncommitted changes; --apply will refuse.\n")
        print("mode: dry-run (nothing written)\n")
        print_plan(root, plan(root), args.verbose)
    return 0


if __name__ == "__main__":
    sys.exit(main())
