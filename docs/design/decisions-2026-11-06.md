# Decisions, 2026-11-06

Made with the user while reviewing performance. Where an earlier page disagrees, this page wins.

## Performance

The page is [performance.md](performance.md).

| # | Question | Answer |
|---|---|---|
| 1 | Where performance status lives | **One page: [performance.md](performance.md).** It is the index and the status of record. [concurrency.md](concurrency.md) and [perf-short-sessions.md](perf-short-sessions.md) point to it. |
| 2 | Design review plan (v2) | **Approved.** Build order: P-R0 first; then P-C4, P-M4, P-M3, P-C3, P-X1, P-X2, P-X3, P-S5; then P-C1 and P-M6; then Step 0 and measure, then P-S1 to P-S3 and P-M1; P-C2 only if measurement shows it pays. |
| 3 | PR #177 review | **No concurrency or speed lost.** One gap in a closed item: Responses tool-call routing (`crates/hoocode-ai-provider-openai-responses/src/shared.rs:597-786`). Tracked as P-R0, a correctness fix. |
| 4 | C5 worker parse | **Folded into the deferred lazy model catalog.** Not a separate item. |
| 5 | Dropped items | **Dropped:** M2 (one copy per message, unless measured); M5 (running totals, unless measured); permission config once per batch (breaks the Always cascade); app-server registry cache (per-RPC reload is the live reload); HTTP/2; bounded stream queue; wire change on text deltas; S4 (the loop stays per delta). |

## Open

Open questions 1 to 5 are in [performance.md](performance.md#open-questions). Each has a recommendation first.
