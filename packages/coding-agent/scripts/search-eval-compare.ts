#!/usr/bin/env bun

/**
 * Paired comparison of two configs inside one eval run record.
 *
 * Use this before changing a retrieval default. Both configs scored the same
 * queries, so a mean gap is not the question — how many individual queries
 * moved, and in which direction, is.
 *
 *   bun scripts/search-eval-compare.ts "hybrid k=2" "hybrid k=60"
 *   bun scripts/search-eval-compare.ts "hybrid k=2" "hybrid k=60" --metric mrr --verbose
 *   bun scripts/search-eval-compare.ts A B --run path/to/record.json
 */

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
	type ComparableMetric,
	type ComparableRun,
	comparePaired,
	formatComparison,
	mergeRunsForComparison,
} from "../src/core/search/eval-compare.js";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function flag(name: string): string | undefined {
	const index = process.argv.indexOf(`--${name}`);
	return index !== -1 ? process.argv[index + 1] : undefined;
}

const positional = process.argv.slice(2).filter((a, i, all) => {
	if (a.startsWith("--")) return false;
	return !all[i - 1]?.startsWith("--");
});
const againstPath = flag("against");
const [before, after] = positional;
// Two modes: two configs in one run, or one config across two runs.
if (!before || (!after && !againstPath)) {
	console.error(
		'usage: search-eval-compare.ts "<config A>" "<config B>" [--metric mrr] [--run <file>] [--verbose]\n' +
			'       search-eval-compare.ts "<config>" --against <baseline.json> [--run <file>]',
	);
	process.exit(2);
}

const runPath = flag("run") ?? path.join(packageRoot, "test", "fixtures", "search-eval-baseline.json");
let run = JSON.parse(readFileSync(runPath, "utf-8")) as ComparableRun;
let labelA = before;
let labelB = after;
if (againstPath) {
	const baseline = JSON.parse(readFileSync(againstPath, "utf-8")) as ComparableRun;
	run = mergeRunsForComparison(baseline, run, before);
	labelA = `${before} (baseline)`;
	labelB = `${before} (this run)`;
	// Guard the one way this comparison lies: different corpora. Two things
	// make a corpus different — the commit it came from, and what was removed
	// from it before indexing. The exclusion list matters as much as the SHA:
	// dropping the eval's own fixture moved MRR by more than any retrieval
	// change measured here, so comparing across it would read as a huge win
	// for whatever happened to run second.
	type Prov = { provenance?: { corpusSha?: string; corpusExcluded?: string[] } };
	const load = (f: string) => (JSON.parse(readFileSync(f, "utf-8")) as Prov).provenance ?? {};
	const a = load(againstPath);
	const b = load(runPath);
	if (a.corpusSha && b.corpusSha && a.corpusSha !== b.corpusSha) {
		console.error(
			`WARNING: corpus differs (${a.corpusSha.slice(0, 12)} vs ${b.corpusSha.slice(0, 12)}) — delta includes corpus drift\n`,
		);
	}
	const exclusions = (p: typeof a) => [...(p.corpusExcluded ?? [])].sort().join(",");
	if (exclusions(a) !== exclusions(b)) {
		console.error(
			"WARNING: corpus exclusions differ — the two runs searched different corpora, " +
				`so this delta is not a retrieval delta\n  baseline: [${exclusions(a) || "none"}]\n  this run: [${exclusions(b) || "none"}]\n`,
		);
	}
}

const metrics: ComparableMetric[] = flag("metric")
	? [flag("metric") as ComparableMetric]
	: ["recallAt1", "recallAt10", "mrr"];

console.log(`run: ${path.relative(process.cwd(), runPath)}\n`);
for (const metric of metrics) {
	const comparison = comparePaired(run, againstPath ? "before" : labelA, againstPath ? "after" : labelB, metric);
	comparison.before = labelA;
	comparison.after = labelB;
	console.log(formatComparison(comparison));
	if (process.argv.includes("--verbose")) {
		for (const d of comparison.deltas) {
			const arrow = d.after > d.before ? "+" : "-";
			console.log(`    ${arrow} ${d.id} (${d.class}): ${d.before.toFixed(3)} -> ${d.after.toFixed(3)}`);
		}
	}
	console.log();
}
