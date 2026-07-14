/**
 * Context garbage collection.
 *
 * The whole transcript is re-sent on every turn, so a tool result that has
 * become useless keeps costing tokens for the rest of the session. This pass
 * runs on the OUTGOING message copy (via the agent's `transformContext` hook),
 * never on the persisted session, and replaces provably-dead `read` results
 * with a short stub. Nothing is lost: the persisted history is untouched and
 * the file is re-readable on demand.
 *
 * Current rule — superseded reads only (the read-then-edit / re-read pattern,
 * which dominates coding sessions):
 *
 *   A `read` result for a path P is stale once, later in the transcript, the
 *   same path is edited/written (its on-disk content changed) or read again
 *   (the newer read reflects newer state). The most recent read of each path is
 *   always kept, and a read is only evicted when a *successful* later event
 *   supersedes it — so a read whose edit failed (and which the model still needs
 *   to retry) is never touched.
 *
 * Deliberately conservative: paths are matched after `path.resolve`, so two
 * different files never collide, and any ambiguity results in NOT evicting.
 * Bash-output eviction is intentionally left out for now — that output is not
 * always recoverable, so it needs its own safeguards.
 */

import { resolve } from "node:path";
import type { AgentMessage } from "@kolisachint/hoocode-agent-core";

/** Tool names whose result injects file contents into the transcript. */
const READ_TOOLS = new Set(["read"]);
/** Tool names that change a file, making a prior read of that path stale. */
const MUTATE_TOOLS = new Set(["edit", "write"]);

export interface ContextGcOptions {
	/** Working directory used to resolve relative tool path arguments. */
	cwd: string;
}

interface AssistantLike {
	role: "assistant";
	content: Array<{ type: string; id?: string; name?: string; arguments?: Record<string, unknown> }>;
}

interface ToolResultLike {
	role: "toolResult";
	toolCallId: string;
	toolName: string;
	content: Array<{ type: string; text?: string }>;
	isError: boolean;
}

function isAssistant(m: AgentMessage): m is AgentMessage & AssistantLike {
	return (m as { role?: string }).role === "assistant" && Array.isArray((m as AssistantLike).content);
}

function isToolResult(m: AgentMessage): m is AgentMessage & ToolResultLike {
	return (m as { role?: string }).role === "toolResult";
}

/**
 * Return a message array with superseded `read` results stubbed out. Returns the
 * original array reference unchanged when there is nothing to evict, so a
 * no-op turn does not needlessly perturb the outgoing context.
 */
export function evictSupersededReads(messages: AgentMessage[], options: ContextGcOptions): AgentMessage[] {
	// Map each tool call id -> the resolved path it operated on, plus a friendly
	// display path (the original argument) for the stub text.
	const resolvedPathByCallId = new Map<string, string>();
	const displayPathByResolved = new Map<string, string>();
	for (const m of messages) {
		if (!isAssistant(m)) continue;
		for (const block of m.content) {
			if (block.type !== "toolCall" || !block.id) continue;
			const rawPath = block.arguments?.path;
			if (typeof rawPath !== "string" || rawPath.length === 0) continue;
			const resolved = resolve(options.cwd, rawPath);
			resolvedPathByCallId.set(block.id, resolved);
			if (!displayPathByResolved.has(resolved)) displayPathByResolved.set(resolved, rawPath);
		}
	}

	// First pass: per path, the index of the last read and the last successful
	// mutate. A read at index i is superseded when a later read (index > i) or a
	// later successful mutate (index > i) exists for the same path.
	const lastReadIndex = new Map<string, number>();
	const lastMutateIndex = new Map<string, number>();
	messages.forEach((m, i) => {
		if (!isToolResult(m) || m.isError) return;
		const path = resolvedPathByCallId.get(m.toolCallId);
		if (!path) return;
		if (READ_TOOLS.has(m.toolName)) {
			lastReadIndex.set(path, i);
		} else if (MUTATE_TOOLS.has(m.toolName)) {
			lastMutateIndex.set(path, i);
		}
	});

	// Second pass: build the output, stubbing evicted reads. Keep every other
	// message by reference; clone only the ones we rewrite so the persisted
	// history (which may share these objects) is never mutated.
	let changed = false;
	const out = messages.map((m, i) => {
		if (!isToolResult(m) || m.isError || !READ_TOOLS.has(m.toolName)) return m;
		const path = resolvedPathByCallId.get(m.toolCallId);
		if (!path) return m;
		const laterRead = (lastReadIndex.get(path) ?? -1) > i;
		const laterMutate = (lastMutateIndex.get(path) ?? -1) > i;
		if (!laterRead && !laterMutate) return m;
		changed = true;
		const display = displayPathByResolved.get(path) ?? path;
		const reason = laterMutate ? "the file was modified after this read" : "the file was read again later";
		return {
			...m,
			content: [
				{
					type: "text" as const,
					text: `[Superseded read of ${display} elided to save context — ${reason}. Re-read the file if you need its current contents.]`,
				},
			],
		};
	});

	return changed ? out : messages;
}
