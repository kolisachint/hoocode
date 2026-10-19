# hoocode

HooCode, the terminal coding agent ([kolisachint/hoocode](https://github.com/kolisachint/hoocode)).

This package is a thin launcher: `pip install hoocode` installs a `hoocode`
command that, on first run, downloads the prebuilt binary matching this
version and your platform from the
[GitHub release](https://github.com/kolisachint/hoocode/releases), verifies
its sha256 against the release `SHA256SUMS`, and runs it. Later runs reuse
the cached binary, so `hoocode` also works offline once fetched.

Supported platforms (same as the release archives):

- Linux on x64 and arm64
- macOS on x64 (Intel) and arm64 (Apple silicon)

The binary is cached under `~/.cache/hoocode/<version>/<target>/`
(`$XDG_CACHE_HOME` respected). To force a fresh download:

```sh
hoocode --hoocode-update --version
```

or set `HOOCODE_UPDATE=1`. To put the cache elsewhere, set
`HOOCODE_CACHE_DIR`.
