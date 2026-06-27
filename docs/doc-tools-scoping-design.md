# Design Note: Scoped / partial DocRead (scan · grep · read)

**Status:** Proposal (design only — no code yet)
**Motivation:** A full `DocRead` is token-heavy. It always extracts the *whole*
document and renders the entire id-addressed tree, capped only by dumb
head-truncation (`DOCREAD_MAX_RENDER_TOKENS`, `filetools-shared.ts:32-40`). You
cannot today ask for "just this section," "an outline," or "the blocks matching
X." This note proposes closing that gap.

## TL;DR

- The `filetools` **Rust binary already implements** the token-sensitive loop we
  want: `scan` (outline), `grep` (search), `read` (hydrate specific blocks). **No
  binary/Rust change is required** — all the work is TypeScript wiring in
  hoocode.
- hoocode currently only wires up `extract` and `reconstruct`
  (`runFiletools(... "extract" | "reconstruct" ...)`, `filetools-shared.ts:268`).
  `scan`/`grep`/`read` are unused.
- Recommended: expose them as **three cheap, read-only discovery tools**
  (`DocScan`, `DocGrep`, and a partial `DocRead`-by-id), keeping the existing
  full `DocRead` (= `extract`) as the editable, id-map-bearing read used right
  before an edit.

## What the binary already gives us

