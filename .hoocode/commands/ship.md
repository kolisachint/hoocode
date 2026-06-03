---
name: ship
description: |
  Ship a release: commit, push, bump version, and publish to npm.
  Usage: /ship <patch|minor|major>
argument-hint: patch|minor|major
---
Ship a release with version bump type: **$1**

Run these steps in order:

1. **Stage and commit changes:**
   ```bash
   git add .
   git commit -m "ship: $1"
   ```

2. **Push to origin:**
   ```bash
   git push origin $(git branch --show-current)
   ```

3. **Bump version and publish:**
   ```bash
   npm run version:$1
   npm run publish
   ```

If any step fails, stop and report the error. Do not proceed if there are uncommitted changes before starting.
