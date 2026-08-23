/**
 * The radar view's chain line: one line for a run of consecutive tool calls.
 *
 * Two renderings of the same chain, because a run that is still going and a run
 * that is over answer different questions.
 *
 * While it runs you want the shape — what it did, in order, and where it broke:
 *
 *   ◐ grep › read › bash✗ › edit › bash…            4 done · 1 failed · running
 *
 * Once it is over the order has stopped mattering and what is left is what it
 * amounted to:
 *
 *   ● Edited packages/tui/src/keys.ts               5 calls · 1 failed
 *
 * A chain that did *not* finish keeps the running form plus a marker. The
 * settled form is a claim about what was accomplished, and a chain cut off
 * partway through has no such claim to make — the same reason
 * `settleDanglingMainTasks` refuses to mark an aborted turn's plan items done.
 *
 * Everything here is pure and deterministic. A model-written phase label would
 * read better than the inferred one, but it would cost tokens on every turn and
 * this agent's whole premise is that nothing happens you did not pay for on
 * purpose.
 */

/** One tool call's contribution to its chain. */
export interface ChainEntry {
	tool: string;
	/** The call's subject — a path, a command, a pattern. */
	subject: string;
	isError: boolean;
	/** Still executing. */
	isPartial: boolean;
	/** Lines of text output the call returned. */
	outputLines: number;
}

export type ChainState =
	/** Calls are still arriving or executing. */
	| "running"
	/** Every call finished and the turn moved on cleanly. */
	| "done"
	/** The turn was aborted, errored, or hit a length cap partway through. */
	| "interrupted";

export type SegmentTone = "ok" | "error" | "running";

export interface ChainSegment {
	label: string;
	tone: SegmentTone;
}

/** Segments beyond this many are elided in the middle rather than wrapped. */
const MAX_SEGMENTS = 8;
/** Kept at the head when eliding. */
const HEAD_SEGMENTS = 3;
/** Kept at the tail when eliding. */
const TAIL_SEGMENTS = 2;

function tone(entry: ChainEntry): SegmentTone {
	if (entry.isError) return "error";
	if (entry.isPartial) return "running";
	return "ok";
}

/**
 * Collapse consecutive calls to the same tool into one segment.
 *
 * A run of five reads is one act, not five, and spelling it out crowds out the
 * calls either side of it. A failure never merges: `read ×4` hiding one broken
 * read would defeat the point of the line.
 */
function collapseRepeats(entries: ChainEntry[]): ChainSegment[] {
	const segments: ChainSegment[] = [];
	let index = 0;
	while (index < entries.length) {
		const entry = entries[index];
		const entryTone = tone(entry);
		let run = 1;
		while (
			index + run < entries.length &&
			entries[index + run].tool === entry.tool &&
			tone(entries[index + run]) === entryTone &&
			entryTone === "ok"
		) {
			run++;
		}
		const marker = entryTone === "error" ? "✗" : "";
		segments.push({ label: run > 1 ? `${entry.tool} ×${run}` : `${entry.tool}${marker}`, tone: entryTone });
		index += run;
	}
	return segments;
}

/**
 * Elide the middle of a long chain, keeping both ends legible.
 *
 * Failures are never elided. Where a run broke is the one thing the line exists
 * to show, and a long chain is exactly when you most need it — hiding the ✗
 * inside "… 22 more …" would leave the stats claiming a failure the line cannot
 * point at.
 */
function capSegments(segments: ChainSegment[]): ChainSegment[] {
	if (segments.length <= MAX_SEGMENTS) return segments;

	const keep = new Set<number>();
	for (let i = 0; i < HEAD_SEGMENTS; i++) keep.add(i);
	for (let i = segments.length - TAIL_SEGMENTS; i < segments.length; i++) keep.add(i);
	segments.forEach((segment, i) => {
		if (segment.tone === "error") keep.add(i);
	});

	const out: ChainSegment[] = [];
	let gap = 0;
	const flushGap = () => {
		if (gap > 0) out.push({ label: `… ${gap} more …`, tone: "ok" });
		gap = 0;
	};
	segments.forEach((segment, i) => {
		if (keep.has(i)) {
			flushGap();
			out.push(segment);
		} else {
			gap++;
		}
	});
	flushGap();
	return out;
}

/** The `grep › read × 3 › bash✗` part of a running chain line. */
export function chainSegments(entries: ChainEntry[]): ChainSegment[] {
	return capSegments(collapseRepeats(entries));
}

