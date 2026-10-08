# hoocode-tui-fuzzy

Fuzzy matching for the hoocode TUI.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

Subsequence fuzzy matching (query characters must appear in order, not
necessarily consecutively) with scoring that rewards consecutive runs, word
boundaries, and exact matches, plus an alpha/digit-swap fallback (`"2v"`
also matches `"v2"`). `fuzzy_filter` filters and sorts a list by best match,
supporting space-separated multi-token queries.

Ported from TypeScript `@kolisachint/hoocode-tui` → `fuzzy.ts`.
