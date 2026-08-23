import { describe, expect, test } from "vitest";
import {
	type ChainEntry,
	chainPhrase,
	chainSegments,
	chainStats,
} from "../src/modes/interactive/components/tool-chain-summary.js";

const call = (tool: string, subject: string, lines = 0, isError = false, isPartial = false): ChainEntry => ({
	tool,
	subject,
	outputLines: lines,
	isError,
	isPartial,
});
const chain = (entries: ChainEntry[]) =>
	chainSegments(entries)
		.map((s) => s.label)
		.join(" › ");

describe("chain segments (the running line)", () => {
	test("keeps the order of the run", () => {
		expect(chain([call("grep", "x"), call("read", "a.ts"), call("edit", "a.ts")])).toBe("grep › read › edit");
	});

	test("collapses a consecutive repeat", () => {
		expect(chain([call("grep", "x"), call("read", "a"), call("read", "b"), call("read", "c")])).toBe(
			"grep › read ×3",
		);
	});

	test("never merges a failure into a repeat", () => {
		// The whole point of the line is showing where it broke.
		expect(chain([call("read", "a"), call("read", "b", 0, true), call("read", "c")])).toBe("read › read✗ › read");
	});

	test("elides the middle of a long chain but never a failure", () => {
		const entries = Array.from({ length: 27 }, (_, i) =>
			i === 9 ? call("bash", "check", 0, true) : call(["ls", "grep", "read", "edit"][i % 4], `f${i}`),
		);
		const rendered = chain(entries);
		expect(rendered).toContain("bash✗");
		expect(rendered).toContain("more …");
		expect(rendered.split(" › ").length).toBeLessThan(entries.length);
	});

	test("marks a still-running call rather than calling it done", () => {
		const segments = chainSegments([call("bash", "x", 0, false, true)]);
		expect(segments[0].tone).toBe("running");
	});
});

describe("chain stats", () => {
	test("counts progress while running, totals once done", () => {
		const entries = [call("grep", "x", 5), call("bash", "y", 0, false, true)];
		expect(chainStats(entries, "running")).toBe("1 done · running");
		expect(chainStats([call("grep", "x", 5), call("bash", "y", 7)], "done")).toBe("2 calls · 12 lines");
	});

	test("always surfaces failures", () => {
		expect(chainStats([call("bash", "x", 1, true)], "done")).toContain("1 failed");
	});

	test("says an interrupted chain was interrupted", () => {
		expect(chainStats([call("grep", "x", 1)], "interrupted")).toContain("interrupted");
	});
});

describe("chain phrase (the settled line)", () => {
	test("names the most consequential thing, not the most frequent", () => {
		// Six reads and one edit is remembered as the edit.
		const entries = [
			...Array.from({ length: 6 }, (_, i) => call("read", `src/f${i}.ts`, 10)),
			call("edit", "src/keys.ts"),
		];
		expect(chainPhrase(entries)).toBe("Edited src/keys.ts");
	});

	test("names a shared location when the calls have one", () => {
		expect(chainPhrase([call("read", "packages/tui/src/a.ts"), call("read", "packages/tui/src/b.ts")])).toBe(
			"Read packages/tui/src",
		);
	});

	test("counts rather than naming one arbitrary target", () => {
		// Naming a single file out of several would claim something the chain never did.
		expect(chainPhrase([call("edit", "a.ts"), call("edit", "b.ts"), call("edit", "c.ts")])).toBe("Edited 3 files");
	});

	test("never presents a glob as a location", () => {
		expect(chainPhrase([call("find", "*.test.ts", 4), call("grep", "keys", 2)])).toBe("Explored");
	});

	test("gives a lone call its own subject, whatever shape it is", () => {
		expect(chainPhrase([call("bash", "bun run check", 40)])).toBe("Ran bun run check");
		expect(chainPhrase([call("grep", "toolOutputView", 27)])).toBe("Searched toolOutputView");
	});

	test("falls back to a count for several commands", () => {
		expect(chainPhrase([call("bash", "a"), call("bash", "b")])).toBe("Ran 2 commands");
	});

	test("still says something for an unrecognised tool", () => {
		expect(chainPhrase([call("mcp__thing__do", "x")])).toBe("Called mcp__thing__do");
	});
});

