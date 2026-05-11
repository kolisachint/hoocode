# Decisions - windows-winget-binary

## Session 1 (Planning Phase)
- **Version**: v0.2.0, tag from HEAD (no npm publish)
- **Platforms**: All 5 (darwin-arm64, darwin-x64, linux-x64, linux-arm64, windows-x64)
- **Binary rename scope**: FULL - rename all user-facing "pi" references
- **Winget installer**: Upload raw `.exe` as separate asset (keep `.zip` for full bundle)
- **API backward compat**: `pi.` method calls and `pkg.pi` manifest fallback preserved
- **Winget submission**: Use `wingetcreate` tool approach (Microsoft official)
- **Winget PackageIdentifier**: `kolisachint.hoocode`
