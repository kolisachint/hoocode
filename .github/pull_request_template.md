## What does this change?

<!-- The diff shows what changed. Say what problem it solves. -->

## Why?

<!-- Link an issue if there is one ("Fixes #123"). If not, a sentence is fine. -->

## How did you check it?

<!-- Tests you added, or what you ran by hand. -->

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo nextest run -p <crates I touched>`
- [ ] A test for the bug, if this fixes one
- [ ] `docs/maps/` updated, if this adds, renames or removes a crate, screen part, picker or slash command
- [ ] A `CHANGELOG.md` entry under `[Unreleased]`, if users will notice the change

<!--
First PR here? Welcome. None of these are a problem if you get them wrong; a reviewer
will say so:

  - One change per PR reviews much faster than two.
  - Don't bump versions; the release workflow does that.
  - Don't add release labels (`rust:*`, `release:*`). Maintainers do that.
  - Only copy code from MIT-licensed sources.
  - Integration tests go in crates/<crate>/tests/it/<name>.rs.

Full guide: https://github.com/kolisachint/hoocode/blob/main/CONTRIBUTING.md
-->
