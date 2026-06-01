# Subagent Delegation

HooCode delegates focused work to specialized **subagents** that run in isolated
child processes. Delegation is **description-driven**: the parent agent decides
when to delegate and which agent to use, based on each agent's `description`.
There is no deterministic keyword router and no blocking dispatch gate.

## How Delegation Works

The parent agent has a `Task` tool whose description lists the available agents
(see the registry below). The model chooses an agent via `subagent_type` and
passes a self-contained `prompt`. The subagent runs in a fresh context and only
its final answer is returned to the parent.

Guidance baked into the parent prompt:

- Delegate self-contained units where you only need the final result.
- Prefer handling small, quick, or single-file work inline.
- The subagent cannot see the parent conversation — pass all needed context in
  `prompt`.

The `DispatchEvaluator` no longer routes. It survives only to:

1. **Block nested delegation** — a subagent (`HOOCODE_SUBAGENT_DEPTH>=1`) must not
   spawn further subagents.
2. **Record a complexity estimate** in `.hoocode/dispatch/<task_id>/dispatch-log.json`
   for diagnostics. This is a cheap heuristic, not a routing decision.

## Available Agents

Agents are defined by frontmatter `.md` files loaded from a registry with
precedence **project > user > built-in**:

- Built-in: `templates/agents/*.md` (explore, edit, test, review, doc)
- Project: `.hoocode/agents/`, and `.claude/agents/` (Claude Code compatible)
- User: `~/.hoocode/agents/`, and `~/.claude/agents/`

Each definition supplies `name`, `description`, and optional `tools`, `model`,
`maxTurns`, and `background`. When a definition omits `tools`, built-in modes
fall back to a per-mode allowlist (`MODE_TOOLS`) so read-only modes (`explore`,
`test`, `review`) cannot edit or write files.

## Forcing a Subagent with `/subagent`

In interactive mode you can dispatch a subagent directly, bypassing the model's
decision (the mode is still validated against the registry):

```
/subagent explore "How does the auth middleware work?"
/subagent test "Run the parser unit tests"
/subagent review "Audit login.ts for security issues"
/subagent doc "Write a README for the API package"
```

## Execution Model

Subagents run as isolated `hoocode` **child processes**, managed by `SubagentPool`.
Each delegation:

1. Spawns `hoocode --mode json --session <file> --task-id <id> [--system-prompt <prompt>] [--tools <allowlist>] --max-turns <n> <prompt>` (re-running the current runtime/entry, so it works from `dist/`, from source via tsx, or as a packaged binary).
2. Runs under a hard turn cap (`--max-turns`, default 50). Near the cap the agent is asked to wrap up; at the cap it stops and returns a `partial` result instead of failing.
3. Emits a periodic `{"ping":true}` heartbeat on stdout; the lifeguard SIGKILLs a child that goes silent for 60s and enforces a per-mode hard timeout.
4. On exit, writes `.hoocode/dispatch/<task_id>/result.json` (summary, files_changed, confidence, status, usage), which the parent verifies before accepting the result.

The tool returns only the subagent's `summary` to the calling agent.

## Background and Resume

- An agent definition can set `background: true`. The `Task` tool then dispatches it detached and returns a `task_id` immediately. The `TaskOutput` tool polls by `task_id` and collects the final answer once finished.
- Subagents persist their session to `.hoocode/dispatch/<task_id>/session.jsonl`. Pass `resume_task_id` to the `Task` tool to continue a previous run with a follow-up `prompt` (full prior transcript intact). Partial results surface their resume handle.

Concurrency is bounded (default 5). Parallelism happens when the model issues
multiple `Task` calls; there is no batch-dispatch API.

## Guardrails

- **Token budget is advisory.** It emits `budget_warning` (80%) and `budget_exceeded` (100%) for telemetry but never kills or fails a subagent; the turn cap is the guaranteed hard stop.
- **No nested delegation.** The `Task`/`TaskOutput` tools are never registered inside a spawned subagent (`--task-id` present), and `HOOCODE_SUBAGENT_DEPTH=1` is set in each child's environment as defense in depth.
- The pool prioritizes `explore` and `review` tasks over `doc` tasks because they often block downstream work.
- On completion, the subagent writes `.hoocode/dispatch/<task_id>/result.json` (verified by the parent) and the pool writes `.hoocode/dispatch/<task_id>/output.json` (raw process outcome).
