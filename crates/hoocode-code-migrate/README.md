# hoocode-code-migrate

The one-time merge that moves data from the old `~/.cortexcode` and `<repo>/.cortexcode/`
folders into `~/.hoocode` and `<repo>/.hoocode/` (naming-and-paths.md §3).

- Copies, never moves or deletes. `.cortexcode` wins a conflict; each overwritten file is
  backed up first into `backup-cortexcode-<timestamp>/`.
- Runs at every start (`hoocode migrate [--dry-run]` runs it on demand). A marker,
  `.merged-from-cortexcode`, stops a second merge; a `.merge.lock` directory stops two
  processes merging at once.
- This crate is the only code that names the old folders; `scripts/ci/no_cortex.sh`
  allows it.
