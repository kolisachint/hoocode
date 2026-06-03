---
name: ship
description: |
  Ship a release: commit current changes, then run the release script.
  Usage: /ship <patch|minor|major>
argument-hint: patch|minor|major
---
Ship a release with version bump type: **$1**

Run these steps in order:

1. **Stage and commit current changes:**
   ```bash
   git add .
   git commit -m "ship: $1"
   ```

2. **Run the release script** (bumps version, updates changelogs, tags, publishes, pushes):
   ```bash
   node scripts/release.mjs $1
   ```

If any step fails, stop and report the error.
