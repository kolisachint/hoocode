# User docs

The pages of the hoocode documentation at
<https://kolisachint.github.io/hoocode>. They are written here, next to the
code, so a change that alters what users see updates its page in the same pull
request.

The site copies this folder from the **latest release tag**, not from `main`,
so the published pages always describe a version people can install. Anything
merged after the last release reaches the site with the next one.

- `install.md`, `quickstart.md`, `configuration.md`, `commands.md`
- Short and plain. Describe only what this build does; check flags against
  `hoocode --help` (`crates/hoocode-code-cli/src/help_text.rs`).
- Link between pages as `commands.md`; link repository files relatively
  (`../../install/install.sh`). The site rewrites both.
- The sidebar order and page titles live in the site repository
  (`src/data/hoocode-nav.json`). A new page here shows up on the site but not
  in the sidebar until it is added there.
- This README is not published.
