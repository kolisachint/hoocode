# hoocode-code-subagents

Subagent orchestration for the hoocode coding agent

Part of the [hoocode](https://github.com/kolisachint/hoocode) Rust workspace.

Ports hoocode's subagent machinery: the child-process pool (`pool`: children run
`hoocode --mode json --task-id <id>`, a verified `result.json` settles them), the
lifeguard that reaps silent or overdue children, the depth guard and dispatch
evaluator, token budgets, `result.json` building and verification, and model
categories (`fast` / `standard` / `capable`).
