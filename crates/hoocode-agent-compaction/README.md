# hoocode-agent-compaction

Session compaction for hoocode agents

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

Context compaction and branch summarization for long sessions. `prepare_compaction`
picks what to summarize from a session path, `compact` asks the model for the summary,
and the session layer writes the compaction entry. `generate_branch_summary` does the
same for a branch being left during tree navigation.
