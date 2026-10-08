# Search: `DocSearch`, capability index, semantic code search

Status: **agreed 2026-10-07** (part A); part B **dropped 2026-10-08**. Design only. Replaces
ledger 12.4 and closes the 8 `l1_done` tasks.

## Goal

**Part A:** add the small `DocSearch` tool so `hoocode`'s default system prompt
matches hoocode-ts, which lets the 8 ported tasks stuck at `l1_done` pass their
side-by-side checks. It also gives the model one place to ask "what can you do?".

**Part B (dropped 2026-10-08):** meaning-based code search through `CodeSearch`.
Lexical `CodeSearch` stays.

## Decisions

| Decision | Why |
|---|---|
| `DocSearch` uses hoocode-ts's name, description and schema (`query`, optional `limit`) | That is exactly what the side-by-side check compares |
| It searches **live capabilities only** (skills, subagents, installed plugins), not docs | The Rust build ships no user docs yet, and indexing hoocode-ts's docs would promise features `hoocode` lacks |
| The index is an in-process keyword (BM25) search in a new crate, `code-capabilities` | No dependencies and always available. The MCP and plugin cards reuse it. |
| Semantic code search uses the external `embsearch` program, not linked in | Linking it adds about 40 MB and ONNX to every binary; few sessions use it |

## What we build

**Part A** (right after [reliability.md](reliability.md)):

1. Crate `code-capabilities`: entries `{kind, name, description, source}`, BM25
   with code-aware word splitting (camelCase, snake_case), and exact name matches
   ranked first.
2. Producers: skills, subagents, installed plugins (MCP tools and plugins later).
3. `DocSearch` tool in the default bundle (registered when `Read` is), as in
   hoocode-ts.
4. Re-run the side-by-side scenarios for 10.2a/b/c/d/f/g, 10.4c and 10.5. If the
   self-docs list in the system prompt still differs, mask that block in the
   harness as install-specific. Mark the tasks done.

Done when: the 8 scenarios pass, and `DocSearch` has tests for results and
schema.

**Part B** (dropped; kept for the record):

5. Implement the existing `EmbsearchService` seam in `code-tool-search` against the
   `embsearch` daemon (line-based JSON over stdio). Opt-in with
   `--enable-semantic-index`; the repo is indexed in the background; anything that
   fails falls back to keyword search with a reason.

## Not doing

- Indexing docs in `DocSearch` until Rust user docs exist.
- A `SearchSkills` tool (skills stay listed in the prompt).

## Open questions

- None. Part B's question (who downloads `embsearch`) went with part B.
