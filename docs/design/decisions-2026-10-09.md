# Decisions, 2026-10-09: TUI activity panel, subagent attach, goldens

Agreed with the user in a design session. The card is [tui-activity.md](tui-activity.md).
Where this page disagrees with an earlier one, this page wins.

| # | Question | Answer |
|---|---|---|
| 1 | Where subagent session data lives | Sibling dir `<session>/subagents/<task_id>.jsonl`, plus a link entry in the parent (a `custom` entry, so hoocode-ts can still read the file) |
| 2 | What "recover" means | View and resume on request; no auto-resume |
| 3 | Attach UX | Full screen, as a **true transcript swap** (not an overlay) |
| 4 | Input while attached | Read-only |
| 5 | Live log source | Forwarded child events, with jsonl backfill on attach |
| 6 | Retention | As long as the parent session; success no longer deletes |
| 7 | Panel content | Plan items, subagents, background shell, scheduled jobs, MCP, as **separate tabs** to reduce clutter |
| 8 | Panel focus key | **Ctrl+G** (Ctrl+T stays "toggle thinking"); Tab / Shift+Tab switch tabs |
| 9 | Teams lens and roster | Delete; rebuild focus on a flat row model |
| 10 | Background shell jobs | In scope: build the registry and tools |
| 11 | Replacement for L1/L2 | L1 plus hoocode-only goldens; text diffs fail; LLM review advisory. Parity with hoocode-ts is no longer required |
| 12 | Visual review input | Text grid plus style legend; PNG only for changed or uncertain screens |
| 13 | Golden capture | Both tiers: in-process component goldens and tmux end-to-end scenarios |
| 14 | Who reviews | A Haiku subagent in-session (no API key, not in CI) |
| 15 | Simplification ideas | Prerequisites first; the rest is a ranked backlog |
| 16 | Who implements | Haiku subagents, one per phase step, with an orchestrating session |
| 17 | `/agents` command | Yes: a picker of this session's subagent runs |
| 18 | Quit with background shell jobs running | Ask once; "kill all" is the default |
| 19 | hoocode-ts replay fixtures | Keep until T0 has run green for a week |
| 20 | Migration ledger | Freeze `ledger.py` read-only; track new work in the card |
| 21 | Where the TUI plan sits | Side by side with the core plan in README.md, not merged into its order. When the user asks to proceed, ask which plan and step |
| 22 | Order inside the TUI plan | **Simplification first**: after T0 (goldens, the safety net), work Phase S of [tui-activity.md](tui-activity.md) (the full per-component inventory) before any feature phase. Replaces answer 15 |
| 23 | Who does the work | **Always Haiku subagents** for all TUI work: review, implement, test, visual review. The main session only orchestrates, briefs, checks results and merges |
| 24 | Sixel images (HI3) | Delete the Sixel path; no decoder |
| 25 | One TUI design doc | The simplification inventory is merged into tui-activity.md; tui-simplify.md is removed |
| 26 | Narrowing the list | Never drop an idea to narrow it. Prioritise into tiers (Now, Next, Later). Only verified non-issues leave the list, recorded under "Checked, not an issue" |
