---
name: postmerge
description: |
  After a PR is merged: verify CI, the npm publish, the version bump, the tag
  and the GitHub release, then move this worktree back to an up-to-date main.
  Usage: /postmerge [pr-number]
argument-hint: pr-number (optional)
---
A PR was just merged (PR: **$1**, discover it if empty). Verify the release
pipeline finished cleanly, then return to `main`.

Read-only checks first. Do not push, tag, or run `release.mjs` — the merge
triggers `.github/workflows/merge-release.yml`, which bumps versions, publishes
to npm, tags, pushes, creates the GitHub release, and uploads binaries.

1. Identify the merged PR and whether it carried a release label:
   ```bash
   gh pr view $1 --json number,title,mergedAt,mergeCommit,labels
   ```
   If `$1` is empty, use `gh pr list --state merged --limit 5` and pick the most
   recent one from this session's work. If it has no `npm:patch|minor|major`
   label, nothing publishes: skip steps 3-5 and report that.

2. Check CI and the release workflows on `main`:
   ```bash
   gh run list --branch main --limit 10
   ```
   Any run still in progress: wait and re-check rather than declaring success.
   Any failed run: fetch `gh run view <id> --log-failed`, report the failure,
   and stop.

3. Confirm the version bump landed on `origin/main`:
   ```bash
   git fetch origin main --tags
   git show origin/main:packages/coding-agent/package.json | node -p "JSON.parse(require('fs').readFileSync(0,'utf8')).version"
   ```
   All packages are versioned in lockstep; spot-check one more if unsure.

4. Confirm npm has that exact version for every published package:
   ```bash
   for p in @kolisachint/hoocode-agent @kolisachint/hoocode-ai @kolisachint/hoocode-agent-core @kolisachint/hoocode-tui; do
     echo "$p $(npm view "$p" version)"
   done
   ```
   Every package must match the version from step 3.

5. Confirm the tag and GitHub release exist for that version:
   ```bash
   git tag --list "v<version>"
   gh release view "v<version>" --json tagName,assets
   ```
   The Windows binary `hoocode-windows-x64.zip` should be attached.

6. Only when steps 2-5 are all green, return to main:
   ```bash
   git status
   git switch main
   git pull --rebase
   ```
   `git status` must be clean of YOUR files before switching. Other agents may
   have uncommitted work here — never run `git reset --hard`, `git checkout .`,
   `git clean -fd`, or `git stash` to make the switch possible. If your own work
   is uncommitted, stop and ask.

7. Optionally delete the merged feature branch locally and on the remote once
   `main` contains it:
   ```bash
   git branch -d <branch> && git push origin --delete <branch>
   ```

Report as a short checklist: PR + label, CI status, version on main, npm
versions, tag/release, current branch. Flag anything that did not match.
