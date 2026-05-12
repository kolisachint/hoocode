# Issues - windows-winget-binary

## Session 1 (Planning Phase)
- None yet — plan approved by Momus

## F4 Verification (Scope Fidelity Check - Re-Run)
- REJECT verdict due to new issue found
- Previously found issues (Task 5 temp prefixes, Task 14 manifest gen, 3 unaccounted files) are all RESOLVED
- Fix 1 (build-binaries.yml winget manifest) PASS - correctly calls generate-winget-manifest.mjs with .exe URL and sha256
- Fix 2 (release.yml winget manifest) - STRUCTURALLY present but functionally broken
- Fix 3 (v0.2.0 tag) PASS - tag exists on remote
- NEW ISSUE: release.yml path mismatch - binaries/ references point to repo-root but build-binaries.sh outputs to packages/coding-agent/binaries/
  - sha256sum binaries/hoocode-windows-x64.exe -> should be packages/coding-agent/binaries/hoocode-windows-x64.exe
  - --output binaries/kolisachint.hoocode.yaml -> binaries/ doesn't exist at repo root
  - gh release upload ... binaries/* -> wrong path
  - build-binaries.yml works correctly because it uses actions/download-artifact with path: binaries/

## F4 Re-Run #2 - Path Fix VERIFIED (2026-05-11)
- release.yml path mismatch is RESOLVED
- All references now use `BIN_DIR="packages/coding-agent/binaries"` or explicit `packages/coding-agent/binaries/` path
- No remaining bare `binaries/` references in release.yml
- Build script output path matches release.yml references
- VERDICT: APPROVE
