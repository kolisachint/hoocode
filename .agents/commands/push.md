---
name: push
description: |
  Push your session's changes straight to origin/main: stage only your files,
  commit with an auto-generated message, rebase on origin/main, and push.
  No PR, no release label, no publish.
  Usage: /push
---
Push the changes YOU made in this session directly to `origin/main`. No PR is
opened and no `npm:*` label is applied, so the merge-release workflow does NOT
run — this publishes nothing. It only fast-forwards `origin/main` with your
commit.

Follow these steps exactly. Stop and report if any step fails.

1. Identify ONLY the files you changed in this session. Do NOT use
   `git add -A` or `git add .` (other agents may have uncommitted work in this
   worktree). List the specific paths you created, modified, or deleted, then
   confirm with `git status` that those are the only files you intend to stage.

2. Sync with main:
   ```bash
   git fetch origin main
   ```
   Do not run `git reset --hard`, `git checkout .`, `git clean`, or `git stash`
   — those can destroy other agents' uncommitted work.

3. Stage ONLY your files and commit with a concise, auto-generated message that
   summarizes the change:
   ```bash
   git add -- <your-file-1> <your-file-2> ...
   git commit -m "<concise, descriptive message>"
   ```
   Write the message yourself from the diff; do not prompt the user for it.
   Include `fixes #<number>` or `closes #<number>` in the commit body if there
   is a related issue or PR.

4. Rebase your work onto the latest `origin/main` so the push is a clean
   fast-forward:
   ```bash
   git rebase origin/main
   ```
   Resolve conflicts only in YOUR files. If a conflict appears in a file you did
   not modify, run `git rebase --abort` and stop and ask the user.

5. Push your commit to `origin/main`:
   ```bash
   git push origin HEAD:main
   ```
   This must be a fast-forward (step 4 guarantees it). NEVER force push. If the
   push is rejected (non-fast-forward or branch protection), stop and report —
   do not retry with `--force`.

6. Report the pushed commit SHA and confirm it landed on `origin/main`. Remind
   the user that, unlike `/pr`, this did not trigger a version bump or npm
   publish.
