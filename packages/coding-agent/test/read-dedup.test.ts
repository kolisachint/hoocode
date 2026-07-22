import { describe, expect, it } from "vitest";
import { evictSupersededReads } from "../src/core/context-gc.js";
import {
	buildDedupPointerText,
	type CoveringRead,
	findCoveringRead,
	isDedupPointerText,
	readRangeFromArgs,
} from "../src/core/tools/read-dedup.js";

// --- Message builders (AgentMessage-shaped, as findCoveringRead/GC consume) ---

function readCall(id: string, path: string, args?: { offset?: number; limit?: number }) {
	return {
		role: "assistant" as const,
		content: [{ type: "toolCall", id, name: "read", arguments: { path, ...args } }],
	};
}

function mutateCall(id: string, path: string, name: "edit" | "write" = "edit") {
	return {
		role: "assistant" as const,
		content: [{ type: "toolCall", id, name, arguments: { path } }],
	};
}

function result(toolCallId: string, toolName: string, text: string, isError = false) {
	return { role: "toolResult" as const, toolCallId, toolName, content: [{ type: "text", text }], isError };
}

function readResult(id: string, text: string) {
	return result(id, "read", text);
}

const identity = (raw: string) => raw;

function cover(entries: unknown[], path: string, args?: { offset?: number; limit?: number }, currentCallId = "cur") {
	return findCoveringRead(entries, {
		resolvedPath: path,
		requestedRange: readRangeFromArgs(args),
		currentCallId,
		resolvePath: identity,
	});
}

describe("findCoveringRead", () => {
	it("covers a whole-file re-read from a prior untruncated whole-file read", () => {
		const entries = [readCall("r1", "/f.txt"), readResult("r1", "line1\nline2\nline3")];
		const covering = cover(entries, "/f.txt");
		expect(covering).not.toBeNull();
		expect(covering?.end).toBe(Number.POSITIVE_INFINITY);
	});

	it("covers a sub-range request from a prior wider read", () => {
		const entries = [readCall("r1", "/f.txt"), readResult("r1", "content")];
		expect(cover(entries, "/f.txt", { offset: 10, limit: 5 })).not.toBeNull();
	});

	it("does NOT cover when the prior read was cap-truncated below the request", () => {
		// Prior whole-file read truncated to lines 1-400; a whole-file re-read is not covered.
		const entries = [
			readCall("r1", "/f.txt"),
			readResult("r1", "body\n\n[Showing lines 1-400 of 1000 (16.0KB limit). Use offset=401 to continue.]"),
		];
		expect(cover(entries, "/f.txt")).toBeNull();
		// ...but a request wholly inside the delivered 1-400 window IS covered.
		expect(cover(entries, "/f.txt", { offset: 1, limit: 50 })).not.toBeNull();
	});

	it("does NOT cover once the file is edited after the read", () => {
		const entries = [
			readCall("r1", "/f.txt"),
			readResult("r1", "content"),
			mutateCall("e1", "/f.txt"),
			result("e1", "edit", "edited"),
		];
		expect(cover(entries, "/f.txt")).toBeNull();
	});

	it("does NOT cover when a later overlapping read has superseded the covering read (GC would stub it)", () => {
		const entries = [
			readCall("r1", "/f.txt"),
			readResult("r1", "full content"),
			readCall("r2", "/f.txt", { offset: 5, limit: 3 }),
			readResult("r2", "partial"),
		];
		// r1 is stubbed by GC because r2 overlaps it; r2 only delivered lines 5-7,
		// so neither can cover a whole-file re-read.
		expect(cover(entries, "/f.txt")).toBeNull();
	});

	it("ignores an earlier dedup pointer as a candidate and as a superseder", () => {
		const pointer = buildDedupPointerText({ display: "/f.txt", start: 1, end: Number.POSITIVE_INFINITY });
		const entries = [
			readCall("r1", "/f.txt"),
			readResult("r1", "full content"),
			readCall("r2", "/f.txt"),
			readResult("r2", pointer), // a prior guard hit — fetched nothing
		];
		// r2 (pointer) neither covers on its own nor supersedes r1, so r1 still covers.
		const covering = cover(entries, "/f.txt", undefined, "cur");
		expect(covering).not.toBeNull();
		expect(covering?.end).toBe(Number.POSITIVE_INFINITY);
	});

	it("is not spoofed by a [Showing lines ...] string inside the file content", () => {
		// A full, untruncated read of a file that happens to contain the truncation
		// notice text must still be treated as delivering the whole file.
		const body = "intro\n[Showing lines 1-5 of 9999] appears in this doc\nmore body\nconclusion";
		const entries = [readCall("r1", "/f.txt"), readResult("r1", body)];
		const covering = cover(entries, "/f.txt");
		expect(covering).not.toBeNull();
		expect(covering?.end).toBe(Number.POSITIVE_INFINITY);
	});

	it("predicts GC supersession by declared range, not delivered range", () => {
		// r1 declared whole-file but was truncated to lines 1-400. r2 reads lines
		// 500-560. The GC stubs r1 because their DECLARED ranges overlap, even
		// though the delivered ranges are disjoint — so the guard must not point at r1.
		const entries = [
			readCall("r1", "/f.txt"),
			readResult("r1", "head\n\n[Showing lines 1-400 of 1000. Use offset=401 to continue.]"),
			readCall("r2", "/f.txt", { offset: 500, limit: 60 }),
			readResult("r2", "tail"),
		];
		expect(cover(entries, "/f.txt", { offset: 1, limit: 50 })).toBeNull();
	});

	it("does not match a different file", () => {
		const entries = [readCall("r1", "/a.txt"), readResult("r1", "content")];
		expect(cover(entries, "/b.txt")).toBeNull();
	});

	it("ignores reads before a compaction boundary (summarized away from live context)", () => {
		const entries = [
			readCall("r0", "/f.txt"),
			readResult("r0", "pre-compaction content"),
			{ type: "compaction", id: "c1", summary: "..." },
		];
		expect(cover(entries, "/f.txt")).toBeNull();
	});

	it("covers a read that follows the last compaction boundary", () => {
		const entries = [
			readCall("r0", "/f.txt"),
			readResult("r0", "pre-compaction content"),
			{ type: "compaction", id: "c1", summary: "..." },
			readCall("r1", "/f.txt"),
			readResult("r1", "fresh content"),
		];
		expect(cover(entries, "/f.txt")).not.toBeNull();
	});

	it("ignores error results", () => {
		const entries = [readCall("r1", "/f.txt"), result("r1", "read", "boom", true)];
		expect(cover(entries, "/f.txt")).toBeNull();
	});
});

