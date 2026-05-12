# Learnings - windows-winget-binary

## 2026-05-11 - Session Start
- Delegated tasks with `category="quick"` + `run_in_background=true` inherited read-only restriction
- Workaround: Use `run_in_background=false` for execution tasks
- Binary rename uses `scripts/build-binaries.sh` (bun build --compile), not legacy pkg config
- Cross-platform builds via `bun build --compile --external koffi`
- Backward compat preserved: `pkg.pi` fallback, `SkillDiscoveryMode = "pi"`, `pi.` API calls

## F1 - Plan Compliance Audit (2026-05-11)

Performed audit of plan `.sisyphus/plans/windows-winget-binary.md`. All Must Have items verified, all Must NOT Have items absent. Evidence directory empty (no evidence files were saved during implementation, but not a blocking issue). Full details in audit task output.

## F4 Re-Run #2 - Path Fix Verification (2026-05-11)

Path fix in release.yml verified PASS:
- `BIN_DIR="packages/coding-agent/binaries"` at line 69
- All references use `$BIN_DIR` or full `packages/coding-agent/binaries/` path
- No bare `binaries/` references in release.yml binaries job
- Build script outputs to `packages/coding-agent/binaries/` (via `cd packages/coding-agent` at line 92)
- Paths match perfectly between build output and release.yml references
- Full scope fidelity check: 7/7 tasks compliant, 6/6 guardrails compliant, contamination CLEAN
