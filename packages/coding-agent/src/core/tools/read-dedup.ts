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

/** A session compaction entry — the boundary that trims the live context. */
function isCompactionEntry(entry: unknown): boolean {
	return !!entry && typeof entry === "object" && (entry as { type?: unknown }).type === "compaction";
}

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
	// The truncation notice is always the trailing `\n\n[Showing lines A-B of N ...]`
	// clause the read tool appends. Anchor to the end so a `[Showing lines ...]`
	// string that merely appears *inside* the file's content can't spoof it.
	const showing = text.match(/\n\n\[Showing lines (\d+)-(\d+) of \d+[^\]]*\]\s*$/);
	if (showing) {
		const a = Number(showing[1]);
		const b = Number(showing[2]);
		if (Number.isFinite(a) && Number.isFinite(b) && b >= a) return { start: a, end: b + 1 };
	}
	// First line alone exceeded the byte limit: the whole result *is* that notice,
	// so it must start the text. Nothing usable was delivered.
	if (/^\[Line \d+ is .+ exceeds .+ limit\./.test(text)) return { start: 1, end: 1 };
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
	// `declared` mirrors the range the GC uses for supersession (straight from the
	// call args); `delivered` is what the result text shows was actually returned,
	// used for coverage. The current call has no result yet, so it never appears.
	interface PriorRead {
		index: number;
		declared: ReadRange;
		delivered: ReadRange;
		display: string;
	}

	// Resolve each distinct raw path once — the resolver may hit the filesystem.
	const resolveCache = new Map<string, string>();
	const resolvePath = (raw: string): string => {
		const hit = resolveCache.get(raw);
		if (hit !== undefined) return hit;
		const resolved = opts.resolvePath(raw);
		resolveCache.set(raw, resolved);
		return resolved;
	};

	// Single ordered pass. `readCall`/`mutateCallPath` map a call id to its path as
	// the call is seen (a toolCall always precedes its result); `reads` collects
	// the target path's reads and `lastMutateIndex` its last mutate. `order` gives
	// live-context position for the supersession/mutate comparisons.
	const readCall = new Map<string, { resolved: string; display: string; declared: ReadRange }>();
	const mutateCallPath = new Map<string, string>();
	const reads: PriorRead[] = [];
	let lastMutateIndex = -1;
	let order = 0;

	for (const entry of entries) {
		// Compaction boundary: everything before it is replaced by a summary in the
		// live context, so drop the state accumulated so far. Conservative — the
		// kept tail before the boundary is dropped too — which can only miss a
		// dedup, never point at content that is no longer in context.
		if (isCompactionEntry(entry)) {
			readCall.clear();
			mutateCallPath.clear();
			reads.length = 0;
			lastMutateIndex = -1;
			order = 0;
			continue;
		}
		const m = toMessage(entry);
		if (!m) continue;
		const i = order++;
		if (m.role === "assistant" && Array.isArray(m.content)) {
			for (const b of m.content) {
				if (b.type !== "toolCall" || !b.id) continue;
				const raw = b.arguments?.path;
				if (typeof raw !== "string" || raw.length === 0) continue;
				const resolved = resolvePath(raw);
				if (b.name === READ_TOOL) {
					readCall.set(b.id, { resolved, display: raw, declared: readRangeFromArgs(b.arguments) });
				} else if (b.name && MUTATE_TOOLS.has(b.name)) {
					mutateCallPath.set(b.id, resolved);
				}
			}
		} else if (m.role === "toolResult" && !m.isError && m.toolCallId) {
			if (m.toolName === READ_TOOL) {
				if (m.toolCallId === opts.currentCallId) continue;
				const info = readCall.get(m.toolCallId);
				if (!info || info.resolved !== opts.resolvedPath) continue;
				const text = resultText(m);
				// A pointer fetched nothing: the GC excludes it from supersession, so we
				// must too (both as a candidate and as a superseder).
				if (isDedupPointerText(text)) continue;
				reads.push({
					index: i,
					declared: info.declared,
					delivered: deliveredRange(text, info.declared),
					display: info.display,
				});
			} else if (m.toolName && MUTATE_TOOLS.has(m.toolName)) {
				if (mutateCallPath.get(m.toolCallId) === opts.resolvedPath) lastMutateIndex = i;
			}
		}
	}

	// A read survives the GC iff no later mutate and no later read overlaps its
	// *declared* range — exactly the GC's own test — so predict it the same way.
	const survivesGc = (r: PriorRead): boolean =>
		lastMutateIndex <= r.index && !reads.some((o) => o.index > r.index && rangesOverlap(o.declared, r.declared));

	let best: PriorRead | null = null;
	for (const r of reads) {
		// Coverage uses the *delivered* range: a truncated read only holds what it returned.
		if (!rangeContains(r.delivered, opts.requestedRange)) continue;
		if (!survivesGc(r)) continue;
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
