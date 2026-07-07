/**
 * Canonical set of AgentSession event `type`s that a spawned subagent writes to
 * its stdout JSON stream, shared so the child emitter (print-mode) and the
 * parent consumer (SubagentPool) cannot drift apart.
 *
 * A subagent child otherwise emits the full per-delta event firehose
 * (`message_update`, `tool_execution_update`, `message_start`, …). The parent
 * only ever consumes:
 *   - the progress events below, forwarded to the UI as `task_progress`, and
 *   - `message_end`, whose usage the parent's TokenBudget accumulates.
 * Everything else is dropped at the source so the firehose never crosses the
 * pipe (and is never framed/parsed on the parent's UI event loop). The child's
 * authoritative result travels via `result.json`, not this stream, and liveness
 * travels via a separate periodic `ping`, so this filtering is lossless for the
 * parent.
 */
export const SUBAGENT_PROGRESS_EVENTS: ReadonlySet<string> = new Set([
	"turn_end",
	"tool_execution_start",
	"tool_execution_end",
]);

/** Superset the child actually writes: progress events plus `message_end` (token
 * usage for the parent budget). */
export const SUBAGENT_STDOUT_EVENT_TYPES: ReadonlySet<string> = new Set([...SUBAGENT_PROGRESS_EVENTS, "message_end"]);
