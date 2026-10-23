# hoocode-code-session

Session handling for the hoocode coding agent

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

Session CRUD, directory layout, and lifecycle. Sessions are stored as append-only `.jsonl`
files where each line is a typed entry. Entries form a tree via `id`/`parent_id`, and the
leaf pointer tracks the current branch.
