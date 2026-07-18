import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { EmbsearchService } from "../src/core/embsearch/embsearch-service.js";
import { evaluateQuery, recallAtK, spanMatchesGold } from "../src/core/search/eval.js";
import { retrieveCandidates, runSearch } from "../src/core/search/hybrid-search.js";
import { getSearchTracePath } from "../src/core/search/trace.js";

// Hermetic setup: native grep (no rg binary dependency) and a temp agent dir
// so search traces never touch the real home directory.
let agentDir: string;
const savedEnv: Record<string, string | undefined> = {};
beforeAll(() => {
	savedEnv.HOOCODE_NATIVE_SEARCH = process.env.HOOCODE_NATIVE_SEARCH;
	savedEnv.HOOCODE_CODING_AGENT_DIR = process.env.HOOCODE_CODING_AGENT_DIR;
	process.env.HOOCODE_NATIVE_SEARCH = "1";
	agentDir = mkdtempSync(join(tmpdir(), "search-agent-dir-"));
	process.env.HOOCODE_CODING_AGENT_DIR = agentDir;
});
afterAll(() => {
	for (const [key, value] of Object.entries(savedEnv)) {
		if (value === undefined) delete process.env[key];
		else process.env[key] = value;
	}
	rmSync(agentDir, { recursive: true, force: true });
});

/** Repo with one "indexed" file (per the stub sidecar below), one unindexed
 *  file with clustered matches, and one file only the embedder knows. */
