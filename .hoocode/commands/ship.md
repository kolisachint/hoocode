---
name: ship
description: |
  Ship a release: stage changes, bump version, publish, tag, and push.
  Usage: /ship <patch|minor|major>
argument-hint: patch|minor|major
---
Ship a release with version bump type: **$1**

Run the release script (stages changes, bumps version, updates changelogs, commits, tags, publishes, pushes):

```bash
node scripts/release.mjs $1
```

The release script handles everything including staging any uncommitted changes.
