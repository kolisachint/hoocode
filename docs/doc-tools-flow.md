# Document Tools Flow: scan → grep → peek → edit

The document tools losslessly project structured and binary documents — XML,
drawio, OOXML (docx/xlsx/pptx), and PDF — into editable, id-addressed JSON. They
are off by default; enable them with `--enable-filetools` or the
`enableFileTools` setting. There are two groups:

- **Discovery (cheap, read-only):** `DocScan` (outline), `DocGrep` (search by
  text), `DocPeek` (hydrate specific blocks).
- **Read + edit:** `DocRead` (full extract / editable id-map), `DocEdit` (patch
  in place), `DocWrite` (patch to a new path).

This page describes the recommended **ordering**. The same guidance is embedded
in each tool's prompt so the agent follows it at runtime.

## The flow

```
  outline ──▶ DocScan            cheap manifest: block path-ids + previews
                 │  (page with offset/limit)
                 ▼
  search  ──▶ DocGrep "text"     blocks by literal text → editable el_ #ids
                 │
        ┌────────┴───────────────────────────┐
        ▼                                     ▼
  read ─▶ DocPeek node[..]            edit ─▶ DocEdit  /structure/<el_ id>/...
        │  hydrate just those blocks         │  (DocWrite to save-as)
        ▼                                     ▼
   nodes carry el_ #ids ──────────────▶ patch them with DocEdit/DocWrite
```

`DocRead` (a full writable extract) is **token-heavy** — reach for it only when
you need the whole editable tree at once. For everything else, the discovery
loop is far cheaper and can reach any block regardless of document size.

### 1. Outline — `DocScan`

Start here for any large document. `DocScan` returns a paginated manifest of
blocks — each with a **structural-path id** (`node[title:0]`, `paragraph[3]`,
`sheet[0].rows[0-99]`), a type, a section label, a token estimate, and a short
preview — without hydrating content. Page through with `offset`/`limit`.

### 2. Search — `DocGrep`

`DocGrep` finds blocks by **literal** substring (pass `ignoreCase` for
case-insensitive; it is not a regex) and returns each hit as an **editable `el_`
node `#id`**, line, and snippet. Those `el_` ids target a `DocEdit` patch
directly — `DocGrep` is the fast path from "find text" to "edit it."

### 3. Read a portion — `DocPeek`

`DocPeek` hydrates only the blocks you ask for, by their **`DocScan` path id**
(`id: ["node[title:0]"]`), or pages through all blocks with `offset`/`limit`. The
hydrated nodes carry the editable `el_` `#ids` you then patch.

> **Two id schemes — route them correctly.** `DocScan` emits *path* ids
> (`node[title:0]`) → use with `DocPeek`. `DocGrep` emits *`el_`* node ids → use
> with `DocEdit`. `DocPeek` accepts **only** path ids (it rejects `el_` ids). To
> edit a block you found with `DocScan`, hydrate it with `DocPeek` first — the
> returned nodes carry the `el_` ids for `DocEdit`.

### 4. Edit — `DocEdit` / `DocWrite`, minimal patches, no re-reading

- `DocEdit` patches **in place** and re-extracts afterward.
- `DocWrite` reconstructs to a **new path**, leaving the source untouched.

Both take an id-based patch (RFC-6902 `replace`/`add`/`remove`) targeting `el_`
`#ids`. Keep patches **minimal** — only the ops you need; never rewrite the whole
document. You do **not** need a prior `DocRead`: `DocEdit`/`DocWrite`
auto-extract (the TS wrapper runs `extract` for you) when the cache is missing or
the file changed on disk. If a patch targets ids that no longer exist, the call
fails and returns the **current id-addressed structure** so you can re-issue
without a separate `DocRead`. So **do not re-run `DocRead`/`DocScan` between
edits.**

### When to use a full `DocRead`

Use `DocRead` (full writable extract) when you genuinely need the entire editable
tree in one view, or `DocRead readonly:true` for a small analysis-only projection
(strips ids; cannot edit against it). For large documents prefer the discovery
loop — a full `DocRead` truncates at a token budget and cannot reach past it,
whereas `DocScan`/`DocPeek` can page to any block.

## Never fall back to ad-hoc scripts

These tools are the canonical way to read and edit these formats. Do **not** use
ad-hoc scripts (`python`/`openpyxl`, `docx`, `PyPDF2`, `unzip`, `sed`) to parse
or rewrite them — that bypasses the lossless id-map and corrupts formatting.

## Quick reference

| Step     | Call                              | Cost        | Id in → out                          |
|----------|-----------------------------------|-------------|--------------------------------------|
| Outline  | `DocScan` (path, offset?, limit?) | cheap       | → path ids (+ previews)              |
| Search   | `DocGrep` (path, pattern)         | cheap       | text → `el_` ids (+ snippets)        |
| Read     | `DocPeek` (path, id?/offset/limit)| cheap       | path id → hydrated nodes w/ `el_` ids |
| Full read| `DocRead` (path, readonly?)       | token-heavy | → whole editable tree (`el_` ids)    |
| Edit     | `DocEdit` (path, patch)           | —           | `el_` id → in-place patch            |
| Write    | `DocWrite` (path, out, patch)     | —           | `el_` id → save-as (source untouched) |