/**
 * Priority order alone is only sound while a chain is small enough for every
 * call to plausibly serve the headline act. These are the rules that switch on
 * once it is not, at 10 calls.
 */
describe("chain phrase (long chains)", () => {
	const reads = (n: number, dir: string) => Array.from({ length: n }, (_, i) => call("read", `${dir}/f${i}.ts`, 10));
	const many = (n: number, tool: string, subject: (i: number) => string) =>
		Array.from({ length: n }, (_, i) => call(tool, subject(i)));

	test("an incidental act does not get to name a long chain", () => {
		// 28 searches and 37 reads ending in one edit is an investigation that
		// happened to change something, not an edit. Naming the edit describes one
		// call out of 66.
		const entries = [
			...many(28, "grep", (i) => `symbol${i}`),
			...reads(37, "packages/coding-agent/src"),
			call("edit", "packages/coding-agent/src/core/tools/subagent.ts"),
		];
		expect(chainPhrase(entries)).toBe("Read packages/coding-agent/src");
	});

	test("a tenth of the calls is enough to keep the headline", () => {
		// Exactly at the threshold the edit still leads, though it owes the reads
		// a mention...
		expect(chainPhrase([...reads(9, "packages/tui/src"), call("edit", "src/keys.ts")])).toBe(
			"Edited src/keys.ts · 9 reads",
		);
		// ...one call further and it stops being the headline at all.
		expect(chainPhrase([...reads(10, "packages/tui/src"), call("edit", "src/keys.ts")])).toBe(
			"Read packages/tui/src",
		);
	});

	test("below the threshold a long chain behaves exactly as before", () => {
		const entries = [...reads(6, "src"), call("edit", "src/keys.ts")];
		expect(chainPhrase(entries)).toBe("Edited src/keys.ts");
	});

	test("a location too broad to mean anything becomes a count", () => {
		// Across enough files the shared root collapses to `packages`, which names
		// half the repo. The count says more.
		const entries = [...reads(20, "packages/tui/src"), ...reads(20, "packages/ai/src")];
		expect(chainPhrase(entries)).toBe("Read 40 files");
	});

	test("a short chain keeps its shallow location", () => {
		// `docs` is a real place; three files rarely share a vague one.
		expect(chainPhrase([call("write", "docs/a.md"), call("write", "docs/b.md")])).toBe("Edited docs");
	});

	test("an absolute location keeps its leading slash", () => {
		expect(chainPhrase([call("read", "/etc/nginx/a.conf"), call("read", "/etc/nginx/b.conf")])).toBe(
			"Read /etc/nginx",
		);
	});

	test("a minority headline admits the largest thing it left out", () => {
		// 3 edits among 18 calls is worth naming, but so is the fact that 10 of the
		// rest were commands.
		const entries = [
			...many(3, "edit", (i) => `packages/tui/src/x${i}.ts`),
			...many(10, "bash", () => "bun run check"),
			...reads(5, "packages/tui/src"),
		];
		expect(chainPhrase(entries)).toBe("Edited packages/tui/src · 10 commands");
	});

	test("a headline that covers the run says nothing more", () => {
		const entries = [...many(10, "edit", (i) => `packages/tui/src/x${i}.ts`), ...reads(3, "packages/tui/src")];
		expect(chainPhrase(entries)).toBe("Edited packages/tui/src");
	});

	test("a footnote-sized remainder is not worth the width", () => {
		const entries = [
			...many(4, "edit", (i) => `src/x${i}.ts`),
			...many(2, "bash", () => "check"),
			...many(2, "read", (i) => `src/y${i}.ts`),
			...many(2, "grep", (i) => `sym${i}`),
			...many(2, "webfetch", () => "https://example.com"),
		];
		expect(chainPhrase(entries)).not.toContain("·");
	});

	test("a chain spread thin across everything still names its largest act", () => {
		// Unrecognised tools inflate the total until every family is incidental;
		// the largest one still describes the run better than the priority order's
		// first hit would.
		const entries = [
			...many(50, "TodoWrite", () => "plan"),
			...many(4, "edit", (i) => `src/x${i}.ts`),
			...many(4, "bash", () => "check"),
		];
		expect(chainPhrase(entries)).toBe("Edited 4 files · 4 commands");
	});
});
