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
 *   over an overlapping line range (the newer read reflects that region's newer
 *   state). Reads of disjoint regions of the same file coexist — a later read of
 *   lines 200-260 does not evict an earlier read of lines 1-40 — so paginating
 *   through a large file never makes the model re-fetch a region it already has.
 *   A read is only evicted when a *successful* later event supersedes it — so a
 *   read whose edit failed (and which the model still needs to retry) is never
 *   touched.
 *
 * Deliberately conservative: paths are matched after `path.resolve`, so two
 * different files never collide, and any ambiguity results in NOT evicting.
 *
 * Bash-output eviction is additionally gated on token-budget pressure
 * (`options.budgetPressure`, the fraction of the model's context window in
 * use). At 0 pressure — the default — behaviour is identical to read-only GC.
 * Bash output is not always recoverable, so it carries its own safeguards: a
 * command that looks side-effecting is never elided (re-running it, as the
 * stub invites, could repeat a destructive action), and below 80% pressure
 * only large outputs are elided.
 */

import { resolve } from "node:path";
import type { AgentMessage } from "@kolisachint/hoocode-agent-core";

/** Tool names whose result injects file contents into the transcript. */
const READ_TOOLS = new Set(["read"]);
/** Tool names that change a file, making a prior read of that path stale. */
const MUTATE_TOOLS = new Set(["edit", "write"]);

/**
 * Commands whose bash output must never be elided, because the stub invites
 * the model to re-run the command and re-running could repeat a destructive or
 * state-changing action. Tested against the whole command string, and
 * deliberately over-broad: a false positive merely keeps an output (safe),
 * while the common verbose *read* commands (cat/grep/ls/find/git log/diff,
 * test runners) intentionally do NOT match, so they stay evictable.
 */
const BASH_SIDE_EFFECT_PATTERN =
	/(\b(write|insert|delete|update|curl|wget|psql|mysql|sqlite3|migrate|drop|truncate|rm|rmdir|mv|cp|dd|kill|tee|chmod|chown|ln|mkdir|touch)\b|\bgit\s+(commit|push|reset|checkout|rebase|clean|apply|merge)\b|\bsed\b[^|]*-i|>>?)/i;
/** Below 80% pressure, only bash outputs larger than this (chars) are elided. */
const BASH_EVICTION_CHAR_THRESHOLD = 2000;

export interface ContextGcOptions {
	/** Working directory used to resolve relative tool path arguments. */
	cwd: string;
	/**
	 * Token-budget pressure in [0, 1] — the fraction of the model's context
	 * window currently in use. Absent or 0 reproduces read-only GC behaviour.
	 * Bash-output eviction begins at 0.6 and becomes unconditional at 0.8.
	 */
	budgetPressure?: number;
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

/** Half-open line interval [start, end) a read call covers; end is exclusive. */
interface ReadRange {
	start: number;
	end: number;
}

/**
 * Derive the line range a read call covers from its `offset`/`limit` args.
 * Missing offset means "from line 1"; missing limit means "to end of file"
 * (open-ended, so it overlaps any later read of the same file).
 */
function readRangeFromArgs(args: Record<string, unknown> | undefined): ReadRange {
	const offsetRaw = args?.offset;
	const limitRaw = args?.limit;
	const offset = typeof offsetRaw === "number" && Number.isFinite(offsetRaw) && offsetRaw > 0 ? offsetRaw : 1;
	const end =
		typeof limitRaw === "number" && Number.isFinite(limitRaw)
			? offset + Math.max(0, limitRaw)
			: Number.POSITIVE_INFINITY;
	return { start: offset, end };
}

/** Whether two half-open line ranges intersect. */
function rangesOverlap(a: ReadRange, b: ReadRange): boolean {
	return a.start < b.end && b.start < a.end;
}

const WHOLE_FILE_RANGE: ReadRange = { start: 1, end: Number.POSITIVE_INFINITY };

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
	// Bash calls carry a `command`, not a `path`; capture it so the eviction
	// pass can consult the side-effect guard by tool call id.
	const bashCommandByCallId = new Map<string, string>();
	// Read calls carry `offset`/`limit`; capture the line range each covers so a
	// later read only supersedes an earlier one when their ranges overlap.
	const rangeByCallId = new Map<string, ReadRange>();
	for (const m of messages) {
		if (!isAssistant(m)) continue;
		for (const block of m.content) {
			if (block.type !== "toolCall" || !block.id) continue;
			if (block.name === "bash") {
				const cmd = block.arguments?.command;
				if (typeof cmd === "string" && cmd.length > 0) bashCommandByCallId.set(block.id, cmd);
			}
			const rawPath = block.arguments?.path;
			if (typeof rawPath !== "string" || rawPath.length === 0) continue;
			const resolved = resolve(options.cwd, rawPath);
			resolvedPathByCallId.set(block.id, resolved);
			if (block.name && READ_TOOLS.has(block.name)) rangeByCallId.set(block.id, readRangeFromArgs(block.arguments));
			if (!displayPathByResolved.has(resolved)) displayPathByResolved.set(resolved, rawPath);
		}
	}

