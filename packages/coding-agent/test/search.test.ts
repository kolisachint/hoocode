import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { adaptGrepHits, type ChunkLookup } from "../src/core/search/adapter.js";
import { assembleContext } from "../src/core/search/context-assembler.js";
import { buildLexicalPattern, runLexicalRetriever } from "../src/core/search/lexical-retriever.js";
import { hasStrongLexicalSignals, resolveSearchMode } from "../src/core/search/mode.js";
import { rrfFuse } from "../src/core/search/rrf.js";
import type { FusedCandidate, RankedHit } from "../src/core/search/types.js";

describe("rrfFuse", () => {
	const hit = (id: string, rank: number, source: "grep" | "embed", score?: number): RankedHit => ({
		id,
		rank,
		source,
		score,
	});

	it("rejects invalid k and ranks", () => {
		expect(() => rrfFuse([], -1)).toThrow(/finite non-negative/);
		expect(() => rrfFuse([], Number.NaN)).toThrow(/finite non-negative/);
		expect(() => rrfFuse([[hit("a", 0, "grep")]])).toThrow(/positive integer/);
		expect(() => rrfFuse([[hit("a", 1.5, "grep")]])).toThrow(/positive integer/);
	});

	it("ranks consensus hits above single-source hits", () => {
		const fused = rrfFuse([
			[hit("both", 2, "grep"), hit("greponly", 1, "grep")],
			[hit("both", 2, "embed", 0.9), hit("embedonly", 1, "embed", 0.95)],
		]);
		expect(fused[0].id).toBe("both");
		expect(fused[0].ranks).toEqual({ grep: 2, embed: 2 });
		expect(fused[0].rawScores).toEqual({ embed: 0.9 });
	});

	it("counts duplicate source:id pairs once, keeping the best rank", () => {
		const clean = rrfFuse([[hit("a", 1, "grep")], [hit("b", 1, "embed")]]);
		const withDupes = rrfFuse([[hit("a", 1, "grep"), hit("a", 3, "grep")], [hit("b", 1, "embed")]]);
		expect(withDupes.find((h) => h.id === "a")!.rrfScore).toBe(clean.find((h) => h.id === "a")!.rrfScore);
		expect(withDupes.find((h) => h.id === "a")!.ranks.grep).toBe(1);
	});

	it("retains the best rank even when the duplicate arrives worst-first", () => {
		const fused = rrfFuse([[hit("a", 5, "grep"), hit("a", 2, "grep")]]);
		expect(fused[0].ranks.grep).toBe(2);
	});

	it("breaks exact ties deterministically by id", () => {
		const fused = rrfFuse([[hit("zzz", 1, "grep")], [hit("aaa", 1, "embed")]]);
		expect(fused.map((h) => h.id)).toEqual(["aaa", "zzz"]);
	});

	it("preserves list order for a single retriever", () => {
		const fused = rrfFuse([[hit("first", 1, "embed", 0.9), hit("second", 2, "embed", 0.5)]]);
		expect(fused.map((h) => h.id)).toEqual(["first", "second"]);
	});
});

