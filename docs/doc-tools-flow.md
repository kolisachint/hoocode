# Document Tools Flow: DocRead → DocEdit / DocWrite

The document tools (`DocRead`, `DocEdit`, `DocWrite`) losslessly project
structured and binary documents — XML, drawio, OOXML (docx/xlsx/pptx), and PDF —
into editable, id-addressed JSON. They are off by default; enable them with
`--enable-filetools` or the `enableFileTools` setting.

This page describes the recommended **ordering** for using them. The same
guidance is embedded in each tool's prompt so the agent follows it at runtime.

## The flow

```
                ┌─────────────────────────────┐
  glimpse  ───▶ │ DocRead readonly:true        │  cheap, analysis-only, no ids
                └──────────────┬──────────────┘
                               │  decided to change something?
                               ▼
  read     ───▶ │ DocRead  (writable, default) │  token-heavy: carries the id-map
                └──────────────┬──────────────┘
                               │
            ┌──────────────────┴──────────────────┐
            ▼                                      ▼
  edit ─▶ DocEdit (in place)            write ─▶ DocWrite (save-as, new path)
            │  minimal id-based patch              │  minimal id-based patch
            ▼                                      ▼
       re-extracts automatically             source left untouched
```

### 1. Scan first — `DocRead readonly:true`

Start with a **glimpse**. `readonly:true` returns an analysis-only projection
that strips node ids, so it is much smaller and cheaper in tokens. Use it to
understand a document's structure and content before deciding whether — and
what — to change. You cannot edit against a readonly view (it has no ids); that
is intentional.

### 2. Read for edit only when you commit to changing it

A full, writable `DocRead` (the default, `readonly` omitted) carries the
**complete id-map** needed to patch the document. That map is what makes
`DocRead` **token-heavy**, so treat it as the expensive step:

- Do it **only when you actually intend to edit**, not just to look.
- Do it **once**. The id-map is established by this extract.

### 3. Edit or write — minimal patches, no re-reading

- `DocEdit` patches the document **in place** and re-extracts afterward.
- `DocWrite` reconstructs a patched document to a **new path**, leaving the
  source untouched (save-as).

Both take an id-based patch (RFC-6902 `replace`/`add`/`remove`) targeting the
`#ids` from the extract. Keep patches **minimal** — only the ops you actually
need; never rewrite the whole document.

Crucially, **do not re-run `DocRead` between edits**:

- `DocEdit`/`DocWrite` re-extract on their own when the cache is missing or the
  file changed on disk, so a fresh `DocRead` is not required to keep editing.
- If a patch targets ids that no longer exist (the document was rewritten
  out-of-band), the call fails and returns the **current id-addressed
  structure** in the error, so you can re-issue the patch without a separate
  `DocRead`.

Reach for a new writable `DocRead` only when ids have genuinely gone stale and
the failure output isn't enough to continue.

## Never fall back to ad-hoc scripts

`DocRead`/`DocEdit`/`DocWrite` are the canonical way to read and edit these
formats. Do **not** use ad-hoc scripts (`python`/`openpyxl`, `docx`, `PyPDF2`,
`unzip`, `sed`) to parse or rewrite them — that bypasses the lossless id-map and
corrupts formatting.

## Quick reference

| Step    | Call                       | Cost        | Notes                                            |
|---------|----------------------------|-------------|--------------------------------------------------|
| Glimpse | `DocRead readonly:true`    | cheap       | Analysis-only; strips ids; cannot edit against it |
| Read    | `DocRead` (default)        | token-heavy | Establishes the id-map; do once, only to edit     |
| Edit    | `DocEdit` (path, patch)    | —           | In place; re-extracts; keep patch minimal         |
| Write   | `DocWrite` (path, out, patch) | —        | Save-as to a new path; source untouched           |
