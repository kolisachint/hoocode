# hoocode-code-tools-fs

File tools for the hoocode coding agent.

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

- `read` (`core/tools/read.ts`): text files byte-for-byte with `offset`/`limit` paging and
  the 800-line / 32KB caps (continuation notices say where to resume), images as attachments
  (resized through `hoocode-code-media`), a note instead of bytes for docx/xlsx/pptx/pdf,
  and Node-style fs errors. `ReadOperations` lets the file access be swapped (e.g. SSH).
- `read_dedup` (`core/tools/read-dedup.ts`): a re-read whose range an earlier, still-live read
  already covers returns an `[Already in context: ...]` pointer. The file's sha1 must still
  match the read being pointed at.

`edit`/`write` and the file mutation queue arrive with ledger task 10.2c.
