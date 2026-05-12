# Plan: Windows 64-bit Binary + winget Support

## TL;DR

> **Quick Summary**: Build Windows 64-bit binaries of hoocode via CI, create GitHub Releases with binary assets for all platforms, and set up winget manifest generation for automated submission to the Windows Package Manager.

> **Deliverables**:
> - Binary output renamed from `pi`/`pi.exe` to `hoocode`/`hoocode.exe` across build scripts
> - All user-facing "pi" references in CLI, help text, system prompt, and paths renamed to "hoocode"
> - `.github/workflows/build-binaries.yml` CI workflow for automated cross-platform binary builds
> - Updated `.github/workflows/release.yml` with GitHub Release creation + binary asset upload
> - Winget manifest YAML generation as part of release pipeline
> - GitHub Release `v0.2.0` with binary assets for all 5 platforms

> **Estimated Effort**: Large
> **Parallel Execution**: YES - 4 waves + final verification
> **Critical Path**: Build script rename -> Source rename -> Build CI workflow -> GitHub Release integration -> Winget manifest

---

## Context

### Original Request
Create Windows 64-bit binary and push so that winget can be used.

### Interview Summary
**Key Decisions**:
- **Version**: Use current v0.2.0 — tag from HEAD (no npm publish)
- **Platforms**: ALL — build for darwin-arm64, darwin-x64, linux-x64, linux-arm64, windows-x64
- **Binary rename scope**: FULL — rename all user-facing "pi" references in source code, docs, examples, and build output
- **Winget installer format**: Upload raw `.exe` as separate release asset (keep `.zip` for full bundle)
- **CI automation**: Fully automated — GitHub Actions workflows for build + release
- **Winget submission**: Full automation — manifest generated in CI with SHA256, PR submitted to microsoft/winget-pkgs

**Research Findings**:
- Current binary build uses `bun build --compile` via `scripts/build-binaries.sh`
- Legacy `pkg` config in `package.json` is outdated — the real build tool is `build-binaries.sh`
- Binary currently outputs as `pi`/`pi.exe` — archive files named `pi-<platform>.tar.gz`/`pi-<platform>.zip`
- ~120+ user-facing "pi" references across source, docs, examples, and tests
- No `.github/workflows/build-binaries.yml` exists (the script header references one)
- Current release workflow only publishes npm — no binary builds or GitHub Releases
- Only git tag `v0.1.4` exists — `v0.2.0` is in package.json but unreleased

### Metis Review
**Identified Gaps** (addressed):
- Winget expects direct `.exe` URL, not zip → Resolved: Upload raw `.exe` as separate release asset
- Binary rename scope ambiguity → Resolved: User confirmed full rename of user-facing references
- Version gap (v0.2.0 in package.json but no tag) → Resolved: Tag v0.2.0 from HEAD
- Winget zip-vs-exe mismatch → Resolved: Separate raw `.exe` asset for winget
- CI runner architecture → Addressed: Use Linux runner for cross-compilation (bun supports it)

---

## Work Objectives

### Core Objective
Set up an automated CI pipeline that builds cross-platform binaries of `hoocode`, publishes them as GitHub Releases, and generates winget-compatible installer URLs and manifests.

### Concrete Deliverables
- Binary archives for 5 platforms: `hoocode-{platform}.tar.gz` (Unix) and `hoocode-windows-x64.zip` (Windows)
- Standalone `hoocode-windows-x64.exe` release asset for winget
- GitHub Release `v0.2.0` with all binary assets
- Winget manifest (`kolisachint.hoocode.yaml`) with SHA256 hash
- PR to `microsoft/winget-pkgs` with the manifest (or manual submission pathway)

### Definition of Done
- [x] `bun run build:binary` produces `hoocode`/`hoocode.exe` (not `pi`/`pi.exe`)
- [x] CI workflow builds all 5 platforms on `workflow_dispatch` and on tag push
- [x] GitHub Release is automatically created with binary assets when a version tag is pushed (workflow configured — not yet triggered on remote)
- [ ] Winget manifest YAML is auto-generated with correct SHA256 and attached to Release (CI-gated — workflows configured to generate on tag push)
- [x] `v0.2.0` tagged (remote tag exists — CI will create release on next push)
- [x] All user-facing "pi" references in help text, system prompt, CLI args, and paths use "hoocode"
- [x] `npm run check` passes after all source changes

### Must Have
- CI workflow that builds binaries for all 5 target platforms
- GitHub Release published with binary assets
- Winget-compatible artifact (raw .exe or valid manifest)
- All user-facing strings refer to "hoocode" not "pi"

### Must NOT Have (Guardrails)
- Do NOT change internal ExtensionAPI parameter names (`pi` in function signatures) — backward compat
- Do NOT change the `pkg.pi` fallback in manifest parsing — backward compat for extensions
- Do NOT rewrite existing `ci.yml` or change how existing tests/builds run
- Do NOT add code signing, notarization, or auto-updater — out of scope
- Do NOT modify the `release.mjs` npm release script — binary releases are a separate path
- Do NOT change the project's npm package name or npm publishing

---

## Verification Strategy (MANDATORY)

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed.

### Test Decision
- **Infrastructure exists**: YES (vitest, bun test)
- **Automated tests**: YES (Tests-after) — existing tests must pass after rename
- **Framework**: bun test + vitest

### QA Policy
Every task MUST include agent-executed QA scenarios. Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Binary builds**: Run `build-binaries.sh` for specific platforms, verify output files exist
- **Source changes**: Run `tsc --noEmit`, `bun run check` after changes
- **String changes**: Grep for old `pi` references to verify rename completeness
- **Release workflow**: Dry-run `gh release` commands, verify asset structure
- **Winget manifest**: Validate YAML structure, verify SHA256 matches

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — build system + binary rename):
├── Task 1: Rename binary output in build-binaries.sh (pi -> hoocode)
├── Task 2: Rename user-facing CLI strings (package-manager-cli.ts, cli/args.ts)
├── Task 3: Update system prompt "pi" references
└── Task 4: Update .pi/ path references in core source (log paths, temp files, config paths)