function plural(n: number, one: string, many = `${one}s`): string {
	return `${n} ${n === 1 ? one : many}`;
}

/** The flush-right stats for either rendering. */
export function chainStats(entries: ChainEntry[], state: ChainState): string {
	const failed = entries.filter((e) => e.isError).length;
	const parts: string[] = [];

	if (state === "running") {
		parts.push(`${entries.filter((e) => !e.isPartial).length} done`);
	} else {
		parts.push(plural(entries.length, "call"));
	}
	if (failed > 0) parts.push(`${failed} failed`);

	if (state === "running") {
		parts.push("running");
	} else if (state === "interrupted") {
		parts.push("interrupted");
	} else {
		const lines = entries.reduce((sum, e) => sum + e.outputLines, 0);
		if (lines > 0) parts.push(plural(lines, "line"));
	}
	return parts.join(" · ");
}

/**
 * Tool families, in the order a settled chain reports them.
 *
 * The ordering is the whole trick: a chain that read six files and then changed
 * one is remembered as the edit. Mutation outranks execution outranks reading
 * outranks searching, so the phrase names the most consequential thing the
 * chain did rather than the most frequent.
 */
const FAMILIES: Array<{ verb: string; noun: string; tools: string[] }> = [
	{ verb: "Edited", noun: "file", tools: ["edit", "write", "MultiEdit", "NotebookEdit"] },
	{ verb: "Ran", noun: "command", tools: ["bash"] },
	{ verb: "Delegated", noun: "task", tools: ["Task", "TaskOutput"] },
	{ verb: "Fetched", noun: "page", tools: ["webfetch", "websearch"] },
	{ verb: "Read", noun: "file", tools: ["read"] },
	{ verb: "Searched", noun: "search", tools: ["grep", "find", "search", "ls"] },
];

/** Longest common directory prefix of the given paths, or "" when they share none. */
function commonPathPrefix(paths: string[]): string {
	if (paths.length === 0) return "";
	const split = paths.map((p) => p.split("/").filter(Boolean));
	const [first, ...rest] = split;
	const prefix: string[] = [];
	for (let i = 0; i < first.length; i++) {
		if (rest.every((parts) => parts[i] === first[i])) prefix.push(first[i]);
		else break;
	}
	return prefix.join("/");
}

/**
 * Whether a subject can stand in for a location.
 *
 * Globs are excluded even though they look pathlike: "Searched *.test.ts" reads
 * as a place the chain worked in, which is exactly what it is not.
 */
function usableAsTarget(subject: string): boolean {
	if (/[*?]/.test(subject)) return false;
	return subject.includes("/") || /\.[a-zA-Z0-9]+$/.test(subject);
}

/**
 * The settled chain's phrase: what this run amounted to.
 *
 * Deliberately one clause, not an inventory. "Edited packages/tui, read 6
 * files, searched packages" is a worse line than "Edited packages/tui" — the
 * count is already in the stats, and the per-call detail is one `alt+u` away.
 *
 * A location is only named when the calls actually share one. Naming a single
 * arbitrary file out of five would read as a claim the chain never made, so
 * several unrelated targets collapse to a count instead.
 */
export function chainPhrase(entries: ChainEntry[]): string {
	if (entries.length === 0) return "No calls";

	for (const family of FAMILIES) {
		const matched = entries.filter((e) => family.tools.includes(e.tool));
		if (matched.length === 0) continue;

		const subjects = matched.map((e) => e.subject).filter(Boolean);

		// A lone call is fully described by its own subject, whatever shape it is:
		// a command, a pattern, a URL, a path.
		if (matched.length === 1 && subjects.length === 1) {
			return `${family.verb} ${subjects[0]}`;
		}

		// Several calls: name the place they share, or fall back to a count.
		const target = commonPathPrefix(subjects.filter(usableAsTarget));
		if (target) return `${family.verb} ${target}`;
		// Searching has no natural plural noun ("3 searches" says nothing the
		// stats do not), so the bare verb carries it.
		if (family.noun === "search") return "Explored";
		return `${family.verb} ${plural(matched.length, family.noun)}`;
	}

	// An unrecognised tool set still gets a line rather than a blank.
	const names = [...new Set(entries.map((e) => e.tool))];
	return names.length === 1 ? `Called ${names[0]}` : `Called ${plural(names.length, "tool")}`;
}
