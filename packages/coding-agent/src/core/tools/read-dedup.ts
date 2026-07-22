/**
 * Read de-duplication primitives.
 *
 * Two mechanisms share this module:
 *
 * - The post-hoc context GC (`context-gc.ts`), which stubs out a `read` result
 *   once a later overlapping read or an edit/write has superseded it.
 * - The at-call-time guard in the `read` tool, which short-circuits a read whose
 *   requested range is *already fully covered* by an earlier, still-live read in
 *   the current session, returning a pointer instead of re-fetching the file.
 *
 * Both reason about half-open line ranges `[start, end)` (end exclusive), so the
 * range math lives here and cannot drift between them. The guard additionally
 * needs to (a) recognise its own pointer results so they are never treated as
 * content-bearing reads, and (b) walk the session branch to find a covering
 * read — both of which are defined here to keep the tool file lean.
 */

/** Half-open line interval `[start, end)` a read call covers; end is exclusive. */
export interface ReadRange {
	start: number;
	end: number;
}

/** A read with no offset/limit covers the whole file (open-ended). */
export const WHOLE_FILE_RANGE: ReadRange = { start: 1, end: Number.POSITIVE_INFINITY };

/**
 * Derive the line range a read call covers from its `offset`/`limit` args.
 * Missing offset means "from line 1"; missing limit means "to end of file"
 * (open-ended, so it overlaps any later read of the same file).
 */
