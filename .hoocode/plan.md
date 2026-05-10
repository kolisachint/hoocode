# Implementation Plan

## Goal
Fix two branding/UX issues in the hoocode agent:
1. Greeting message incorrectly references "pi" instead of "hoocode"
2. Changelog displays on every login instead of only showing new entries once

---

## Issue 1: Fix "pi" reference in system prompt

### Problem
The system prompt tells the AI "You are an expert coding assistant operating inside pi, a coding agent harness" which causes the AI to respond with "I'm pi..." when users greet it.

### Files to modify

**File:** `packages/coding-agent/src/core/system-prompt.ts`
**Line:** 131
**Change:** Replace "pi" with "hoocode" in the system prompt identity statement.

```typescript
// Before:
let prompt = `You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.

// After:
let prompt = `You are an expert coding assistant operating inside hoocode, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.
```

Also update the Pi documentation reference section (lines 145-150) to say "Hoocode documentation" instead of "Pi documentation".

---

## Issue 2: Fix changelog showing on every login

### Problem
The `getChangelogForDisplay()` function in `interactive-mode.ts` sets `lastChangelogVersion` to `VERSION` (the current app version) after displaying changelog entries. However, if the app version doesn't match the latest changelog entry version, the same entries will be shown again on next startup.

For example:
- App VERSION = "0.74.0"
- Changelog has entries for: [Unreleased], 0.74.0, 0.73.1, 0.73.0...
- User sees entries for 0.74.0 and any others newer than their last seen version
- System records lastChangelogVersion = "0.74.0" (VERSION)
- On next run, if the comparison logic has any issues or if new entries are added, all changes are shown again

### Root Cause
The function stores `VERSION` as the last seen version, but should store the actual latest version from the displayed entries to properly track what the user has seen.

### Files to modify

**File:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts`
**Function:** `getChangelogForDisplay()` (lines 833-860)
**Change:** When new entries are found and displayed, set `lastChangelogVersion` to the version of the latest entry (highest version number) instead of `VERSION`.

```typescript
// Current code (lines 853-858):
const newEntries = getNewEntries(entries, lastVersion);
if (newEntries.length > 0) {
    this.settingsManager.setLastChangelogVersion(VERSION);  // BUG: uses app VERSION
    this.reportInstallTelemetry(VERSION);
    return newEntries.map((e) => e.content).join("\n\n");
}

// Fixed code:
const newEntries = getNewEntries(entries, lastVersion);
if (newEntries.length > 0) {
    // Use the latest entry's version instead of app VERSION
    // newEntries is sorted by parseChangelog, so first entry is latest
    const latestEntry = newEntries[0];
    const latestVersion = `${latestEntry.major}.${latestEntry.minor}.${latestEntry.patch}`;
    this.settingsManager.setLastChangelogVersion(latestVersion);
    this.reportInstallTelemetry(latestVersion);
    return newEntries.map((e) => e.content).join("\n\n");
}
```

---

## Tests

### Manual verification steps:

1. **System prompt fix:**
   - Start the agent
   - Type "hi" 
   - Verify response says "I'm hoocode..." not "I'm pi..."

2. **Changelog fix:**
   - Clear/reset the `lastChangelogVersion` in settings.json
   - Start the agent - changelog should display
   - Exit the agent
   - Check that settings.json has `lastChangelogVersion` set to the latest version from CHANGELOG.md
   - Start the agent again - changelog should NOT display

---

## Verification Commands

```bash
# Build and check
npm run check

# Test specifically the coding-agent package
cd packages/coding-agent
npx tsx ../../node_modules/vitest/dist/cli.js --run
```

---

## Summary of Changes

| File | Lines | Change |
|------|-------|--------|
| `packages/coding-agent/src/core/system-prompt.ts` | 131, 145-150 | Replace "pi" with "hoocode" in system prompt and documentation references |
| `packages/coding-agent/src/modes/interactive/interactive-mode.ts` | 855-857 | Fix changelog version tracking to use latest entry version instead of app VERSION |
