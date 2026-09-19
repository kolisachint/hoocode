# Release handover — finish what PR #251 could not

**This whole directory is temporary. Deleting it is the last step of the
follow-up PR described below.**

## Why it exists

PR #251 was prepared in an environment whose GitHub credentials lacked the
`workflow` OAuth scope. Every push touching `.github/workflows/` was refused:

```
! [remote rejected] refusing to allow an OAuth App to create or update workflow
  `.github/workflows/ci.yml` without `workflow` scope
```

So the four workflow files that PR needs are parked here instead, where an
ordinary push can carry them. They are finished and reviewed — they just need
moving into place by a client that is allowed to write workflows (any normal
`git push` from a laptop, or the GitHub web editor, which is always allowed).

## State as of this commit

- PR #251 is merged (or about to be). Its code is on `main`.
- **The `npm:patch` label was deliberately removed before merging**, so no
  release was published. npm is still on `0.5.79`.
- That was on purpose: `merge-release.yml` on `main` still uploads only
  `hoocode-windows-x64.zip`. Releasing before the fix below would publish a
  version whose macOS and Linux installers 404 — and the website already
  advertises them.

## What is broken until this is done

`https://kolisachint.github.io/hoocode/` is live and shows:

```bash
curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh
```

That script resolves, runs, and then cannot find an archive for the user's
platform, because no release has one yet. It fails cleanly — it prints the URL
it checked and points at `npm install -g @kolisachint/hoocode-agent` — but until
the steps below are done, the headline install route on the website does not
work for anyone except Windows users.

## The follow-up PR

```bash
git checkout main && git pull
git checkout -b pr/release-workflows

git mv docs/release-handover/ci.yml            .github/workflows/ci.yml
git mv docs/release-handover/release.yml       .github/workflows/release.yml
git mv docs/release-handover/merge-release.yml .github/workflows/merge-release.yml
git mv docs/release-handover/welcome.yml       .github/workflows/welcome.yml
git rm docs/release-handover/README.md

git commit -m "Publish every platform's archive, and welcome first-timers"
git push -u origin pr/release-workflows
```

Open the PR and **label it `npm:patch`**. Merging it cuts the release.

### Then verify, in this order

1. **CI on the PR.** The new `installers` job runs for the first time here and is
   the first real check of `install.ps1` — it has never been through a PowerShell
   parser, because the container it was written in had no `pwsh`. A parse error
   here is a bug in that file, not a regression.
2. **The release.** After merge, `merge-release.yml` bumps to `0.5.80`, tags,
   publishes to npm and uploads the assets. Expect **10 assets**:
   - `hoocode-linux-x64.tar.gz`
   - `hoocode-linux-arm64.tar.gz`
   - `hoocode-linux-x64-musl.tar.gz`
   - `hoocode-linux-arm64-musl.tar.gz`
   - `hoocode-darwin-x64.tar.gz`
   - `hoocode-darwin-arm64.tar.gz`
   - `hoocode-windows-x64.zip`
   - `checksums.txt`
   - `install.sh`
   - `install.ps1`

   Only the Windows zip means the workflow move did not take.
3. **The one-liner, for real:**
   ```bash
   curl -fsSL https://kolisachint.github.io/hoocode/install.sh | sh
   hoocode --version
   ```
4. **The website.** In `kolisachint/kolisachint.github.io`, run `npm run docs`
   and commit any drift. It should be close to a no-op — the docs and both
   installers were vendored by hand from this branch — but from now on the
   weekly sync in `deploy.yml` keeps them current from `main`.

## What each workflow file changes

| File | Change |
|------|--------|
| `release.yml` | The `binaries` job uploads all seven archives plus `checksums.txt` and both installers, instead of only the Windows zip. |
| `merge-release.yml` | The same change, for the merge-triggered release path — this is the one a labelled PR fires. |
| `ci.yml` | New `installers` job: parses `install.sh` with `dash` (it is piped to `sh`, which is not bash on Debian/Ubuntu or Alpine), parses `install.ps1` with the PowerShell parser, and smoke-runs `build-binaries.sh --list` and `test.sh --help`. Nothing else in the repo executes any of those, so without this job a typo ships to the front door and is found by a stranger. |
| `welcome.yml` | New. `actions/first-interaction` replies to somebody's first issue or PR. |

## Already verified, so you do not have to

- All four files parse as YAML, and carry no emoji (AGENTS.md: "no emojis in
  commits, issues, PR comments, or code").
- `scripts/build-binaries.sh --targets linux-x64` was run for real: it produced a
  42MB tarball and a `checksums.txt`, and `hoocode --version` ran out of the
  extracted archive. `release.yml` runs that same script.
- `install.sh` was run end to end against a locally served release —
  download, checksum verification, unpack, symlink, PATH write, re-run as an
  upgrade, PATH idempotency across three installs — plus both failure paths: a
  tampered checksum and a missing platform archive each refuse, exit non-zero,
  and leave the previous install working.

## Still unverified

- **`install.ps1` has never been parsed.** See step 1 above.
- **The external-tool asset names** in both installers mirror
  `packages/coding-agent/src/utils/tools-manager.ts`, but could not be checked
  against live GitHub releases — the sandbox proxy returned 403 for the GitHub
  API. A wrong name 404s, the installer warns and carries on, and hoocode fetches
  the tool on first use as before, so the failure is soft. Worth confirming once
  against a real install.
- **No `windows-arm64` build** — bun has no such compile target. `install.ps1`
  installs the x64 build there, which Windows runs under emulation, and says so
  once.

## Two open judgment calls from PR #251

Neither blocks the release.

- Emoji were removed from the issue-template chooser labels to satisfy
  AGENTS.md. Keeping them there is common practice on GitHub and arguably helps
  scanning; put them back if you prefer.
- `test.sh` is new surface area. It exists because `CONTRIBUTING.md`,
  `docs/install.md`, `docs/development.md` and the PR template all already told
  contributors to run it, and it did not exist. Fixing the root `test` script to
  work under bun would be a smaller footprint if you would rather go that way.