describe("buildDedupPointerText / isDedupPointerText", () => {
	it("round-trips the pointer marker for whole-file, range, and single-line", () => {
		const whole: CoveringRead = { display: "/f.txt", start: 1, end: Number.POSITIVE_INFINITY };
		const range: CoveringRead = { display: "/f.txt", start: 10, end: 21 };
		const single: CoveringRead = { display: "/f.txt", start: 7, end: 8 };
		expect(buildDedupPointerText(whole)).toContain("the entire file");
		expect(buildDedupPointerText(range)).toContain("lines 10-20");
		expect(buildDedupPointerText(single)).toContain("line 7");
		for (const c of [whole, range, single]) {
			expect(isDedupPointerText(buildDedupPointerText(c))).toBe(true);
		}
		expect(isDedupPointerText("just some file contents")).toBe(false);
	});
});

describe("context GC x dedup pointers", () => {
	it("does not stub a covering read just because a later dedup-pointer read points at it", () => {
		const pointer = buildDedupPointerText({ display: "/f.txt", start: 1, end: Number.POSITIVE_INFINITY });
		const messages = [
			readCall("r1", "/f.txt"),
			readResult("r1", "full content that must survive"),
			readCall("r2", "/f.txt"),
			readResult("r2", pointer),
		];
		const out = evictSupersededReads(messages as never, { cwd: "/" });
		// r1's result (index 1) is untouched — the pointer did not supersede it.
		const r1 = out[1] as { content: Array<{ text?: string }> };
		expect(r1.content[0].text).toContain("full content that must survive");
	});

	it("still stubs a read superseded by a later real overlapping read", () => {
		const messages = [
			readCall("r1", "/f.txt"),
			readResult("r1", "old content"),
			readCall("r2", "/f.txt"),
			readResult("r2", "new content"),
		];
		const out = evictSupersededReads(messages as never, { cwd: "/" });
		const r1 = out[1] as { content: Array<{ text?: string }> };
		expect(r1.content[0].text).toContain("Superseded read");
	});
});