export function readRangeFromArgs(args: Record<string, unknown> | undefined): ReadRange {
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
export function rangesOverlap(a: ReadRange, b: ReadRange): boolean {
	return a.start < b.end && b.start < a.end;
}

/** Whether `outer` fully contains `inner` (every line of inner lies within outer). */
export function rangeContains(outer: ReadRange, inner: ReadRange): boolean {
	return outer.start <= inner.start && outer.end >= inner.end;
}

/**
 * Marker prefix for the at-call dedup pointer. Kept stable so the context GC can
 * recognise a pointer result and exclude it from supersession bookkeeping (a
 * pointer fetched no content, so it must not stub the read it points at).
 */
export const DEDUP_POINTER_PREFIX = "[Already in context:";

/** Whether a tool-result text is an at-call dedup pointer. */
export function isDedupPointerText(text: string): boolean {
	return text.trimStart().startsWith(DEDUP_POINTER_PREFIX);
}

/** A covering earlier read, described for the pointer message. */
export interface CoveringRead {
	/** The path as the earlier read spelled it (for a friendly pointer). */
	display: string;
	/** Delivered range start (1-indexed line). */
	start: number;
	/** Delivered range end (exclusive; Infinity for a whole-file read). */
	end: number;
}

export interface FindCoveringReadOptions {
	/** Resolved absolute path of the current read. */
	resolvedPath: string;
	/** Range the current read is asking for. */
	requestedRange: ReadRange;
	/** Tool call id of the current read, excluded from candidate/supersession sets. */
	currentCallId: string;
	/** Resolve a raw read-arg path the same way the current read resolved its path. */
	resolvePath: (rawPath: string) => string;
}

interface ContentBlock {
	type?: string;
	id?: string;
	name?: string;
	arguments?: Record<string, unknown>;
	text?: string;
}

interface MessageLike {
	role?: string;
	content?: ContentBlock[];
	toolCallId?: string;
	toolName?: string;
	isError?: boolean;
}

const READ_TOOL = "read";
const MUTATE_TOOLS = new Set(["edit", "write"]);

/** Accept either raw `AgentMessage`s or session entries wrapping `.message`. */
function toMessage(entry: unknown): MessageLike | null {
	if (!entry || typeof entry !== "object") return null;
	const e = entry as { message?: unknown; role?: unknown };
	if (e.message && typeof e.message === "object") return e.message as MessageLike;
	if (typeof e.role === "string") return e as MessageLike;
	return null;
}

function resultText(m: MessageLike): string {
	return (m.content ?? [])
		.filter((c) => c.type === "text")
		.map((c) => c.text ?? "")
		.join("");
}

/**
 * Recover the range a read result actually *delivered* from its text.
 *
 * A cap-truncated read announces `[Showing lines A-B of N ...]`, so it delivered
 * only `[A, B+1)` even though its args declared a wider range. A read whose first
 * line alone exceeded the byte cap delivered nothing. Any other (untruncated)
 * read delivered its full declared range. The user-`limit` early-stop notice
 * (`[N more lines in file ...]`) is *not* a cap truncation — the declared range
 * was delivered in full — so it falls through to `declared`.
 */
function deliveredRange(text: string, declared: ReadRange): ReadRange {
	const showing = text.match(/\[Showing lines (\d+)-(\d+) of \d+/);
	if (showing) {
		const a = Number(showing[1]);
		const b = Number(showing[2]);
		if (Number.isFinite(a) && Number.isFinite(b) && b >= a) return { start: a, end: b + 1 };
	}
	// First line exceeded the byte limit: nothing usable was delivered.
	if (/\[Line \d+ is .+ exceeds .+ limit\./.test(text)) return { start: 1, end: 1 };
	return declared;
}

/**
 * Find the latest earlier read that (a) is for the same resolved path, (b)
 * actually delivered a range containing the requested range, and (c) is still
 * live in the outgoing context — i.e. the post-hoc GC will not have stubbed it,
 * because no later edit/write and no later overlapping content read supersede
 * it. Returns that read's delivered range for the pointer, or null when the
 * current read must actually run.
 *
 * Deliberately conservative: a truncated earlier read only covers what it
 * delivered, a whole-file read must have been delivered untruncated to count as
 * covering, and any pointer results (which fetched nothing) are ignored on both
 * the candidate and the supersession side.
 */
export function findCoveringRead(entries: readonly unknown[], opts: FindCoveringReadOptions): CoveringRead | null {
	const messages: MessageLike[] = [];
	for (const e of entries) {
		const m = toMessage(e);
		if (m) messages.push(m);
	}

	// Pass 1: map each tool call id to the path/range it operated on.
	const readCall = new Map<string, { resolved: string; display: string; declared: ReadRange }>();
	const mutateCallPath = new Map<string, string>();
	for (const m of messages) {
		if (m.role !== "assistant" || !Array.isArray(m.content)) continue;
		for (const b of m.content) {
			if (b.type !== "toolCall" || !b.id) continue;
			const raw = b.arguments?.path;
			if (typeof raw !== "string" || raw.length === 0) continue;
			const resolved = opts.resolvePath(raw);
			if (b.name === READ_TOOL) {
				readCall.set(b.id, { resolved, display: raw, declared: readRangeFromArgs(b.arguments) });
			} else if (b.name && MUTATE_TOOLS.has(b.name)) {
				mutateCallPath.set(b.id, resolved);
			}
		}
	}

	// Pass 2: collect successful results for the target path.
	interface PriorRead {
		index: number;
		delivered: ReadRange;
		display: string;
	}
	const priorReads: PriorRead[] = [];
	// Every content-bearing read of the target path (for the supersession check),
	// mirroring what the GC counts when it decides to stub an earlier read.
	const contentReads: Array<{ index: number; range: ReadRange }> = [];
	let lastMutateIndex = -1;
	messages.forEach((m, i) => {
		if (m.role !== "toolResult" || m.isError || !m.toolCallId) return;
		if (m.toolName === READ_TOOL) {
			const info = readCall.get(m.toolCallId);
			if (!info || info.resolved !== opts.resolvedPath) return;
			const text = resultText(m);
			if (isDedupPointerText(text)) return; // a pointer fetched nothing: not content, never supersedes
			const delivered = deliveredRange(text, info.declared);
			const hasContent = delivered.end > delivered.start;
			if (hasContent) contentReads.push({ index: i, range: delivered });
			if (m.toolCallId === opts.currentCallId || !hasContent) return;
			priorReads.push({ index: i, delivered, display: info.display });
		} else if (m.toolName && MUTATE_TOOLS.has(m.toolName)) {
			if (mutateCallPath.get(m.toolCallId) === opts.resolvedPath) lastMutateIndex = Math.max(lastMutateIndex, i);
		}
	});

	let best: PriorRead | null = null;
	for (const r of priorReads) {
		if (!rangeContains(r.delivered, opts.requestedRange)) continue;
		if (lastMutateIndex > r.index) continue; // file changed after this read
		const supersededByRead = contentReads.some((o) => o.index > r.index && rangesOverlap(o.range, r.delivered));
		if (supersededByRead) continue; // GC will have stubbed this read
		if (!best || r.index > best.index) best = r;
	}
	if (!best) return null;
	return { display: best.display, start: best.delivered.start, end: best.delivered.end };
}

/** Build the pointer text returned in place of a re-fetch. */
export function buildDedupPointerText(covering: CoveringRead): string {
	const where =
		covering.end === Number.POSITIVE_INFINITY
			? "the entire file"
			: covering.end - 1 > covering.start
				? `lines ${covering.start}-${covering.end - 1}`
				: `line ${covering.start}`;
	return `${DEDUP_POINTER_PREFIX} ${covering.display} (${where}) was already read earlier in this session and has not changed since. Not re-fetched to save tokens — pass a different offset/limit, or edit the file, if you need other or newer content.]`;
}
