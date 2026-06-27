# Design Note: Scoped / partial DocRead (scan · grep · read)

**Status:** Implemented as `DocScan` / `DocGrep` / `DocPeek` (see
`packages/coding-agent/src/core/tools/docscan.ts`, `docgrep.ts`, `docpeek.ts`
and `docs/doc-tools-flow.md`). This note is kept as the design record; the
"id layers" finding below was confirmed against the real binary during
implementation.
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

## The id schemes (the crux) — verified against the real binary

There are **two** id shapes, and which command emits/accepts which is the whole
ballgame. Confirmed by running the binary on `<doc><title>Hello</title>…</doc>`:

| Command | Emits / accepts | Id shape | Connects to |
|---------|-----------------|----------|-------------|
| `scan` | emits | structural path — `node[title:0]` (the `el_` id is in `content_hash`) | `read`/`DocPeek` |
| `grep` | emits `block_id` | opaque `el_…` — **the edit id space** | `reconstruct`/`DocEdit` |
| `read` | accepts `--id` | structural path (`node[title:0]`); **rejects `el_` ids → 0 results** | from `scan` |
| `extract`→`reconstruct` | patch targets | opaque `el_…` `#id` + sidecar id-map | from `grep` or a hydrated `read`/`DocRead` |

So the connective tissue is **not** a single shared id space:

- **`DocScan` path id → `DocPeek`** hydrates the block; the hydrated nodes then
  carry the **`el_` ids → `DocEdit`**.
- **`DocGrep` `el_` id → `DocEdit`** directly (the fast path from "find" to
  "edit"). A `DocGrep` id will **not** work in `DocPeek`.

This is reflected in the three tools' `promptGuidelines` so the agent routes ids
correctly. The earlier worry — that `reconstruct` might self-extract — is moot:
it mandates a pre-built envelope + sidecar idmap, verified against the Rust
source (`src/bin/filetools.rs`):

```rust
Reconstruct {
    #[arg(long)] envelope: PathBuf,   // required (PathBuf, not Option)
    #[arg(long)] patch:    PathBuf,
    #[arg(long)] out:      PathBuf,
    #[arg(long)] original: Option<PathBuf>,
}
// ... write(&env, &idmap, &original, &patch)
```

So `reconstruct` **cannot** run from `--original` alone and does **not** extract
internally — an `extract` envelope + `.idmap.json` sidecar must already exist.
The conclusion:

- `scan`/`grep`/`read` find and read a portion cheaply and yield block ids.
- An `extract` is still mandatory in the *pipeline* before reconstruct — but it
  does **not** have to be an agent-issued `DocRead` (see next section).

## The edit path needs no `DocRead` (auto-extract)

The agent never has to call `DocRead` to edit. `DocEdit`/`DocWrite` already
auto-extract: `reconstructDocument` → `ensureExtractRecord`
(`filetools-shared.ts:409-467`) runs `filetools extract` for you whenever the
cache is missing or the file changed on disk (shipped v0.4.92). The mandatory
extract happens in the **TS wrapper**, not inside Rust `reconstruct`.

So in the **target** design (once `scan`/`grep`/`read` are wired): the agent
gets ids from discovery, then calls `DocEdit`/`DocWrite` directly — the wrapper
extracts under the hood. `DocRead` (full `extract`) drops out of the agent's
flow entirely; its only remaining roles are (a) the internal auto-extract the
wrapper performs, and (b) a fallback when you want the full editable tree
explicitly.

Caveat to confirm during implementation: a `grep`/`read` block id must be a
valid patch target *against the auto-extracted envelope's id space*. The docs
assert it is ("feeds straight back into a patch"); the TS-side id validation in
`reconstructDocument` (`findMissingPatchIds`) must be checked against block-path
ids so it doesn't reject them as "stale."

## Verified: format coverage

Ran the real binary on fixtures for each format (XML inline; docx/xlsx/pdf
hand-built as valid OOXML/PDF, since LibreOffice is non-functional in the test
env). Results:

| Format | `DocScan` | `DocGrep` | `DocPeek` | `DocRead`/`DocEdit` |
|--------|-----------|-----------|-----------|---------------------|
| XML / drawio | ✅ | ✅ | ✅ (path ids) | ✅ |
| docx (OOXML word) | ✅ | ✅ (`el_` ids) | ✅ (`paragraph[n]`) | ✅ |
| pdf | ✅ | ✅ (`page[n]`, `pdf_*`) | ✅ (`page[n]`) | ✅ |
| xlsx (OOXML calc) | ✅ | ✅ (cell values) | ✅ (`sheet[n].rows[a-b]`) | ✅ |
| pptx (OOXML slides) | ✅ | ✅ (`slide[n]`) | ✅ (`slide[n]`) | ✅ |

