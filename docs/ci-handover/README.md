# Parked: the `installer-windows` CI job

`docs/ci-handover/ci.yml` is `.github/workflows/ci.yml` with one job added. The
credentials this branch was prepared with have no `workflow` OAuth scope, so
every push touching `.github/workflows` is refused — the same wall
`2fbed3c` hit. The file is finished; it just needs a client allowed to write
workflows.

Parked as a whole file rather than described in a PR comment, so applying it is
a move and not a transcription:

```sh
git mv docs/ci-handover/ci.yml .github/workflows/ci.yml
rmdir docs/ci-handover 2>/dev/null || rm -rf docs/ci-handover
```

Check the base file has not moved underneath it first:

```sh
git diff origin/main -- .github/workflows/ci.yml   # expect: no output
```

If that prints anything, `ci.yml` changed on main after this was parked — take
the `installer-windows` job out of the parked copy and append it to the current
file instead of overwriting.

## What the job is

The existing `installers` job parses `install.ps1` and stops there. Its own
comment said actually installing "would need a published release to install
from", but `HOOCODE_RELEASE_BASE_URL` exists precisely so it does not —
`install.sh` calls it "the installers' own tests". Nothing ever used it.

All three installer bugs fixed in this PR were runtime errors in the last
thirty lines of `install.ps1`, every one of them shipped under a green parse.

The job serves a release off disk on a `windows-latest` runner and installs
from it for real:

- **First install** — archive unpacks, `hoocode.cmd` and `hoo.cmd` land, the
  bin directory reaches the user PATH.
- **Re-install over it** — the same, on a machine that already has HooCode.
  This is the path the `.Count` bug also threw on, and the reason re-running
  the installer never repaired anything.
- **A failing install under `iex`** — pinned to a version that is not there,
  asserting the shell is still alive afterwards. That is the whole difference
  between `exit` and a terminating error when a script runs in the caller's
  scope.

`-NoTools` keeps it offline and deterministic: pre-seeding reaches out to five
GitHub repos and would make the job flaky for a step the install does not
depend on.

## What was verified

Run against the *unfixed* `install.ps1`, the job fails — first install exits 1
on `The property 'Count' cannot be found on this object`. Restoring `exit 1` in
`Fail` trips the shell-survival check instead. Both assertions were confirmed
to catch the regression they exist for, rather than passing vacuously.

The job body was run end to end on Linux `pwsh` 7.4.6 (with `python3` for
`python`, POSIX path separators, and a stub for the Windows-only
`Unblock-File`), exiting 0 with the server torn down. What that run could
**not** cover, and what a reviewer should watch on the first real execution:

- `Unblock-File` against genuinely downloaded files.
- `[Environment]::SetEnvironmentVariable(..., 'User')`, which is a silent no-op
  off Windows — so the PATH assertion is the one line that has never actually
  executed. It is also the assertion that matters most.
- `python` resolving on the runner image (it is `python3` on Linux).
