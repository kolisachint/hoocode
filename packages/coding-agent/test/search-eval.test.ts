/**
 * Guards the retrieval eval gate itself.
 *
 * Two failures this file exists to prevent, both of which already happened:
 *
 *  1. **The harness stopped running and nothing noticed.** `EVAL_CONFIGS` lost
 *     its `export` to a dead-export sweep (02efaab) because its only importer
 *     was a `.mjs` script reading `dist/`, invisible to static analysis. The
 *     imports in this file are TypeScript importers, so the same sweep now
 *     keeps the eval surface alive — and if someone removes them anyway, this
 *     test fails to compile instead of the gate silently dying.
 *  2. **The gold set rots.** The corpus is this repo, so refactors move the
 *     spans gold points at. A stale fixture does not error, it just quietly
 *     depresses every recall number.
 */

import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { EVAL_CONFIGS, type EvalGoldSpan, mrr, recallAtK, spanMatchesGold } from "../src/core/search/eval.js";
import { comparePaired, signTestPValue } from "../src/core/search/eval-compare.js";
import { loadGoldSet, validateGoldSet } from "../src/core/search/eval-gold.js";
import { hashRetrievalSource, summarizeGoldSet } from "../src/core/search/eval-harness.js";
import type { CandidateSpan } from "../src/core/search/types.js";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(packageRoot, "..", "..");
const fixturePath = path.join(packageRoot, "test", "fixtures", "search-eval.json");

const span = (p: string, startLine: number, endLine: number): CandidateSpan => ({ path: p, startLine, endLine });

/** Minimal per-config result row for the paired-comparison tests. */
const res = (label: string, value: number) => ({
	label,
	resolvedMode: "hybrid" as const,
	degraded: false,
	recallAt1: value,
	recallAt5: value,
	recallAt10: value,
	recallAt50: value,
	mrr: value,
});

describe("search eval gold set", () => {
	const dataset = loadGoldSet(fixturePath);

	it("is anchored, in bounds, and current against the working tree", () => {
		const issues = validateGoldSet(repoRoot, dataset);
		const rendered = issues.map((i) => `${i.queryId} [${i.path}]: ${i.problem}`).join("\n");
		expect(rendered, "re-pin with: bun scripts/search-eval-gold.ts --fix").toBe("");
	});

	it("is large enough and spread across query classes", () => {
		const summary = summarizeGoldSet(dataset);
		// The first gold set was 12 queries — small enough that every gap in the
		// published table sat inside the sampling noise. This is the floor that
		// keeps a rerun meaningful, not a target.
		expect(summary.queryCount).toBeGreaterThanOrEqual(50);
		for (const cls of ["exact-symbol", "error-fragment", "path", "conceptual", "cross-file", "boundary"]) {
			expect(summary.byClass[cls] ?? 0, `class ${cls}`).toBeGreaterThan(0);
		}
		// Identifier queries are where a lexical retriever should be strongest,
		// so they carry the most weight in any lexical-vs-semantic comparison.
		expect(summary.byClass["exact-symbol"]).toBeGreaterThanOrEqual(15);
	});

	it("does not concentrate gold in the search subsystem it is measuring", () => {
		const files = new Set(dataset.flatMap((q) => q.gold.map((g) => g.path)));
		const inSearch = [...files].filter((f) => f.includes("/core/search/")).length;
		expect(inSearch / files.size).toBeLessThan(0.35);
	});

	it("scores span-scoped gold by overlap, not by file", () => {
		const gold = dataset.flatMap((q) => q.gold).filter((g) => g.scope !== "file");
		expect(gold.length).toBeGreaterThan(0);
		for (const g of gold) {
			expect(g.startLine, `${g.path} needs line numbers`).toBeDefined();
			expect(g.anchor, `${g.path} needs an anchor`).toBeTruthy();
		}
	});
});

