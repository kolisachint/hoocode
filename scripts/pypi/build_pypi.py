#!/usr/bin/env python3
"""Build the PyPI wheel+sdist for a release.

    python3 scripts/pypi/build_pypi.py <tag> [out-dir]

<tag> is the release tag (e.g. v0.1.34); the package version is the tag
without the leading `v`. The wheel is thin: it holds only the launcher in
pypi/, which downloads the matching release archive on first run. Writes
hoocode-<version>-py3-none-any.whl and hoocode-<version>.tar.gz into
[out-dir] (default pypi-dist/). Needs the `build` package
(`pip install build`).
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "pypi"
VERSION_MARKER = 'version = "0.0.0"'


def build(version: str, out: Path) -> None:
    # `--outdir` is interpreted relative to the build backend's cwd (the
    # staging dir below), so resolve it first: a relative out-dir would
    # otherwise land inside the staging dir and vanish with it.
    out = out.resolve()
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    try:
        import build  # noqa: F401
    except ImportError:
        sys.exit("the `build` package is missing: pip install build")
    with tempfile.TemporaryDirectory() as tmp:
        stage = Path(tmp) / "pkg"
        shutil.copytree(SRC, stage)
        shutil.copy(ROOT / "LICENSE", stage / "LICENSE")
        shutil.copy(ROOT / "NOTICE", stage / "NOTICE")
        text = (stage / "pyproject.toml").read_text()
        if VERSION_MARKER not in text:
            sys.exit(f"{SRC}/pyproject.toml has no {VERSION_MARKER!r} to stamp")
        (stage / "pyproject.toml").write_text(
            text.replace(VERSION_MARKER, f'version = "{version}"')
        )
        result = subprocess.run(
            [sys.executable, "-m", "build", "--outdir", str(out)],
            cwd=stage,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(result.stdout[-2000:], result.stderr[-2000:], sep="\n", file=sys.stderr)
            sys.exit(f"python -m build failed for {version}")
    artifacts = sorted(out.iterdir())
    if not artifacts:
        sys.exit(f"no artifacts in {out}")
    print(f"built {version}: " + ", ".join(p.name for p in artifacts))


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3) or sys.argv[1] in ("-h", "--help"):
        sys.exit(__doc__)
    tag_version = sys.argv[1].removeprefix("v")
    if not re.fullmatch(r"\d+\.\d+\.\d+", tag_version):
        sys.exit(f"bad tag {sys.argv[1]!r}: want vX.Y.Z (e.g. v0.1.34)")
    build(tag_version, Path(sys.argv[2]) if len(sys.argv) == 3 else ROOT / "pypi-dist")
