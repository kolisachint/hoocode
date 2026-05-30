# Subagent Routing

HooCode uses a deterministic **dispatch evaluator** to decide whether a task should be handled inline by the main agent or delegated to a specialized subagent. No LLM call is made for this decision — it is purely keyword + heuristic based.

## How Routing Decisions Work

When the parent agent receives a task (via the `subagent` tool or a `/subagent` slash command), the `DispatchEvaluator` scans the task for keywords and estimates complexity:

- **Low complexity** (< 50 lines, 1 file, read-only or trivial edit) → handle **inline**
- **Medium complexity** (2–3 files, 50–200 lines) → delegate to a **single matching agent**
- **High complexity** (4+ files, 200+ lines, multi-domain) → **split** across multiple subagents

Keyword routing:

| Keywords | Agent type |
|----------|------------|
| explore, understand, scout, investigate, trace, find | `explore` |
| write, create, implement, refactor, add, build | `edit` |
| test, validate, assert, coverage | `test` |
| review, audit, critique, security, check | `review` |
| document, readme, comment, explain, docs | `doc` |

## When to Expect Inline vs Subagent

- **Inline** — Simple single-file changes, read-only lookups, trivial edits.
- **Subagent** — Anything that spans multiple files, requires isolation, or matches a specialized mode (testing, review, documentation).

If you disagree with the evaluator, you can force a subagent by setting `force=true` in the tool call or using the `/subagent <mode> <task>` slash command.

## How to Force a Subagent with `/subagent`

In interactive mode, type:

```
/subagent <mode> <task>
```

Examples:

```
/subagent explore "How does the auth middleware work?"
/subagent test "Run the parser unit tests"
/subagent review "Audit login.ts for security issues"
/subagent doc "Write a README for the API package"
```

This bypasses the evaluator and spawns the subagent directly.

## Execution Model

Subagents run as isolated `hoocode` **child processes**, managed by `SubagentPool`. Each delegation:

1. Spawns `hoocode --mode json --no-session --task-id <id> --system-prompt <mode prompt> --tools <mode allowlist>` (re-running the current runtime/entry, so it works from `dist/`, from source via tsx, or as a packaged binary).
2. Runs with a per-mode tool allowlist so read-only modes (`explore`, `review`) cannot edit or write files.
3. Emits a periodic `{"ping":true}` heartbeat on stdout; the lifeguard SIGKILLs a child that goes silent for 60s, and enforces a per-mode hard timeout.
4. On exit, writes `.hoocode/agents/<task_id>/result.json` (summary, files_changed, confidence, status, usage), which the parent verifies before accepting the result.

The tool returns only the subagent's `summary` to the calling agent.

## Parallel Batches

For multi-domain tasks (e.g. "Implement X, write tests, and review"), the evaluator sets `parallelizable: true` and produces a list of subtasks. The `SubagentPool.dispatchBatch()` method spawns up to 5 concurrent subagents and returns aggregated results.

## Guardrails

- Subagents **cannot spawn subagents**. The pool sets `HOOCODE_SUBAGENT_DEPTH=1` in each child's environment, so a nested delegation's evaluator returns `should_delegate: false`.
- The pool prioritizes `explore` and `review` tasks over `doc` tasks because they often block downstream work.
- On completion, each subagent writes `.hoocode/agents/<task_id>/result.json` (verified by the parent) and the pool writes `.hoocode/agents/<task_id>/output.json` (raw process outcome).