**The xlsx gap is closed as of `filetools` `v0.1.7`** ("reach full cell/text
content via scan/grep/read for all formats", upstream PR
[#11](https://github.com/kolisachint/filetools/pull/11)). The full loop now
reaches cell/text content for every supported format. Re-verified against the
v0.1.7 binary on hand-built OOXML fixtures: for **xlsx**, `DocScan` previews now
include cell text (`Rows 0-1: Item, MAGICVALUE`), `DocGrep` matches cell values
(returning the `sheet[n].rows[a-b]` block), and `DocPeek` of that exact row block
hydrates the cells with their text and editable ids — matching the CSV handler it
used to lag. **pptx** is likewise verified end-to-end (`DocGrep` matches slide
text, `DocPeek` hydrates `slide[n]`). The earlier finding below records the
pre-v0.1.7 behaviour for history.

> **Historical (pre-v0.1.7): xlsx was the gap, handler-specific (not a fixture
> artifact).** `DocScan` reported sheet structure and a row-range outline
> (`sheet[0].rows[0-99]`), and `DocRead`/`DocEdit` saw and edited all cell text,
> but the discovery loop did **not** reach xlsx cell content: `DocGrep` matched
> only the sheet name, and `DocPeek` of the row-range blocks returned zero rows
> (only `sheet[0]` hydrated). Confirmed then on both inline-string and
> shared-strings fixtures, and isolated to the OOXML calc handler by contrast
> with **CSV** (same `rows[0-99]` addressing, where the loop already worked). The
> `DocGrep`/`DocPeek` guidelines that steered spreadsheet cell work to
> `DocRead`/`DocEdit` have been dropped now that the loop reaches cells.

**Usage rule confirmed: `DocPeek` needs the EXACT id `DocScan` emits.** A
hand-built sub-range (`rows[0-2]`) returns nothing; only the exact `rows[0-99]`
block hydrates. This is now stated in `DocPeek`'s schema + guidelines.

## Verified: token cost (measured, not estimated)

`test/filetools-token-cost.test.ts` measures rendered-output tokens on an
~80-section XML document large enough that a full `DocRead` truncates:

| Path | Tokens | vs full read |
|------|--------|--------------|
| Full `DocRead` | **9927** (truncated — cannot see the late section) | 1× |
| `DocScan` (outline, 80 blocks) | 2045 | — |
| `DocGrep` (one pattern) | **43** | ~230× cheaper |
| `DocPeek` (one section body) | 180 | — |
| **scan → grep → peek loop** | **2268** | **~4.4× cheaper** |

Two takeaways: (1) the loop is several × cheaper and dominated by `DocScan`
(paginate it on huge docs); `DocGrep`-only is the cheapest path to "find → edit."
(2) The full read **truncated past the target section**, but `DocGrep` still
found it — the loop reaches content a full `DocRead` cannot. (Numbers scale with
document size; this is one representative point, regenerated by the test.)

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
            └─ DocEdit / DocWrite directly          (wrapper auto-extracts;
                                                      no DocRead step needed)
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

## Open questions — resolved during implementation

1. ~~Can a `grep`/`read` `block_id` be patched without a prior `extract`?~~
   **Resolved.** `reconstruct` mandates a pre-built envelope+sidecar, so an
   `extract` is always required — but the TS wrapper performs it automatically
   (`ensureExtractRecord`), so it is not an agent step. And the id schemes split
   (see above): `DocGrep` already returns the editable `el_` `#ids`, so a grep
   hit patches via `DocEdit` directly; `DocScan` path ids do not, so the path is
   `DocScan → DocPeek → el_ id → DocEdit`.
2. ~~Partial-read tool: fold `id` into `DocRead`, or separate?~~ **Resolved:**
   separate tool (`DocPeek`), to avoid overloading `DocRead`'s extract/cache
   semantics with a second, non-caching backend.
3. ~~Do `scan`/`grep`/`read` cover PDF and all OOXML types?~~ **Resolved (see
   "Verified: format coverage").** XML/docx/pdf/xlsx/pptx: the full loop works.
   The earlier xlsx gap (discovery loop saw sheet structure only) was an upstream
   `filetools` bug, fixed in `v0.1.7` ("reach full cell/text content via
   scan/grep/read for all formats"). Re-verified against the v0.1.7 binary on
   hand-built OOXML fixtures for both xlsx (cell values) and pptx (slide text);
   the `DocGrep`/`DocPeek` xlsx caveat guidelines were dropped accordingly.
4. ~~Exact stdout JSON schemas for `scan` and `read`?~~ **Resolved:** captured as
   `ScanView` / `GrepView` / `ReadView` in `filetools-shared.ts` and exercised by
   `test/filetools.test.ts` against the real binary.