describe("eval metrics", () => {
	const gold: EvalGoldSpan[] = [{ path: "a.ts", startLine: 10, endLine: 20 }];

	it("matches a gold span by line overlap", () => {
		expect(spanMatchesGold(span("a.ts", 15, 30), gold[0])).toBe(true);
		expect(spanMatchesGold(span("a.ts", 1, 9), gold[0])).toBe(false);
		expect(spanMatchesGold(span("b.ts", 15, 16), gold[0])).toBe(false);
	});

	it("accepts any span in the file for file-scoped gold", () => {
		const fileGold: EvalGoldSpan = { path: "a.ts", startLine: 1, endLine: 400, scope: "file" };
		expect(spanMatchesGold(span("a.ts", 380, 400), fileGold)).toBe(true);
	});

	it("separates rank-1 from rank-5 the way Recall@5 cannot", () => {
		const first = [span("a.ts", 10, 20), span("z.ts", 1, 5)];
		const fifth = [
			span("z.ts", 1, 5),
			span("y.ts", 1, 5),
			span("x.ts", 1, 5),
			span("w.ts", 1, 5),
			span("a.ts", 10, 20),
		];
		expect(recallAtK(first, gold, 5)).toBe(recallAtK(fifth, gold, 5));
		expect(recallAtK(first, gold, 1)).toBe(1);
		expect(recallAtK(fifth, gold, 1)).toBe(0);
		expect(mrr(first, gold)).toBe(1);
		expect(mrr(fifth, gold)).toBeCloseTo(0.2, 5);
	});

	it("averages MRR over gold spans so partial cross-file answers score partially", () => {
		const twoGold: EvalGoldSpan[] = [
			{ path: "a.ts", startLine: 1, endLine: 5 },
			{ path: "b.ts", startLine: 1, endLine: 5 },
		];
		expect(mrr([span("a.ts", 1, 5)], twoGold)).toBeCloseTo(0.5, 5);
		expect(mrr([span("a.ts", 1, 5), span("b.ts", 1, 5)], twoGold)).toBeCloseTo(0.75, 5);
	});

	it("scores an empty gold set as zero rather than dividing by zero", () => {
		expect(recallAtK([span("a.ts", 1, 5)], [], 5)).toBe(0);
		expect(mrr([span("a.ts", 1, 5)], [])).toBe(0);
	});
});

describe("eval harness provenance", () => {
	it("sweeps every retriever mode and both rerank settings", () => {
		const labels = EVAL_CONFIGS.map((c) => c.label);
		expect(labels).toContain("lexical");
		expect(labels).toContain("semantic");
		expect(labels).toContain("auto");
		expect(EVAL_CONFIGS.some((c) => c.rerank === true)).toBe(true);
		expect(new Set(labels).size).toBe(labels.length);
	});

	it("hashes retrieval source so a tuning change invalidates old numbers", () => {
		const hash = hashRetrievalSource(repoRoot);
		expect(hash).toMatch(/^[0-9a-f]{16}$/);
		expect(hashRetrievalSource(repoRoot)).toBe(hash);
	});
});

describe("paired comparison", () => {
	const run = {
		perQuery: [
			{ id: "q1", class: "c", results: [res("A", 1), res("B", 0)] },
			{ id: "q2", class: "c", results: [res("A", 1), res("B", 0)] },
			{ id: "q3", class: "c", results: [res("A", 0), res("B", 1)] },
			{ id: "q4", class: "c", results: [res("A", 0.5), res("B", 0.5)] },
		],
	};

	it("counts wins and losses per query rather than comparing means", () => {
		const c = comparePaired(run, "A", "B", "mrr");
		expect({ better: c.better, worse: c.worse, tied: c.tied }).toEqual({ better: 1, worse: 2, tied: 1 });
	});

	it("treats a small mean gap over few moved queries as not significant", () => {
		// 1 vs 2 is exactly the shape of the k=2/k=60 comparison: a mean gap
		// carried by a couple of queries, which the sign test refuses to call.
		expect(comparePaired(run, "A", "B", "mrr").pValue).toBeGreaterThan(0.05);
	});

	it("reports p=1 when every query ties, rather than dividing by zero", () => {
		const tiedRun = { perQuery: [{ id: "q", class: "c", results: [res("A", 1), res("B", 1)] }] };
		expect(comparePaired(tiedRun, "A", "B", "mrr").pValue).toBe(1);
	});

	it("calls a lopsided split significant", () => {
		expect(signTestPValue(10, 0)).toBeLessThanOrEqual(0.05);
		expect(signTestPValue(5, 5)).toBe(1);
	});
});
