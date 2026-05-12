# Issues Log

## 2026-05-11: Integration QA

### Winget filename mismatch (BLOCKER)
- `build-binaries.yml` generates `binaries/hoocode-windows-x64.yaml` (line 83)
- `submit-winget.yml` downloads `kolisachint.hoocode.yaml` (line 15)
- These must match for winget submission to work
- Additionally, `release.yml` does not generate any winget manifest, so releases from that workflow will always lack a manifest

### Stale upstream URL (LOW)
- `packages/coding-agent/src/cli/args.ts:344` references `https://pi.dev/session/` as default share viewer URL
- Should be updated to a HooCode-specific URL or removed
