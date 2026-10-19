"""First-run downloader for the real hoocode binary. Stdlib only.

`pip install hoocode` ships just this module. On first run it fetches the
release archive matching this package's version and the current platform,
checks its sha256 against the release SHA256SUMS, extracts the binary into
a user cache dir, and execs it. Later runs reuse the cached binary.

Refresh the cache with `hoocode --hoocode-update ...` or `HOOCODE_UPDATE=1`.
Override the cache location with `HOOCODE_CACHE_DIR`.
"""

from __future__ import annotations

import hashlib
import io
import os
import platform
import sys
import tarfile
import urllib.request
from importlib import metadata

REPO = "kolisachint/hoocode"

# (sys.platform, normalized machine) -> Rust target of the release archive.
# Must match the binaries.yml matrix and scripts/npm/build_packages.py.
PLATFORMS = {
    ("linux", "x86_64"): "x86_64-unknown-linux-musl",
    ("linux", "aarch64"): "aarch64-unknown-linux-musl",
    ("darwin", "x86_64"): "x86_64-apple-darwin",
    ("darwin", "arm64"): "aarch64-apple-darwin",
}

NETWORK_TIMEOUT_S = 60


def package_version() -> str:
    """The version of the installed `hoocode` dist: the release to download."""
    try:
        return metadata.version("hoocode")
    except metadata.PackageNotFoundError:
        sys.exit("hoocode: package metadata not found; reinstall with `pip install hoocode`.")


def current_target() -> str:
    machine = platform.machine().lower()
    if machine == "amd64":
        machine = "x86_64"
    if sys.platform == "darwin" and machine == "aarch64":
        machine = "arm64"
    if sys.platform == "linux" and machine == "arm64":
        machine = "aarch64"
    target = PLATFORMS.get((sys.platform, machine))
    if target is None:
        sys.exit(
            f"hoocode: no prebuilt binary for {sys.platform}-{platform.machine()}.\n"
            "Supported: macOS and Linux on x64 and arm64."
        )
    return target


def cached_binary(version: str, target: str) -> str:
    base = os.environ.get("HOOCODE_CACHE_DIR")
    if not base:
        cache_home = os.environ.get("XDG_CACHE_HOME") or os.path.join(
            os.path.expanduser("~"), ".cache"
        )
        base = os.path.join(cache_home, "hoocode")
    return os.path.join(base, version, target, "hoocode")


def download(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "hoocode-pypi-launcher"})
    try:
        with urllib.request.urlopen(request, timeout=NETWORK_TIMEOUT_S) as response:
            return response.read()
    except Exception as error:
        sys.exit(f"hoocode: download failed ({url}): {error}")


def ensure_binary(version: str, target: str, force: bool) -> str:
    dest = cached_binary(version, target)
    if not force and os.path.isfile(dest) and os.access(dest, os.X_OK):
        return dest
    tag = f"v{version}"
    base_url = f"https://github.com/{REPO}/releases/download/{tag}"
    archive = f"hoocode-{target}.tar.gz"
    data = download(f"{base_url}/{archive}")
    sums = download(f"{base_url}/SHA256SUMS").decode("utf-8", "replace")
    want = None
    for line in sums.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[-1].lstrip("*").endswith(archive):
            want = parts[0]
            break
    if want is None:
        sys.exit(f"hoocode: {archive} is not listed in the {tag} SHA256SUMS.")
    got = hashlib.sha256(data).hexdigest()
    if got != want:
        sys.exit(f"hoocode: sha256 mismatch for {archive}: expected {want}, got {got}.")
    try:
        with tarfile.open(fileobj=io.BytesIO(data)) as tar:
            member = tar.getmember(f"hoocode-{target}/hoocode")
            src = tar.extractfile(member)
            assert src is not None
            blob = src.read()
    except (KeyError, tarfile.TarError, AssertionError, EOFError) as error:
        sys.exit(f"hoocode: bad archive {archive}: {error}")
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    tmp = dest + ".tmp"
    with open(tmp, "wb") as handle:
        handle.write(blob)
    os.chmod(tmp, 0o755)
    os.replace(tmp, dest)
    return dest


def main(argv: list | None = None) -> None:
    args = sys.argv[1:] if argv is None else argv
    force = False
    if "--hoocode-update" in args:
        force = True
        args = [a for a in args if a != "--hoocode-update"]
    if os.environ.get("HOOCODE_UPDATE"):
        force = True
    binary = ensure_binary(package_version(), current_target(), force)
    try:
        os.execv(binary, [binary, *args])
    except OSError as error:
        sys.exit(f"hoocode: could not start {binary}: {error}")
