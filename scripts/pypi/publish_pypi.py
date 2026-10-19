#!/usr/bin/env python3
"""Publish the PyPI wheel+sdist and prove the version reached the index.

    python3 scripts/pypi/publish_pypi.py <dist-dir>
    python3 scripts/pypi/publish_pypi.py <dist-dir> --verify-only

<dist-dir> is what build_pypi.py wrote: hoocode-<version>-py3-none-any.whl
plus hoocode-<version>.tar.gz, uploaded with twine (TWINE_USERNAME /
TWINE_PASSWORD from the environment).

A version already on PyPI is skipped, so a re-run only does the missing
work. After uploading, the version is checked against the PyPI JSON API —
the same document `pip install hoocode==<version>` resolves through — and
the script exits non-zero if it never turns up. That check exists because a
successful upload does not mean the version is installable yet: the index
and its CDN lag behind the upload, so without it a partial publish looks
identical to a successful one.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

INDEX = "https://pypi.org/pypi/hoocode/json"
# The index sits behind a CDN; a unique param forces the origin.
VERIFY_TIMEOUT_S = 900
VERIFY_INTERVAL_S = 15


def dist_version(dist: Path) -> str:
    """The release version, read off the wheel filename build_pypi.py wrote."""
    wheels = sorted(dist.glob("hoocode-*-py3-none-any.whl"))
    if not wheels:
        sys.exit(f"no wheel in {dist}")
    match = re.fullmatch(r"hoocode-(.+)-py3-none-any\.whl", wheels[0].name)
    assert match is not None
    return match.group(1)


def on_pypi(version: str) -> bool:
    """True once `pip install hoocode==version` could resolve it."""
    url = f"{INDEX}?cb={time.time_ns()}"
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            data = json.load(response)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return False
        raise
    return version in data.get("releases", {})


def publish(dist: Path, version: str) -> None:
    files = sorted(p for p in dist.iterdir() if p.is_file())
    if not files:
        sys.exit(f"no files in {dist}")
    result = subprocess.run(
        [sys.executable, "-m", "twine", "upload", "--non-interactive", *(str(p) for p in files)],
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        print(f"published hoocode {version}")
        return
    # A concurrent publisher winning the race is success, not failure.
    if on_pypi(version):
        print(f"hoocode {version} was already published")
        return
    print(result.stdout[-2000:], result.stderr[-2000:], sep="\n", file=sys.stderr)
    sys.exit(f"twine upload failed for hoocode {version}")


def verify(version: str) -> bool:
    deadline = time.monotonic() + VERIFY_TIMEOUT_S
    while True:
        if on_pypi(version):
            print(f"hoocode {version} is on the index")
            return True
        if time.monotonic() >= deadline:
            sys.exit(
                f"hoocode {version} not on the index after {VERIFY_TIMEOUT_S}s: "
                "the upload reported success, so this is either index-side lag "
                "that has not cleared or the release was rolled back. Re-run "
                "this workflow once `pip index versions hoocode` lists it."
            )
        remaining = int(deadline - time.monotonic())
        print(f"waiting for hoocode {version} ({remaining}s left)")
        time.sleep(VERIFY_INTERVAL_S)


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    verify_only = "--verify-only" in sys.argv[1:]
    if len(args) != 1:
        sys.exit(__doc__)
    dist = Path(args[0])
    version = dist_version(dist)
    if verify_only:
        print(f"checking hoocode {version} (no upload)")
    elif on_pypi(version):
        print(f"skip hoocode {version} (already on the index)")
    else:
        publish(dist, version)
    verify(version)
    return 0


if __name__ == "__main__":
    sys.exit(main())