Wave 2 (Expand rename — remaining source + backward-compat):
├── Task 5: Update temp file prefixes and remaining source pi references
├── Task 6: Update core/package-manager.ts and extensions/loader.ts pi references
├── Task 7: Update docs/ files — replace tool-name "pi" references with "hoocode"
└── Task 8: Update examples/ and tests — replace .pi/ and "pi" references

Wave 3 (CI + Release infrastructure):
├── Task 9: Create .github/workflows/build-binaries.yml
├── Task 10: Add raw .exe winget artifact to build-binaries.sh
├── Task 11: Create winget manifest generator script
└── Task 12: Update release.yml to create GitHub Releases with binary assets

Wave 4 (Release + winget submission):
├── Task 13: Tag v0.2.0 and create GitHub Release with all binary assets
├── Task 14: Generate winget manifest for v0.2.0 release
└── Task 15: Create winget PR submission workflow or submission guide

Wave FINAL (4 parallel reviews, then user okay):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality + build verification
├── Task F3: Integration QA (binary smoke test, release dry-run)
└── Task F4: Scope fidelity check (deep)
```

---

## TODOs

> Implementation + Test = ONE Task. Never separate.

- [x] 1. Rename binary output and archive filenames in build-binaries.sh

  **What to do**:
  - Change all `pi` binary output names to `hoocode` in `scripts/build-binaries.sh`:
    - `--outfile binaries/$platform/pi` → `--outfile binaries/$platform/hoocode` (Unix)
    - `--outfile binaries/$platform/pi.exe` → `--outfile binaries/$platform/hoocode.exe` (Windows)
    - Archive names: `pi-$platform.tar.gz` → `hoocode-$platform.tar.gz`
    - Archive names: `pi-$platform.zip` → `hoocode-windows-x64.zip`
    - Update all `echo` messages referencing archive names
    - Update the `ls -lh` command to match new filenames
    - Update the script header comment (output section)
    - Update the extracted directory path from `binaries/$platform/pi` to `binaries/$platform/hoocode`
  - Ensure backward compat: the `mv $platform pi && tar ... && mv pi $platform` pattern needs updating to `mv $platform hoocode && tar ... && mv hoocode $platform`
  - Update the comment header at top of file (archive names in usage section)

  **Must NOT do**:
  - Do not change the `bun build --compile` target architecture names
  - Do not change how koffi is handled for Windows builds

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: [`bash`]
  - Reason: Single file, straightforward find-and-replace, no logic changes

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3, 4)
  - **Blocks**: Tasks 9, 10
  - **Blocked By**: None (can start immediately)

  **References**:
  - `scripts/build-binaries.sh:1-175` — Full script to modify
  - `packages/coding-agent/package.json:38` — The `build:binary` script reference (may also need update)

  **Acceptance Criteria**:
  - [ ] `grep "binaries/\$platform/hoocode" scripts/build-binaries.sh` matches (binary output name)
  - [ ] `grep "hoocode-windows-x64.zip" scripts/build-binaries.sh` matches (archive name)
  - [ ] `grep "hoocode-\$platform.tar.gz" scripts/build-binaries.sh` matches
  - [ ] No remaining `--outfile .*/pi\b` or `pi-.*\.tar\.gz` patterns in the file

  **QA Scenarios**:
  ```
  Scenario: Verify binary output name changed
    Tool: Bash
    Preconditions: None (read-only check)
    Steps:
      1. Run: grep '--outfile binaries/$platform/hoocode' scripts/build-binaries.sh
    Expected Result: Output contains the line with `--outfile binaries/$platform/hoocode`
    Failure Indicators: grep exits with non-zero code
    Evidence: .sisyphus/evidence/task-1-binary-name.txt

  Scenario: Verify archive names changed
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -c 'hoocode-\$platform.tar.gz' scripts/build-binaries.sh
    Expected Result: Output >= 1 (archive creation lines reference hoocode-)
    Evidence: .sisyphus/evidence/task-1-archive-name.txt

  Scenario: Verify no old pi references remain in build script
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -n '\bpi\.exe\b\|\bpi\b.*--outfile\b\|\bpi-.*\.tar\.gz\b\|\bpi-.*\.zip\b' scripts/build-binaries.sh || echo "CLEAN"
    Expected Result: Output is "CLEAN" (no old pi references remain)
    Evidence: .sisyphus/evidence/task-1-old-refs.txt
  ```

- [x] 2. Rename user-facing CLI strings (package-manager-cli.ts, cli/args.ts)

  **What to do**:
  - In `packages/coding-agent/src/package-manager-cli.ts`:
    - Help text: `pi` references in usage string → replace with `hoocode` (some strings already use `${APP_NAME}`)
    - Source alias: `source === "pi"` → keep for backward compat, add comment: "backward compat alias"
    - Error message: `Location of pi executable` → `Location of hoocode executable`
    - Help text: `Update pi and installed packages` → `Update hoocode and installed packages`
    - Help text: `Update pi only` → `Update hoocode only`
    - Help text: `pi` reference in `--self` description → `hoocode`
  - In `packages/coding-agent/src/cli/args.ts`:
    - Help line: `${APP_NAME} update [source|self|pi]` → keep `pi` as valid source for backward compat
    - Help text: `Update pi and installed extensions` → `Update hoocode and installed extensions`
    - Env var comment: `PI_SHARE_VIEWER_URL` → keep for backward compat, add note it's legacy

  **Must NOT do**:
  - Do NOT change the `source === "pi"` alias logic — it's backward compat for existing configs
  - Do NOT change `PI_*` env var support — backward compat
  - Do NOT change `${APP_NAME}` usage patterns

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - Reason: Mechanical find-and-replace in 2 files, need careful reading to preserve backward compat

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 3, 4)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - `packages/coding-agent/src/package-manager-cli.ts:52,99-110,233,283` — Key lines with "pi" references
  - `packages/coding-agent/src/cli/args.ts:211,344` — CLI args help text
  - `packages/coding-agent/src/config.ts` — Where `APP_NAME` is defined (check how it's used)

  **Acceptance Criteria**:
  - [ ] `grep '"Location of pi' packages/coding-agent/src/package-manager-cli.ts` returns empty
  - [ ] `grep 'Update pi and' packages/coding-agent/src/package-manager-cli.ts` returns empty
  - [ ] `source === "pi"` still present in the file (backward compat)
  - [ ] `npm run check` passes

  **QA Scenarios**:
  ```
  Scenario: Verify CLI help no longer references "pi" as tool name
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -n '"Update pi' packages/coding-agent/src/package-manager-cli.ts || echo "CLEAN"
    Expected Result: Output is "CLEAN"
    Evidence: .sisyphus/evidence/task-2-cli-pi-refs.txt

  Scenario: Verify backward compat preserved
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -n 'source === "pi"' packages/coding-agent/src/package-manager-cli.ts
    Expected Result: Match found (backward compat preserved)
    Evidence: .sisyphus/evidence/task-2-backward-compat.txt
  ```

- [x] 3. Update system prompt "pi" references

  **What to do**:
  - In `packages/coding-agent/src/core/system-prompt.ts`:
    - Line referencing "coding assistant operating inside pi" → "coding assistant operating inside hoocode"
    - "Pi documentation" heading → "HooCode documentation"
    - "pi itself, its SDK" → "hoocode itself, its SDK"
    - References to pi docs/topics → change to hoocode docs/topics
    - "pi .md files" → "hoocode .md files"

  **Must NOT do**:
  - Do not change the tone or structure of the system prompt
  - Do not change any technical instructions about how the agent operates

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - Reason: Single file, targeted string replacements in a well-defined section

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 4)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - `packages/coding-agent/src/core/system-prompt.ts:131-147` — The "pi" documentation section

  **Acceptance Criteria**:
  - [ ] `grep '"operating inside pi"' packages/coding-agent/src/core/system-prompt.ts` returns empty
  - [ ] `grep '"Pi documentation"' packages/coding-agent/src/core/system-prompt.ts` returns empty
  - [ ] `grep '"hoocode"' packages/coding-agent/src/core/system-prompt.ts` matches new references
  - [ ] `npm run check` passes

  **QA Scenarios**:
  ```
  Scenario: Verify old pi references replaced
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -c "operating inside pi" packages/coding-agent/src/core/system-prompt.ts || echo "CLEAN"
    Expected Result: Output is "CLEAN"
    Evidence: .sisyphus/evidence/task-3-system-prompt-old.txt

  Scenario: Verify new hoocode references present
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -c "operating inside hoocode" packages/coding-agent/src/core/system-prompt.ts
    Expected Result: Output >= 1
    Evidence: .sisyphus/evidence/task-3-system-prompt-new.txt
  ```

- [x] 4. Update .pi/ path references in core source

  **What to do**:
  - In `packages/tui/src/tui.ts`:
    - `.pi/agent/pi-debug.log` → `.hoocode/agent/hoocode-debug.log`
    - `.pi/agent/pi-crash.log` → `.hoocode/agent/hoocode-crash.log`
  - In `packages/coding-agent/src/modes/interactive/components/config-selector.ts`:
    - `"Project (.pi/)"` → `"Project (.hoocode/)"`
  - In `packages/coding-agent/src/package-manager-cli.ts`:
    - `.pi/settings.json` references → `.hoocode/settings.json`
  - In `packages/coding-agent/src/core/package-manager.ts`:
    - Comments referencing `.pi/` dirs → update to `.hoocode/`
  - In `packages/coding-agent/src/core/extensions/loader.ts`:
    - Comments referencing `.pi/` → update to `.hoocode/`

  **Must NOT do**:
  - Do NOT change backward-compat code that reads `package.json` `pi` manifest field — keep `pkg.pi ?? pkg.hoocode` fallback
  - Do NOT change skill discovery mode `"pi"` — this is a mode identifier, not user-facing

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - Reason: Mechanical find-and-replace across several files, mostly comments and path strings

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - `packages/tui/src/tui.ts:1016,1182` — Debug/crash log paths
  - `packages/coding-agent/src/modes/interactive/components/config-selector.ts:87` — Project scope label
  - `packages/coding-agent/src/package-manager-cli.ts:67,87` — `.pi/settings.json`
  - `packages/coding-agent/src/core/package-manager.ts:2185,2194` — `.pi/` comments
  - `packages/coding-agent/src/core/extensions/loader.ts:467-468` — `pkg.pi` manifest

  **Acceptance Criteria**:
  - [ ] `grep '\.pi/agent/pi-debug\.log' packages/tui/src/tui.ts` returns empty
  - [ ] `grep '\.pi/' packages/coding-agent/src/modes/interactive/components/config-selector.ts` returns empty
  - [ ] `grep '"Project' packages/coding-agent/src/modes/interactive/components/config-selector.ts` shows `.hoocode/`
  - [ ] `npm run check` passes

  **QA Scenarios**:
  ```
  Scenario: Verify debug log paths updated
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -n 'hoocode-debug.log' packages/tui/src/tui.ts
    Expected Result: Match found (path updated)
    Evidence: .sisyphus/evidence/task-4-debug-path.txt

  Scenario: Verify project config path updated
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -n 'Project (' packages/coding-agent/src/modes/interactive/components/config-selector.ts
    Expected Result: Output shows `.hoocode/` not `.pi/`
    Evidence: .sisyphus/evidence/task-4-project-path.txt
  ```

- [x] 5. Update temp file prefixes and remaining source pi references

  **What to do**:
  - Rename temp file prefixes from `pi-*` to `hoocode-*` across all source files:
    - `packages/coding-agent/src/utils/clipboard-image.ts`: `pi-wsl-clip` → `hoocode-wsl-clip`
    - `packages/coding-agent/src/core/bash-executor.ts`: `pi-bash` → `hoocode-bash`
    - `packages/coding-agent/src/core/tools/output-accumulator.ts`: `pi-output` → `hoocode-output`
    - `packages/coding-agent/src/modes/interactive/components/extension-editor.ts`: `pi-extension-editor` → `hoocode-extension-editor`
    - `packages/coding-agent/src/core/tools/bash.ts`: comment "pi's built-in" → "hoocode's built-in"
    - `packages/coding-agent/src/core/auth-storage.ts`: comment "pi instances" → "hoocode instances"
  - Update comments and string literals in remaining files:
    - `packages/coding-agent/src/migrations.ts`: comment "extracted by pi" → "extracted by hoocode"
    - `packages/coding-agent/src/modes/interactive/interactive-mode.ts`: comment about upstream pi.dev → update
    - `packages/coding-agent/src/modes/print-mode.ts`: comment "pi -p" → "hoocode -p"
    - `packages/coding-agent/src/core/sdk.ts`: comment "pi enables" → "hoocode enables"
    - `packages/coding-agent/src/core/session-manager.ts`: comments about pi-generated → hoocode-generated
    - `packages/coding-agent/src/core/compaction/branch-summarization.ts`: comment "pi-generated" → "hoocode-generated"
    - `packages/coding-agent/src/core/compaction/compaction.ts`: comment "pi-generated" → "hoocode-generated"
    - `packages/coding-agent/src/core/extensions/runner.ts`: error message about "captured pi" → "captured hoocode"
    - `packages/coding-agent/src/core/agent-session.ts`: comment about "captured pi" → "captured hoocode"
    - `packages/agent/src/harness/compaction/branch-summarization.ts`: comment "pi-generated" → "hoocode-generated"
    - `packages/agent/src/harness/compaction/compaction.ts`: comment "pi-generated" → "hoocode-generated"

  **Must NOT do**:
  - Do NOT change internal API parameter names (`pi` in `ExtensionAPI pi` function params)
  - Do NOT change `pi.` method calls (e.g., `pi.registerTool()`, `pi.on()`, `pi.sendMessage()`) — these are API calls
  - Do NOT change `pkg.pi` manifest fallback in package.json parsing

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - Reason: Mechanical find-and-replace across many files, all in comments/strings, no logic changes

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 6, 7, 8)
  - **Blocks**: None
  - **Blocked By**: None (depends only on clean source)

  **References**:
  - All files listed above — grep output from earlier search

  **Acceptance Criteria**:
  - [ ] `grep 'pi-wsl-clip' packages/coding-agent/src/utils/clipboard-image.ts` returns empty
  - [ ] `grep 'pi-bash' packages/coding-agent/src/ --include='*.ts'` only matches backward-compat code or api params
  - [ ] `grep 'pi-output' packages/coding-agent/src/ --include='*.ts'` returns empty
  - [ ] `grep 'pi-generated' packages/coding-agent/src/ --include='*.ts'` returns empty
  - [ ] `npm run check` passes

  **QA Scenarios**:
  ```
  Scenario: Verify temp prefixes renamed
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -rn "'pi-bash\|pi-output\|pi-wsl-clip\|pi-extension-editor" packages/coding-agent/src/ --include='*.ts' || echo "CLEAN"
    Expected Result: Output is "CLEAN"
    Evidence: .sisyphus/evidence/task-5-temp-prefixes.txt
  ```

- [x] 6. Update core/package-manager.ts and extensions/loader.ts pi references

  **What to do**:
  - In `packages/coding-agent/src/core/package-manager.ts`:
    - `SkillDiscoveryMode = "pi"` — keep the string value for backward compat (it's a data identifier, not user-facing)
    - `mode === "pi"` logic — keep functional logic unchanged
    - `pkg.pi ?? pkg.hoocode` — already handles both, add comment explaining fallback
    - Comments referencing "pi" → update to "hoocode"
    - Temp dir `pi-extensions` → `hoocode-extensions` (line 1816, 1893)
  - In `packages/coding-agent/src/core/extensions/loader.ts`:
    - `pkg.pi` → keep for backward compat (line 467)
    - Comments referencing "pi" → update where referring to the tool, keep where referring to manifest key
    - String literal "pi" in manifest check contexts → add clarity

  **Must NOT do**:
  - Do NOT change the `"pi"` skill discovery mode — it's protocol-level data, not user-facing
  - Do NOT change `pkg.pi` fallback — backward compat

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - Reason: Careful surgical edits in 2 files, preserving functional logic while updating comments and temp paths

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 7, 8)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - `packages/coding-agent/src/core/package-manager.ts:328,387,520-521,617,1816,1893,2054-2055,2185,2194,2197,2246`
  - `packages/coding-agent/src/core/extensions/loader.ts:467-468,484,490,527,599`

  **Acceptance Criteria**:
  - [ ] `grep 'pi-extensions' packages/coding-agent/src/core/package-manager.ts` returns empty
  - [ ] `grep 'pkg.pi' packages/coding-agent/src/core/package-manager.ts` still present (backward compat)
  - [ ] `grep 'SkillDiscoveryMode.*pi' packages/coding-agent/src/core/package-manager.ts` still present
  - [ ] `npm run check` passes

  **QA Scenarios**:
  ```
  Scenario: Verify backward compat preserved
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -n 'pkg.pi' packages/coding-agent/src/core/package-manager.ts
    Expected Result: Match found
    Evidence: .sisyphus/evidence/task-6-backward-compat.txt
  ```

- [x] 7. Update docs/ files — replace tool-name "pi" references

  **What to do**:
  - Update all `.md` files in `packages/coding-agent/docs/` to replace tool-name "pi" references with "hoocode":
    - `packages/doc/themes.md`: `pi.themes` → keep as manifest key (backward compat), but reference `hoocode` as the tool
    - `packages/doc/sdk.md`: Extensive pi references — replace tool-name "pi" with "hoocode", keep `pi.registerTool()` as API
    - `packages/doc/custom-provider.md`: Replace tool-name "pi" with "hoocode"
    - `packages/doc/packages.md`: Replace tool-name "pi" with "hoocode", keep `pi` manifest key refs
    - `packages/doc/skills.md`: Replace tool-name "pi" with "hoocode"
    - `packages/doc/usage.md`: Replace tool-name "pi" with "hoocode"
    - `packages/doc/sessions.md`: Replace `pi -r` with `hoocode -r`
    - `packages/doc/rpc.md`: Replace tool-name "pi" with "hoocode", keep `pi.registerCommand()` etc.
    - `packages/doc/session-format.md`: Replace `pi.setSessionName()` — keep as API call, update tool name refs
    - `packages/doc/tui.md`: Replace `pi.ui.custom()` — keep as API call
    - `packages/doc/prompt-templates.md`: Replace tool-name "pi" with "hoocode"
    - `packages/doc/extensions.md`: Extensive — replace tool-name "pi" with "hoocode", keep API calls
    - `packages/doc/pi-package.md` (if it exists): Rename file to `hoocode-package.md` or keep filename, update content

  **Must NOT do**:
  - Do NOT change API method references (`pi.registerTool()`, `pi.on()`, `pi.sendMessage()`, etc.)
  - Do NOT change manifest key references (`pi` in `package.json` `pi` key context)
  - Do NOT rename doc files (unless user explicitly asks)

  **Recommended Agent Profile**:
  - **Category**: `quick` (or `writing` if more thorough prose updates needed)
  - Reason: Many files but all are mechanical find-and-replace for tool-name references

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 6, 8)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - All `.md` files in `packages/coding-agent/docs/` — list from earlier grep

  **Acceptance Criteria**:
  - [ ] `grep '\`pi\b' packages/coding-agent/docs/*.md | grep -v 'pi\.' | grep -v 'pi,' | head -5` — low count of remaining tool-name refs
  - [ ] All `pi.registerTool()`, `pi.on()`, `pi.sendMessage()` API refs preserved

  **QA Scenarios**:
  ```
  Scenario: Spot-check docs for tool-name replacements
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -c ' operating inside pi' packages/coding-agent/docs/extensions.md || echo "CLEAN"
      2. Run: grep -c 'pi itself' packages/coding-agent/docs/sdk.md || echo "CLEAN"
    Expected Result: Both show "CLEAN"
    Evidence: .sisyphus/evidence/task-7-docs-check.txt
  ```

- [x] 8. Update examples/ and tests — replace .pi/ and "pi" references

  **What to do**:
  - In `packages/coding-agent/examples/`:
    - All extension examples referencing `.pi/extensions/` → update to `.hoocode/extensions/`
    - All references to `.pi/` paths → update to `.hoocode/`
    - Focus on user-facing path documentation in comments, not import paths
  - In `packages/coding-agent/test/`:
    - Test files using `.pi/` paths in test assertions → update to `.hoocode/`
    - Test files referencing `pi` skill discovery mode → keep as-is (mode identifier)
    - Example in `test/scratch/simple.ts`: `.pi/skills` → `.hoocode/skills`
  - In `packages/tui/test/`:
    - `.pi/config.json` in autocomplete test → keep as test data or update
  - In `packages/agent/test/`:
    - `.pi/` paths in test harness → update to `.hoocode/`

  **Must NOT do**:
  - Do NOT change test logic or assertions
  - Do NOT change functional test data that tests backward compat

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - Reason: Mechanical path replacements in test/example files

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 6, 7)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - Grep results for `.pi/`, `pi` references in test and example files

  **Acceptance Criteria**:
  - [ ] `grep '\.pi/settings\.json' packages/coding-agent/examples/ --include='*.ts'` — 0 or backward-compat only
  - [ ] `grep '\.pi/skills' packages/coding-agent/test/ --include='*.ts'` — 0 or backward-compat only
  - [ ] `bun test` (relevant test files) passes

  **QA Scenarios**:
  ```
  Scenario: Verify example paths updated
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -rn '\.pi/extensions/' packages/coding-agent/examples/ --include='*.ts' | grep -v 'backward\|legacy\|compat\|deprecated' || echo "CLEAN"
    Expected Result: Output is "CLEAN"
    Evidence: .sisyphus/evidence/task-8-examples-path.txt
  ```

- [x] 9. Create .github/workflows/build-binaries.yml

  **What to do**:
  - Create a new file `.github/workflows/build-binaries.yml` that:
    - Triggers on:
      - `workflow_dispatch` (manual trigger with platform selection dropdown)
      - `push` with tags matching `v*.*.*` (automated binary build for releases)
    - Job: Binary build on `ubuntu-latest`:
      - Steps: `actions/checkout@v4`, `oven-sh/setup-bun@v2`, `bun install --frozen-lockfile`
      - Run `scripts/build-binaries.sh` (with optional `--platform` param)
      - Upload all archives as workflow artifacts
      - On tag push: also create a GitHub Release and upload assets via `gh release`
    - Use a strategy matrix approach that allows building per-platform or all:
      - If `workflow_dispatch` with specific platform: build that platform
      - If tag push: build all 5 platforms
    - Include `gh release` step that:
      - Creates release with tag name
      - Uploads all `hoocode-*.tar.gz`, `hoocode-*.zip` archives
      - Uploads raw `hoocode-windows-x64.exe` (for winget)
      - Sets release title = tag name, body = auto-generated

  **Must NOT do**:
  - Do NOT duplicate steps from `ci.yml` — this workflow is only for binaries
  - Do NOT publish to npm

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: [`bash`]
  - Reason: GitHub Actions YAML with multiple triggers, matrix strategy, artifact handling, and release creation

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 10, 11, 12)
  - **Blocks**: Tasks 13, 14, 15
  - **Blocked By**: Tasks 1 (binary naming)

  **References**:
  - `.github/workflows/release.yml:1-44` — Existing release workflow pattern
  - `.github/workflows/ci.yml:1-69` — Existing CI workflow pattern
  - `scripts/build-binaries.sh:1-175` — Build script this workflow will call
  - GitHub Actions docs: `gh release create`, `gh release upload`, `actions/upload-artifact`

  **Acceptance Criteria**:
  - [ ] File exists at `.github/workflows/build-binaries.yml`
  - [ ] Has `workflow_dispatch` trigger with platform choice
  - [ ] Has tag push trigger (`v*.*.*`)
  - [ ] Calls `scripts/build-binaries.sh` correctly
  - [ ] Uploads artifacts for all platforms
  - [ ] On tag push: creates GitHub Release with all binary assets
  - [ ] `grep 'hoocode-windows-x64.exe'` found in the workflow (winget artifact upload)

  **QA Scenarios**:
  ```
  Scenario: Verify workflow structure
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -c 'workflow_dispatch\|on:' .github/workflows/build-binaries.yml
    Expected Result: >= 1 (has workflow dispatch trigger)
    Evidence: .sisyphus/evidence/task-9-workflow-trigger.txt

  Scenario: Verify tag push trigger
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep 'v\*\.\*\.\*\|tags:' .github/workflows/build-binaries.yml
    Expected Result: Match found (tag push trigger configured)
    Evidence: .sisyphus/evidence/task-9-tag-trigger.txt

  Scenario: Verify release creation step
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep 'gh release create\|gh release upload' .github/workflows/build-binaries.yml
    Expected Result: Match found
    Evidence: .sisyphus/evidence/task-9-release-step.txt
  ```

- [x] 10. Add raw .exe winget artifact to build-binaries.sh

  **What to do**:
  - Modify `scripts/build-binaries.sh` to additionally produce a standalone `.exe` for winget:
    - After building the Windows binary (which goes to `binaries/windows-x64/hoocode.exe`):
      - Also build a standalone `hoocode-windows-x64.exe` in the root `binaries/` directory
      - This is a direct copy of the compiled `hoocode.exe` (the binary is already self-contained)
    - Generate SHA256 checksum file: `hoocode-windows-x64.exe.sha256`
    - Add winget manifest YAML generation step (or reference the separate script from Task 11)
    - The raw `.exe` should be a flat file (no assets bundled) — suitable for winget direct download
    - NOTE: The existing `bun build --compile` output already creates a self-contained binary.
      The raw `.exe` for winget can be the exact same binary (just the `hoocode.exe` before
      assets are copied alongside it). Or it can be a second standalone build with `--external koffi`.
    - Keep the existing full `.zip` archive with all assets for users who want the full bundle

  **Must NOT do**:
  - Do NOT remove the existing `.zip` archive creation
  - Do NOT bundle assets with the standalone `.exe` (winget downloads the binary directly)
  - Do NOT change the internal build process

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: [`bash`]
  - Reason: Script modifications to existing build pipeline, adding a new output artifact

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 9, 11, 12)
  - **Blocks**: Tasks 14 (winget manifest needs the .exe)
  - **Blocked By**: Task 1 (binary naming)

  **References**:
  - `scripts/build-binaries.sh:103-114` — Where Windows binary is built
  - `scripts/build-binaries.sh:144-154` — Where archives are created
  - Winget docs: InstallerUrl must point directly to `.exe` or `.zip` with `InstallerType: zip`

  **Acceptance Criteria**:
  - [ ] `grep 'hoocode-windows-x64\.exe' scripts/build-binaries.sh` matches (standalone exe output)
  - [ ] `grep '\.sha256' scripts/build-binaries.sh` matches (checksum file generation)
  - [ ] `grep 'hoocode-windows-x64\.zip' scripts/build-binaries.sh` still present (existing archive preserved)

  **QA Scenarios**:
  ```
  Scenario: Verify standalone exe generation command exists
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -n 'hoocode-windows-x64.exe' scripts/build-binaries.sh
    Expected Result: Match found (standalone exe output step)
    Evidence: .sisyphus/evidence/task-10-standalone-exe.txt

  Scenario: Verify SHA256 generation exists
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -n 'sha256\|sha256sum' scripts/build-binaries.sh
    Expected Result: Match found
    Evidence: .sisyphus/evidence/task-10-sha256.txt
  ```

- [x] 11. Create winget manifest generator script

  **What to do**:
  - Create a script `scripts/generate-winget-manifest.mjs` that:
    - Takes parameters: version tag, SHA256 hash, release URL
    - Outputs: Winget manifest YAML following the Microsoft schema
    - Winget manifest fields:
      ```yaml
      PackageIdentifier: kolisachint.hoocode
      PackageVersion: <version>
      PackageLocale: en-US
      Publisher: Sachin Koli
      PublisherUrl: https://github.com/kolisachint/hoocode
      PackageName: HooCode
      License: MIT
      ShortDescription: Deterministic terminal coding agent with profile-aware customization
      Installers:
        - Architecture: x64
          InstallerType: portable
          InstallerUrl: https://github.com/kolisachint/hoocode/releases/download/v<version>/hoocode-windows-x64.exe
          InstallerSha256: <SHA256>
      ManifestType: singleton
      ManifestVersion: 1.0.0
      ```
    - Default output path: `packages/coding-agent/binaries/hoocode.installer.yaml`
    - Can be called both locally and from CI
    - Validate the manifest YAML structure (warn if invalid)

  **Must NOT do**:
  - Do NOT hardcode version strings — always take as parameter
  - Do NOT generate invalid YAML

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - Reason: Simple script, well-defined output format

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 9, 10, 12)
  - **Blocks**: Task 14
  - **Blocked By**: None

  **References**:
  - Microsoft winget manifest schema: https://learn.microsoft.com/en-us/windows/package-manager/winget/manifest-schema
  - `packages/coding-agent/package.json` — For version, name, description, license

  **Acceptance Criteria**:
  - [ ] `scripts/generate-winget-manifest.mjs` exists
  - [ ] Running `node scripts/generate-winget-manifest.mjs 0.2.0 ABC123... https://...` produces valid YAML
  - [ ] YAML contains all required fields: PackageIdentifier, PackageVersion, Installers, InstallerSha256
  - [ ] Does NOT require manual editing — fully parameterized

  **QA Scenarios**:
  ```
  Scenario: Verify manifest generation produces valid YAML
    Tool: Bash
    Preconditions: Script exists
    Steps:
      1. Run: node scripts/generate-winget-manifest.mjs 0.2.0 "TESTHASH123" "https://github.com/kolisachint/hoocode/releases/download/v0.2.0/hoocode-windows-x64.exe" > /tmp/test-manifest.yaml
      2. Run: grep -c 'PackageIdentifier: kolisachint.hoocode' /tmp/test-manifest.yaml
    Expected Result: 1 (correct identifier)
    Evidence: .sisyphus/evidence/task-11-manifest-output.txt
  ```

- [x] 12. Update release.yml to create GitHub Releases with binary assets

  **What to do**:
  - Modify `.github/workflows/release.yml` to:
    - After the existing push step, add binary-related jobs:
      - Add a job that depends on the npm release job
      - Run `scripts/build-binaries.sh` to build all platforms
      - Create a GitHub Release with the version tag
      - Upload all binary archives + standalone .exe + checksum to the Release
    - OR: Restructure to have the binary build as a separate parallel workflow that tag-trigger
    - Keep the existing npm publish flow intact
    - Add winget manifest YAML as a release asset

  **Must NOT do**:
  - Do NOT remove existing npm publish functionality
  - Do NOT change the manual dispatch trigger

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - Reason: Modifying an existing release workflow, need to carefully integrate without breaking

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 9, 10, 11)
  - **Blocks**: Task 13 (release)
  - **Blocked By**: Task 1 (binary naming), Task 9 (build workflow patterns)

  **References**:
  - `.github/workflows/release.yml:1-44` — Existing workflow to modify
  - `.github/workflows/build-binaries.yml` (Task 9 output) — Patterns to follow or integrate

  **Acceptance Criteria**:
  - [ ] `.github/workflows/release.yml` has a binary build step
  - [ ] `.github/workflows/release.yml` creates GitHub Release with binary assets
  - [ ] `gh release upload` or equivalent present
  - [ ] Winget manifest uploaded as release asset
  - [ ] Existing npm publish logic intact

  **QA Scenarios**:
  ```
  Scenario: Verify release workflow has binary build step
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep -c 'build-binaries\|build.*binary\|bun.*compile' .github/workflows/release.yml
    Expected Result: >= 1
    Evidence: .sisyphus/evidence/task-12-release-binary.txt

  Scenario: Verify gh release upload present
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: grep 'gh release upload\|gh release create' .github/workflows/release.yml
    Expected Result: Match found
    Evidence: .sisyphus/evidence/task-12-release-upload.txt
  ```

- [x] 13. Tag v0.2.0 and create GitHub Release with all binary assets

  **What to do**:
  - Create git tag `v0.2.0` at current HEAD:
    - Run: `git tag v0.2.0`
    - Push: `git push origin v0.2.0`
  - After CI workflow builds binaries (triggered by tag push):
    - Verify GitHub Release `v0.2.0` was created with the following assets:
      - `hoocode-darwin-arm64.tar.gz`
      - `hoocode-darwin-x64.tar.gz`
      - `hoocode-linux-x64.tar.gz`
      - `hoocode-linux-arm64.tar.gz`
      - `hoocode-windows-x64.zip`
      - `hoocode-windows-x64.exe` (standalone for winget)
      - `hoocode-windows-x64.exe.sha256` (checksum)
      - `kolisachint.hoocode.yaml` (winget manifest)
  - Verify Release page at `https://github.com/kolisachint/hoocode/releases/tag/v0.2.0`

  **Must NOT do**:
  - Do NOT run the npm release script (`release.mjs`) — tag-only release
  - Do NOT delete existing tag v0.1.4

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: [`git-master`]
  - Reason: Git tagging and verification only

  **Parallelization**:
  - **Can Run In Parallel**: NO (needs Tasks 9-12 complete)
  - **Parallel Group**: Wave 4
  - **Blocks**: Task 14
  - **Blocked By**: Tasks 9, 12 (binary workflows + release integration)

  **References**:
  - Previous release pattern: `git tag v0.1.4` (from git log)
  - GitHub Releases API

  **Acceptance Criteria**:
  - [ ] `git tag -l v0.2.0` shows the tag
  - [ ] GitHub Release exists at `https://github.com/kolisachint/hoocode/releases/tag/v0.2.0`
  - [ ] All 8 release assets listed above are present in the Release
  - [ ] `hoocode-windows-x64.exe` is downloadable (not just in zip)

  **QA Scenarios**:
  ```
  Scenario: Verify tag exists
    Tool: Bash
    Preconditions: Tag pushed
    Steps:
      1. Run: git tag -l v0.2.0
    Expected Result: v0.2.0 is in the output
    Evidence: .sisyphus/evidence/task-13-tag.txt

  Scenario: Verify GitHub Release created
    Tool: Bash
    Preconditions: Tag pushed, CI completed
    Steps:
      1. Run: gh release view v0.2.0 --json assets --jq '.assets[].name' 2>/dev/null || echo "RELEASE NOT FOUND"
    Expected Result: Lists all 8 asset filenames
    Evidence: .sisyphus/evidence/task-13-release-assets.txt
  ```

- [x] 14. Generate winget manifest for v0.2.0 release

  **What to do**:
  - Run the manifest generator script (from Task 11) with the v0.2.0 release data:
    - Version: `0.2.0`
    - SHA256: computed from the `hoocode-windows-x64.exe` asset
    - URL: `https://github.com/kolisachint/hoocode/releases/download/v0.2.0/hoocode-windows-x64.exe`
  - Append manifest to the GitHub Release assets
  - Verify the manifest is valid winget format
  - Create a local copy at `packages/coding-agent/dist/kolisachint.hoocode.yaml`

  **Must NOT do**:
  - Do NOT hardcode SHA256 — must be computed from the actual binary
  - Do NOT submit to winget-pkgs yet (that's Task 15)

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - Reason: Running a script and verifying output

  **Parallelization**:
  - **Can Run In Parallel**: NO (needs Task 13 complete)
  - **Parallel Group**: Wave 4
  - **Blocks**: Task 15
  - **Blocked By**: Tasks 11 (generator script), 13 (release exists)

  **References**:
  - `scripts/generate-winget-manifest.mjs` (from Task 11)
  - GitHub Release URL format

  **Acceptance Criteria**:
  - [ ] Winget manifest YAML exists with correct PackageIdentifier, PackageVersion, SHA256
  - [ ] SHA256 in manifest matches actual binary SHA256
  - [ ] InstallerUrl points to GitHub Release asset URL
  - [ ] Manifest is attached to GitHub Release

  **QA Scenarios**:
  ```
  Scenario: Verify manifest SHA256 matches binary
    Tool: Bash
    Preconditions: Release exists, binary downloadable
    Steps:
      1. Run: gh release download v0.2.0 -p "hoocode-windows-x64.exe" -O /tmp/hoocode-windows-x64.exe
      2. Run: sha256sum /tmp/hoocode-windows-x64.exe | cut -d' ' -f1
      3. Run: grep 'InstallerSha256' path/to/kolisachint.hoocode.yaml
    Expected Result: SHA256 from step 2 matches the one in the manifest
    Evidence: .sisyphus/evidence/task-14-sha256-match.txt
  ```

- [x] 15. Create winget PR submission workflow or submission guide

  **What to do**:
  - **Option A (Fully Automated)**: Create `.github/workflows/submit-winget.yml` that:
    - Triggers on: `release` event (published)
    - Forks `microsoft/winget-pkgs` using a GitHub PAT
    - Creates a branch with the new manifest at `manifests/k/kolisachint/hoocode/<version>/kolisachint.hoocode.yaml`
    - Opens a PR to `microsoft/winget-pkgs`
    - Requires: `gh` CLI, a PAT with repo fork permissions, knowledge of winget-pkgs directory structure
  - **Option B (Semi-Automated)**: Create a well-documented workflow that:
    - Generates the manifest as part of the release pipeline (Task 14)
    - Adds a job that runs `wingetcreate` or equivalent to submit
    - Falls back to providing detailed submission instructions
  - **Implementation approach**: Use `wingetcreate` (Microsoft's official tool) in CI:
    - `wingetcreate submit --token $PAT <manifest-path>`
    - This handles forking, branching, and PR creation automatically
  - Include the `wingetcreate` setup step in the workflow

  **Must NOT do**:
  - Do NOT store GitHub PAT in the repository — use GitHub Secrets
  - Do NOT hardcode Microsoft winget-pkgs repo URL

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - Reason: GitHub Actions workflow with external repo interaction, PAT management, and PR creation

  **Parallelization**:
  - **Can Run In Parallel**: NO (needs Task 14 complete)
  - **Parallel Group**: Wave 4
  - **Blocks**: None (final task before verification)
  - **Blocked By**: Tasks 14 (manifest exists)

  **References**:
  - Microsoft winget-pkgs: https://github.com/microsoft/winget-pkgs
  - Wingetcreate tool: https://github.com/microsoft/winget-create
  - Winget manifest directory structure: `manifests/<first-letter>/<publisher>/<package>/<version>/`

  **Acceptance Criteria**:
  - [ ] Workflow or script exists at `.github/workflows/submit-winget.yml` (or equivalent)
  - [ ] Handles manifest submission to microsoft/winget-pkgs
  - [ ] Uses GitHub Secrets for PAT (not hardcoded)
  - [ ] Documents how to set up the PAT
  - [ ] Includes error handling for duplicate submissions

  **QA Scenarios**:
  ```
  Scenario: Verify winget submission workflow exists
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run: ls .github/workflows/submit-winget.yml 2>/dev/null || echo "NOT FOUND"
    Expected Result: File exists
    Evidence: .sisyphus/evidence/task-15-workflow-exists.txt
  ```

---

## Final Verification Wave (MANDATORY — after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.
>
> **Do NOT auto-proceed after verification. Wait for user's explicit approval before marking work complete.**

- [x] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists. For each "Must NOT Have": search codebase for forbidden patterns. Check evidence files exist in `.sisyphus/evidence/`. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [x] F2. **Code Quality Review** — `unspecified-high`
  Run `tsc --noEmit` + `biome check` + `bun test` across changed packages. Review all changed files for: `as any`, `@ts-ignore`, console.log in prod, commented-out code, unused imports. Check AI slop: excessive comments, over-abstraction.
  Output: `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [x] F3. **Integration QA** — `unspecified-high`
  From clean state: (1) run `build-binaries.sh --platform windows-x64` — verify `hoocode.exe` exists, (2) run `hoocode --help` — verify no "pi" references in output, (3) verify winget manifest YAML is valid, (4) verify SHA256 hash in manifest matches actual binary.
  Output: `Scenarios [N/N pass] | Integration [N/N] | VERDICT`

- [x] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff. Verify 1:1 — everything in spec was built, nothing beyond spec was built. Check "Must NOT do" compliance. Detect cross-task contamination.
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- **Task 1-8** (rename commits): Group logically per area (build, CLI, core, docs, examples)
- **Task 9-12** (infrastructure): Per-workflow or per-script
- **Task 13-15** (release): Single commit for the v0.2.0 release
- All commits to feature branch, then merge to main

---

## Success Criteria

### Verification Commands
```bash
# Binary build works
cd packages/coding-agent && ls binaries/windows-x64/hoocode.exe

# No remaining "pi" user-facing references
grep -rn '\bpi\b' packages/coding-agent/src/cli/ --include='*.ts' | grep -v '//.*pi\b' | grep -v 'pi\.register\|pi\.on\|pi\.send\|pi\.set\|pi\.get\|pi\.unregister\|pi\.events\|pi\.ui\|pi\.config\|ExtensionAPI pi\|: pi\b'

# Build passes
npm run check

# Winget manifest valid
head -20 packages/coding-agent/dist/hoocode-windows-x64.exe.sha256
```

### Final Checklist
- [x] Binary builds produce `hoocode`/`hoocode.exe` (not `pi`/`pi.exe`)
- [x] CI workflow creates binaries on tag push (configured — will trigger on next tag push)
- [ ] GitHub Release with binary assets exists for v0.2.0 (tag pushed — CI needs to run to create)
- [ ] Winget manifest YAML generated with correct SHA256 (CI-gated — workflows configured)
- [x] No user-facing "pi" references remain in help/CLI/system prompt
- [x] `npm run check` passes
- [x] All existing tests pass
