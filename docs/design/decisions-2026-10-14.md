# Decisions, 2026-10-14: closing plan step 0

Made with the user while closing steps 0a–0d of [README.md](README.md). Where an earlier
page disagrees, this page wins.

| # | Question | Answer |
|---|---|---|
| 1 | Delete `hoocode-agent-mcp`? | **No. Keep it.** It is the home of the MCP client that card 9.1 ([mcp.md](mcp.md)) rebuilds on rmcp. The delete row for `agent-mcp` in [decisions-2026-10-08.md](../../archive/docs/decisions-2026-10-08.md) no longer applies. |
| 2 | Migration task 13.4 (TS test port ledger) | **Dropped**, not moved. Its `moved_to` pointed at `docs/design/ts-to-rust-migration.md`, which does not exist, and it has no design card. The ledger status is now `dropped`. |
