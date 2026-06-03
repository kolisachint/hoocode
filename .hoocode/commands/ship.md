---
name: ship
description: |
  Ship a release: run the release script to bump version, publish, tag, and push.
  Usage: /ship <patch|minor|major>
  Ensure all changes are committed before running.
argument-hint: patch|minor|major
---
Ship a release with version bump type: **$1**

Run the release script (bumps version, updates changelogs, commits, tags, publishes, pushes):

```bash
node scripts/release.mjs $1
```

If there are uncommitted changes, commit them first with a descriptive message before running this command.