From `kolisachint/filetools` `docs/usage.md` and `README.md` (the
"token-sensitive loop: `scan` structure, `grep` by text, `read` only the blocks
you need"):

### `scan` — structural outline / manifest (the real "glimpse")

| Flag | Purpose | Default |
|------|---------|---------|
| `--input <path>` | File to scan | required |
| `--offset <n>` | Skip first N blocks | 0 |
| `--limit <n>` | Max blocks returned | 0 (no limit) |

Outputs JSON **to stdout**: block structure + previews, no full content.
Maps to the user's **`docscan` / depth-outline** idea.

### `grep` — locate blocks by text without hydrating

| Flag | Purpose | Default |
|------|---------|---------|
| `--input <path>` | File to search | required |
| `--pattern <s>` | Literal substring match per line | required |
| `--ignore-case` | Case-insensitive | off |
| `--limit <n>` | Stop after N matches | 0 (no limit) |

Outputs `{ pattern, returned, matches: [{ block_id, line, snippet, writable }] }`
to **stdout**. Maps to the user's **`docgrep` / match** idea.

### `read` — hydrate specific blocks by id

| Flag | Purpose | Default |
|------|---------|---------|
| `--input <path>` | File to read | required |
| `--id <id>` | Block id to hydrate (repeatable) | all blocks if omitted |
| `--offset <n>` | Pagination start | none |
| `--limit <n>` | Pagination size | none |

Accepts structural paths, `part:<name>` markers, and xlsx `sheet[n].rows[a-b]`
ranges, e.g. `paragraph[3]`, `table[0]`, `sheet[0].rows[0-99]`. Outputs hydrated
block content as JSON to **stdout**. Maps to the user's **"certain portion only"
/ node** idea.

## The two id layers (the crux)

There are two addressing schemes, and the design hinges on how they connect:

| Layer | Produced by | Id shape | Purpose |
|-------|-------------|----------|---------|
| Discovery | `scan` / `grep` / `read` | block paths — `paragraph[3]`, `sheet[0].rows[0-99]` | cheap, read-only navigation |
| Edit | `extract` → `reconstruct` | opaque `#id` envelope + sidecar id-map | lossless patching |

The filetools docs say a `grep` `block_id` "feeds straight back into `read` or a
**patch**," i.e. the two layers share an addressing space. **But** `reconstruct`
still requires the `extract` envelope + sidecar to be present
(`filetools-shared.ts:433-467`). So the likely reality — **to verify during
implementation** — is:

- `scan`/`grep`/`read` are sufficient to *find and read* a portion cheaply.
- A full `extract` (our current `DocRead`) is still required to *edit*, because
  reconstruct needs the sidecar id-map.

If block ids turn out to be directly patchable without a prior `extract`, that's
a strictly better path and would let us drop the heavy `extract` from the edit
flow too — but we should not assume it.

## Proposed hoocode shape

**Recommendation: three discovery tools, leave the edit path alone.**

1. `DocScan(path, offset?, limit?)` → run `scan`, render the manifest. The cheap
   first step; replaces "DocRead readonly:true" as the recommended glimpse.
2. `DocGrep(path, pattern, ignoreCase?, limit?)` → run `grep`, render matches
   with their `block_id`s.
3. Partial read: either `DocRead(path, id?, offset?, limit?)` gaining the
   `read` semantics, **or** a separate `DocPeek`. Naming caution: filetools calls
   this `read`, but hoocode's `DocRead` already means `extract`. Folding `id`
   into `DocRead` overloads one tool with two backends (`extract` when no `id`,
   `read` when scoped) — cleaner to keep them distinct. **Leaning toward a
   separate partial-read tool** to avoid that overload; open for discussion.

Full `DocRead` (= `extract`) stays as-is: the editable, id-map-bearing read you
do immediately before `DocEdit`/`DocWrite`.

### Resulting recommended flow

```
DocScan            → outline, cheap                 (offset/limit to page)
  └─ DocGrep       → find blocks by text            (block_ids + snippets)
       └─ DocRead-by-id → hydrate just those blocks (no full dump)
            └─ (only if editing) full DocRead → DocEdit / DocWrite
```

This supersedes the "`DocRead readonly:true` glimpse" advice in
[`doc-tools-flow.md`](doc-tools-flow.md): `scan` is the proper cheap glimpse, and
`readonly` becomes a niche (analysis projection of a full extract). Update that
doc when this lands.

## Integration concerns (TS side)

1. **stdout vs file output.** `runFiletools` is built around extract/reconstruct,
   which write to `--out` files and emit only a status line on stderr; callers
   read the files, not stdout (`filetools-shared.ts:9-17,268-287`).
   `scan`/`grep`/`read` write their JSON to **stdout**. We need a stdout-capturing
   variant (or generalize `runFiletools` to return stdout).
2. **Wire types.** Add types for the scan manifest and grep match shapes
   (`{ block_id, line, snippet, writable }`) and a hydrated-block shape. Keep them
   locked against the Rust `model.rs`, same discipline as the existing envelope
   types.
3. **Caching.** Discovery is stateless/read-only; it should **not** populate the
   extract record cache (which underpins `DocEdit`/`DocWrite`). Keep the discovery
   path out of `records` so it can't mask a stale edit cache.
4. **Rendering + token budget.** Reuse `truncateRenderToTokenBudget` for the
   scan/read renders. `scan` should comfortably fit; `read` of a huge range may
   still need the budget + an "narrow your range" hint.
5. **`--enable-filetools` gating + tool registration.** Same gating as the
   existing doc tools; register in `tools/index.ts` (`ToolName`, `allToolNames`,
   the create/definition switches, and the `createAll*` maps).
6. **Prompt guidance.** Add `promptGuidelines` steering the loop: scan → grep →
   read-by-id, and only fall back to a full `DocRead` when about to edit.

## Decision summary

- **Binary vs TS:** TS only. The Rust binary already ships `scan`/`grep`/`read`;
  there is nothing to add upstream for v1.
- **Scope:** all three primitives (scan, grep, partial read) — they're the
  intended loop and the binary already supports them.
- **Shape:** three read-only discovery tools; do not disturb the
  extract→patch→reconstruct edit path.

## Open questions to resolve before implementing

1. Can a `grep`/`read` `block_id` be used **directly** as a `reconstruct` patch
   target without a prior `extract`? (Docs hint yes; reconstruct's sidecar
   requirement hints no. Verify.)
2. Partial-read tool: fold `id` into `DocRead`, or a separate tool? (Leaning
   separate.)
3. Do `scan`/`grep`/`read` support PDF and all OOXML types, or a subset? Confirm
   per-handler coverage so guidance doesn't promise unsupported formats.
4. Exact stdout JSON schemas for `scan` and `read` (the note has `grep`'s; `scan`
   and `read` shapes need to be read from the Rust source / examples).