describe("runLexicalRetriever glob filter", () => {
	const makeRepo = () => {
		const root = mkdtempSync(join(tmpdir(), "glob-test-"));
		mkdirSync(join(root, "src"));
		mkdirSync(join(root, "docs"));
		writeFileSync(join(root, "src/a.ts"), "const alpha = 1;");
		writeFileSync(join(root, "src/b.ts"), "const beta = 2;");
		writeFileSync(join(root, "docs/readme.md"), "alpha docs");
		return root;
	};

	it("filters by basename glob", async () => {
		const root = makeRepo();
		try {
			const hits = await runLexicalRetriever({ cwd: root, query: "alpha", limit: 10, glob: "*.ts" });
			const rels = hits.map((h) => h.rel);
			expect(rels).toContain("src/a.ts");
			expect(rels).not.toContain("docs/readme.md");
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("filters by path glob", async () => {
		const root = makeRepo();
		mkdirSync(join(root, "src", "nested"), { recursive: true });
		writeFileSync(join(root, "src", "nested", "c.ts"), "const alphaNested = 1;");
		try {
			const hits = await runLexicalRetriever({
				cwd: root,
				query: "alpha",
				limit: 10,
				glob: "src/**/*.ts",
			});
			const rels = hits.map((h) => h.rel);
			// After normalization to `**/src/**/*.ts`, all files under src/ match.
			expect(rels).toContain("src/nested/c.ts");
			expect(rels).toContain("src/a.ts");
			expect(rels).not.toContain("docs/readme.md");
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("returns all matches when no glob is given", async () => {
		const root = makeRepo();
		try {
			const hits = await runLexicalRetriever({ cwd: root, query: "alpha", limit: 10 });
			const rels = hits.map((h) => h.rel);
			expect(rels).toContain("src/a.ts");
			expect(rels).toContain("docs/readme.md");
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});
});

describe("resolveSearchMode", () => {
	it("resolves auto to hybrid when the index is available", () => {
		expect(resolveSearchMode("how does compaction work", "auto", true)).toEqual({ mode: "hybrid" });
	});

	it("resolves auto to lexical on strong lexical signals", () => {
		expect(resolveSearchMode('"token budget exceeded"', "auto", true).mode).toBe("lexical");
		expect(resolveSearchMode("foo\\.bar\\(", "auto", true).mode).toBe("lexical");
	});

	it("routes path-like queries to hybrid (eval: 0% lexical, 100% hybrid)", () => {
		expect(resolveSearchMode("src/core/sdk.ts", "auto", true).mode).toBe("hybrid");
	});

	it("resolves auto to lexical without degradation when the index is down", () => {
		expect(resolveSearchMode("anything", "auto", false, "not enabled")).toEqual({ mode: "lexical" });
	});

	it("degrades semantic/hybrid to lexical with a reason when the index is down", () => {
		const res = resolveSearchMode("anything", "hybrid", false, "repo under threshold");
		expect(res.mode).toBe("lexical");
		expect(res.degradedReason).toContain("repo under threshold");
		expect(resolveSearchMode("anything", "semantic", false).degradedReason).toBeDefined();
	});

	it("honors explicit requests when the index is up", () => {
		expect(resolveSearchMode("q", "lexical", true)).toEqual({ mode: "lexical" });
		expect(resolveSearchMode("q", "semantic", true)).toEqual({ mode: "semantic" });
		expect(resolveSearchMode("q", "hybrid", true)).toEqual({ mode: "hybrid" });
	});

	it("detects strong lexical signals without false-positives on prose", () => {
		expect(hasStrongLexicalSignals("where is retrieval mode selected")).toBe(false);
		expect(hasStrongLexicalSignals("parseTokenStream overflow behavior")).toBe(false);
	});
});

describe("adaptGrepHits", () => {
	// One indexed file with two chunks: lines 1-60 and 51-120.
	const lookup: ChunkLookup = (rel, line) => {
		if (rel !== "src/indexed.ts") return undefined;
		if (line >= 1 && line <= 60) return { id: "src/indexed.ts#0", path: rel, startLine: 1, endLine: 60 };
		if (line <= 120) return { id: "src/indexed.ts#1", path: rel, startLine: 51, endLine: 120 };
		return undefined;
	};

	it("collapses same-chunk hits and re-ranks gap-free", () => {
		const { hits, spans } = adaptGrepHits(
			[
				{ rel: "src/indexed.ts", line: 10 },
				{ rel: "src/indexed.ts", line: 20 }, // same chunk as line 10
				{ rel: "src/indexed.ts", line: 100 },
			],
			lookup,
		);
		expect(hits.map((h) => ({ id: h.id, rank: h.rank }))).toEqual([
			{ id: "src/indexed.ts#0", rank: 1 },
			{ id: "src/indexed.ts#1", rank: 2 },
		]);
		expect(spans.get("src/indexed.ts#0")).toEqual({ path: "src/indexed.ts", startLine: 1, endLine: 60 });
	});

	it("maps overlapping-chunk lines to the first containing chunk", () => {
		const { hits } = adaptGrepHits([{ rel: "src/indexed.ts", line: 55 }], lookup);
		expect(hits[0].id).toBe("src/indexed.ts#0");
	});

	it("synthesizes fallback ids for unindexed files instead of dropping them", () => {
		const { hits, spans } = adaptGrepHits(
			[
				{ rel: "src/unindexed.ts", line: 7 },
				{ rel: "src/indexed.ts", line: 5 },
			],
			lookup,
		);
		expect(hits.map((h) => h.id)).toEqual(["src/unindexed.ts#L7", "src/indexed.ts#0"]);
		expect(spans.get("src/unindexed.ts#L7")).toEqual({ path: "src/unindexed.ts", startLine: 2, endLine: 12 });
	});

	it("works without any lookup (index disabled)", () => {
		const { hits } = adaptGrepHits([{ rel: "a.ts", line: 3 }]);
		expect(hits).toEqual([{ id: "a.ts#L3", rank: 1, source: "grep" }]);
	});
});

describe("buildLexicalPattern", () => {
	it("prefers the longest quoted segment verbatim", () => {
		expect(buildLexicalPattern('find "token budget exceeded" in code')).toBe("token budget exceeded");
	});

	it("escapes regex metacharacters in quoted segments", () => {
		expect(buildLexicalPattern('"a.b(c)"')).toBe("a\\.b\\(c\\)");
	});

	it("ORs the longest identifier-ish tokens", () => {
		const pattern = buildLexicalPattern("where does parseTokenStream handle overflow");
		expect(pattern).toContain("parseTokenStream");
		expect(pattern).toContain("|");
		expect(pattern).not.toContain("does|"); // shorter tokens dropped after the cap? "does" is 4 chars
	});

	it("returns undefined for an unsearchable query", () => {
		expect(buildLexicalPattern("   ")).toBeUndefined();
	});
});

describe("assembleContext", () => {
	const makeRepo = (): string => {
		const root = mkdtempSync(join(tmpdir(), "search-assemble-"));
		mkdirSync(join(root, "src"), { recursive: true });
		const lines = Array.from({ length: 40 }, (_, i) => `const line${i + 1} = ${i + 1};`);
		writeFileSync(join(root, "src", "a.ts"), lines.join("\n"));
		return root;
	};

	const candidate = (id: string, path: string, startLine: number, endLine: number): FusedCandidate => ({
		id,
		rrfScore: 1,
		ranks: { grep: 1, embed: 2 },
		rawScores: {},
		path,
		startLine,
		endLine,
	});

	it("emits headers with sources and inline snippets within budget", () => {
		const root = makeRepo();
		try {
			const { text, snippetCount } = assembleContext([candidate("src/a.ts#0", "src/a.ts", 1, 5)], { cwd: root });
			expect(text).toContain("src/a.ts:1-5 [embed+grep]");
			expect(text).toContain("  1: const line1 = 1;");
			expect(text).toContain("  5: const line5 = 5;");
			expect(snippetCount).toBe(1);
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("clamps spans that overshoot the file", () => {
		const root = makeRepo();
		try {
			const { text } = assembleContext([candidate("src/a.ts#L38", "src/a.ts", 35, 43)], { cwd: root });
			expect(text).toContain("src/a.ts:35-40");
			expect(text).not.toContain("41:");
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("stops snippets when the budget runs out but keeps listing headers", () => {
		const root = makeRepo();
		try {
			const candidates = Array.from({ length: 6 }, (_, i) => candidate(`src/a.ts#${i}`, "src/a.ts", 1, 20));
			// ~25 tokens ≈ 100 chars: enough for at most one snippet.
			const { text, snippetCount } = assembleContext(candidates, { cwd: root, tokenBudget: 25 });
			expect(snippetCount).toBeLessThan(candidates.length);
			// Every candidate still appears as a header.
			expect(text.match(/src\/a\.ts:1-20/g)?.length).toBe(6);
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("degrades to a bare header when the file is unreadable", () => {
		const { text, snippetCount } = assembleContext([candidate("gone.ts#0", "gone.ts", 1, 10)], {
			cwd: "/nonexistent-root",
		});
		expect(text).toBe("gone.ts:1-10 [embed+grep]");
		expect(snippetCount).toBe(0);
	});
});
