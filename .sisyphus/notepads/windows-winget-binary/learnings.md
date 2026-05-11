# Learnings - windows-winget-binary

## 2026-05-11 - Session Start
- Delegated tasks with `category="quick"` + `run_in_background=true` inherited read-only restriction
- Workaround: Use `run_in_background=false` for execution tasks
- Binary rename uses `scripts/build-binaries.sh` (bun build --compile), not legacy pkg config
- Cross-platform builds via `bun build --compile --external koffi`
- Backward compat preserved: `pkg.pi` fallback, `SkillDiscoveryMode = "pi"`, `pi.` API calls