	// First pass: per path, collect every successful read (index + line range)
	// and the last successful mutate. A read at index i is superseded when a later
	// successful mutate exists (index > i, whole file changed) or a later read of
	// an overlapping range exists (index > i) for the same path.
	const readsByPath = new Map<string, Array<{ index: number; range: ReadRange }>>();
	const lastMutateIndex = new Map<string, number>();
	messages.forEach((m, i) => {
		if (!isToolResult(m) || m.isError) return;
		const path = resolvedPathByCallId.get(m.toolCallId);
		if (!path) return;
		if (READ_TOOLS.has(m.toolName)) {
			const range = rangeByCallId.get(m.toolCallId) ?? WHOLE_FILE_RANGE;
			const list = readsByPath.get(path);
			if (list) list.push({ index: i, range });
			else readsByPath.set(path, [{ index: i, range }]);
		} else if (MUTATE_TOOLS.has(m.toolName)) {
			lastMutateIndex.set(path, i);
		}
	});

	// Second pass: build the output, stubbing evicted reads. Keep every other
	// message by reference; clone only the ones we rewrite so the persisted
	// history (which may share these objects) is never mutated.
	const pressure = options.budgetPressure ?? 0;
	let changed = false;
	const out = messages.map((m, i) => {
		if (!isToolResult(m) || m.isError) return m;

		// Superseded-read eviction — always on, pressure-independent.
		if (READ_TOOLS.has(m.toolName)) {
			const path = resolvedPathByCallId.get(m.toolCallId);
			if (!path) return m;
			const range = rangeByCallId.get(m.toolCallId) ?? WHOLE_FILE_RANGE;
			const laterMutate = (lastMutateIndex.get(path) ?? -1) > i;
			const laterOverlappingRead = (readsByPath.get(path) ?? []).some(
				(r) => r.index > i && rangesOverlap(r.range, range),
			);
			if (!laterOverlappingRead && !laterMutate) return m;
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
		}

		// Bash-output eviction — only under token-budget pressure (>= 0.6).
		if (pressure >= 0.6 && m.toolName === "bash") {
			const cmd = bashCommandByCallId.get(m.toolCallId) ?? "";
			if (BASH_SIDE_EFFECT_PATTERN.test(cmd)) return m;
			const textLen = m.content.filter((c) => c.type === "text").reduce((sum, c) => sum + (c.text?.length ?? 0), 0);
			// 60–79%: elide only large outputs. 80%+: elide unconditionally.
			const shouldEvict = pressure >= 0.8 || textLen > BASH_EVICTION_CHAR_THRESHOLD;
			if (shouldEvict) {
				changed = true;
				return {
					...m,
					content: [
						{
							type: "text" as const,
							text: `[Bash output elided at ${Math.round(pressure * 100)}% token budget — re-run if needed.]`,
						},
					],
				};
			}
		}

		return m;
	});

	return changed ? out : messages;
}