function makeRepo(): string {
	const root = mkdtempSync(join(tmpdir(), "hybrid-search-"));
	mkdirSync(join(root, "src"), { recursive: true });
	const indexed = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`);
	indexed[4] = "function spendTokenBudget() {"; // line 5 → chunk #0
	indexed[14] = "const tokenBudget = 2000;"; // line 15 → chunk #1
	writeFileSync(join(root, "src", "indexed.ts"), indexed.join("\n"));
	const unindexed = Array.from({ length: 10 }, (_, i) => `filler ${i + 1}`);
	unindexed[2] = "tokenBudget";
	unindexed[4] = "tokenBudget";
	unindexed[6] = "tokenBudget";
	writeFileSync(join(root, "src", "unindexed.ts"), unindexed.join("\n"));
	writeFileSync(join(root, "src", "other.ts"), "conceptually related, no literal match\n");
	return root;
}

const readyService = {
	getState: () => ({ phase: "ready", chunkCount: 3 }),
	isAvailable: () => true,
	searchChunks: async () => [
		{ id: "src/indexed.ts#0", path: "src/indexed.ts", startLine: 1, endLine: 10, score: 0.9 },
		{ id: "src/other.ts#0", path: "src/other.ts", startLine: 1, endLine: 1, score: 0.5 },
	],
	findEnclosingChunk: (rel: string, line: number) => {
		if (rel !== "src/indexed.ts") return undefined;
		return line <= 10
			? { id: "src/indexed.ts#0", path: rel, startLine: 1, endLine: 10 }
			: { id: "src/indexed.ts#1", path: rel, startLine: 11, endLine: 20 };
	},
} as unknown as EmbsearchService;

const downService = {
	getState: () => ({ phase: "unavailable", reason: "binary not found" }),
	isAvailable: () => false,
} as unknown as EmbsearchService;

describe("retrieveCandidates (stubbed service)", () => {
	it("hybrid ranks the grep+embed consensus chunk first", async () => {
		const root = makeRepo();
		try {
			const { candidates, resolvedMode } = await retrieveCandidates({
				cwd: root,
				query: "tokenBudget",
				mode: "hybrid",
				service: readyService,
			});
			expect(resolvedMode).toBe("hybrid");
			expect(candidates[0].id).toBe("src/indexed.ts#0");
			expect(Object.keys(candidates[0].ranks).sort()).toEqual(["embed", "grep"]);
			// Embed-only and grep-only candidates both survive fusion.
			expect(candidates.map((c) => c.id)).toContain("src/other.ts#0");
			expect(candidates.map((c) => c.id)).toContain("src/unindexed.ts#L3");
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("coalesces adjacent unindexed hits into one fallback candidate", async () => {
		const root = makeRepo();
		try {
			const { candidates } = await retrieveCandidates({
				cwd: root,
				query: "tokenBudget",
				mode: "lexical",
			});
			const fallbacks = candidates.filter((c) => c.path === "src/unindexed.ts");
			expect(fallbacks).toHaveLength(1);
			expect(fallbacks[0].id).toBe("src/unindexed.ts#L3");
			// Cluster spans lines 3..7 plus padding.
			expect(fallbacks[0].startLine).toBe(1);
			expect(fallbacks[0].endLine).toBe(12);
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("degrades hybrid to lexical with a reason when the index is down", async () => {
		const root = makeRepo();
		try {
			const result = await retrieveCandidates({
				cwd: root,
				query: "tokenBudget",
				mode: "hybrid",
				service: downService,
			});
			expect(result.resolvedMode).toBe("lexical");
			expect(result.degradedReason).toContain("binary not found");
			expect(result.indexPhase).toBe("unavailable");
			expect(result.candidates.length).toBeGreaterThan(0);
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});
});

describe("runSearch (stubbed service)", () => {
	it("returns budgeted text with source labels and writes a trace record", async () => {
		const root = makeRepo();
		try {
			const result = await runSearch({ cwd: root, query: "tokenBudget", mode: "hybrid", service: readyService });
			expect(result.resolvedMode).toBe("hybrid");
			expect(result.text).toContain("src/indexed.ts:1-10 [embed+grep]");

			const tracePath = getSearchTracePath(root);
			expect(existsSync(tracePath)).toBe(true);
			const lines = readFileSync(tracePath, "utf-8").trim().split("\n");
			const trace = JSON.parse(lines[lines.length - 1]);
			expect(trace.resolvedMode).toBe("hybrid");
			expect(trace.rrfK).toBe(2);
			expect(trace.retrievers.grep.hitCount).toBeGreaterThan(0);
			expect(trace.retrievers.embed.hitCount).toBe(2);
			expect(trace.fused[0].id).toBe("src/indexed.ts#0");
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});
});

describe("eval scoring", () => {
	it("matches gold spans by path and line overlap", () => {
		const span = { path: "a.ts", startLine: 10, endLine: 20 };
		expect(spanMatchesGold(span, { path: "a.ts" })).toBe(true);
		expect(spanMatchesGold(span, { path: "a.ts", startLine: 20, endLine: 30 })).toBe(true);
		expect(spanMatchesGold(span, { path: "a.ts", startLine: 21, endLine: 30 })).toBe(false);
		expect(spanMatchesGold(span, { path: "b.ts" })).toBe(false);
	});

	it("computes recall over the top-k only", () => {
		const candidates = [
			{ path: "a.ts", startLine: 1, endLine: 5 },
			{ path: "b.ts", startLine: 1, endLine: 5 },
			{ path: "c.ts", startLine: 1, endLine: 5 },
		];
		const gold = [{ path: "a.ts" }, { path: "c.ts" }];
		expect(recallAtK(candidates, gold, 3)).toBe(1);
		expect(recallAtK(candidates, gold, 2)).toBe(0.5);
		expect(recallAtK(candidates, gold, 0)).toBe(0);
		expect(recallAtK(candidates, [], 3)).toBe(0);
	});

	it("evaluates a query across configs against a stub service", async () => {
		const root = makeRepo();
		try {
			const results = await evaluateQuery(
				root,
				{
					id: "q1",
					class: "exact-symbol",
					query: "tokenBudget",
					gold: [{ path: "src/indexed.ts" }],
				},
				undefined,
				readyService,
			);
			const byLabel = new Map(results.map((r) => [r.label, r]));
			expect(byLabel.get("lexical")!.recallAt5).toBe(1);
			expect(byLabel.get("semantic")!.recallAt5).toBe(1);
			expect(byLabel.get("hybrid k=60")!.recallAt5).toBe(1);
			expect(byLabel.get("auto")!.resolvedMode).toBe("hybrid");
			expect(results.every((r) => !r.degraded)).toBe(true);
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});
});

describe("rerankCandidates", () => {
	const fused = (id: string, path: string, startLine: number, endLine: number) => ({
		id,
		rrfScore: 1,
		ranks: { grep: 1 as number },
		rawScores: {},
		path,
		startLine,
		endLine,
	});

	it("lifts a deep candidate whose window covers more query terms", async () => {
		const { rerankCandidates } = await import("../src/core/search/rerank.js");
		const root = mkdtempSync(join(tmpdir(), "rerank-"));
		try {
			writeFileSync(join(root, "weak.ts"), "only budget here\n");
			writeFileSync(join(root, "strong.ts"), "tokenBudget assembler snippet window\n");
			const result = rerankCandidates(
				"tokenBudget assembler window",
				[fused("weak.ts#0", "weak.ts", 1, 1), fused("strong.ts#0", "strong.ts", 1, 1)],
				root,
			);
			expect(result.candidates[0].id).toBe("strong.ts#0");
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("ranks a path-named file first for path-like queries", async () => {
		const { rerankCandidates } = await import("../src/core/search/rerank.js");
		const root = mkdtempSync(join(tmpdir(), "rerank-path-"));
		try {
			mkdirSync(join(root, "src"), { recursive: true });
			writeFileSync(join(root, "src", "other.ts"), "mentions hybrid-search.ts in a comment\n");
			writeFileSync(join(root, "src", "hybrid-search.ts"), "export const x = 1;\n");
			const result = rerankCandidates(
				"src/hybrid-search.ts",
				[
					fused("src/other.ts#0", "src/other.ts", 1, 1),
					fused("src/hybrid-search.ts#0", "src/hybrid-search.ts", 1, 1),
				],
				root,
			);
			expect(result.candidates[0].id).toBe("src/hybrid-search.ts#0");
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("keeps fused order on ties and unreadable files", async () => {
		const { rerankCandidates } = await import("../src/core/search/rerank.js");
		const result = rerankCandidates(
			"nomatch",
			[fused("a.ts#0", "a.ts", 1, 1), fused("b.ts#0", "b.ts", 1, 1)],
			"/nonexistent",
		);
		expect(result.candidates.map((c) => c.id)).toEqual(["a.ts#0", "b.ts#0"]);
	});
});
