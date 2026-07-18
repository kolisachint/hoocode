#!/usr/bin/env node
/**
 * Retrieval eval gate (docs/hybrid-retrieval-design.md, step 6).
 *
 * Runs the gold set in test/fixtures/search-eval.json against the built
 * search pipeline (run `npm run build` first) and prints mean Recall@5/10/50
 * per config: lexical, semantic, hybrid k ∈ {0, 2, 10, 60}, and routed auto.
 *
 * Usage:  node scripts/search-eval.mjs [--cwd <repo-root>]
 *
 * Semantic/hybrid rows need the embsearch binary; when it is unavailable the
 * script still runs and marks those rows as degraded-to-lexical, so the
 * lexical baseline is always measurable.
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { EmbsearchService } from "../dist/core/embsearch/embsearch-service.js";
import { EVAL_CONFIGS, evaluateQuery } from "../dist/core/search/eval.js";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const cwdArgIndex = process.argv.indexOf("--cwd");
// Default to the repo root (two levels up from packages/coding-agent/scripts).
const cwd = path.resolve(cwdArgIndex !== -1 ? process.argv[cwdArgIndex + 1] : path.join(scriptDir, "..", "..", ".."));

const dataset = JSON.parse(readFileSync(path.join(scriptDir, "..", "test", "fixtures", "search-eval.json"), "utf-8"));

console.error(`search-eval: ${dataset.length} queries, cwd=${cwd}`);

// Try to bring the semantic index up; every failure degrades to lexical.
const service = new EmbsearchService({
	cwd,
	thresholdBytes: 0,
	onProgress: (state) => {
		if (state.phase === "indexing" && state.done % 480 === 0) {
			console.error(`  embsearch: indexing ${state.done}/${state.total}`);
		}
	},
});
await service.start();
const state = service.getState();
console.error(`  embsearch: ${state.phase}${"reason" in state ? ` (${state.reason})` : ""}`);

// label -> { r5, r10, r50, n, degraded }
const totals = new Map();
const perQuery = [];

for (const evalQuery of dataset) {
	const results = await evaluateQuery(cwd, evalQuery, EVAL_CONFIGS, service);
	perQuery.push({ id: evalQuery.id, class: evalQuery.class, results });
	for (const r of results) {
		const t = totals.get(r.label) ?? { r5: 0, r10: 0, r50: 0, n: 0, degraded: 0 };
		t.r5 += r.recallAt5;
		t.r10 += r.recallAt10;
		t.r50 += r.recallAt50;
		t.n++;
		if (r.degraded) t.degraded++;
		totals.set(r.label, t);
	}
}

await service.dispose();

const pct = (x) => `${Math.round(x * 100)}%`.padStart(5);
console.log("\nconfig        | R@5   | R@10  | R@50  | notes");
console.log("--------------|-------|-------|-------|------");
for (const config of EVAL_CONFIGS) {
	const t = totals.get(config.label);
	const notes = t.degraded === t.n ? "degraded to lexical" : t.degraded > 0 ? `${t.degraded}/${t.n} degraded` : "";
	console.log(
		`${config.label.padEnd(13)} | ${pct(t.r5 / t.n)} | ${pct(t.r10 / t.n)} | ${pct(t.r50 / t.n)} | ${notes}`,
	);
}

// Per-query misses at R@10 — the actionable list.
const misses = perQuery.filter((q) => q.results.some((r) => !r.degraded && r.recallAt10 < 1));
if (misses.length > 0) {
	console.log("\nqueries below full R@10 (non-degraded configs):");
	for (const q of misses) {
		const worst = q.results
			.filter((r) => !r.degraded)
			.map((r) => `${r.label}=${pct(r.recallAt10).trim()}`)
			.join(", ");
		console.log(`  ${q.id} (${q.class}): ${worst}`);
	}
}
