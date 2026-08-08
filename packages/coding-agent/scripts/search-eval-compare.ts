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
const [before, after] = positional;
if (!before || !after) {
	console.error('usage: search-eval-compare.ts "<config A>" "<config B>" [--metric mrr] [--run <file>] [--verbose]');
	process.exit(2);
}

const runPath = flag("run") ?? path.join(packageRoot, "test", "fixtures", "search-eval-baseline.json");
const run = JSON.parse(readFileSync(runPath, "utf-8")) as ComparableRun;

const metrics: ComparableMetric[] = flag("metric")
	? [flag("metric") as ComparableMetric]
	: ["recallAt1", "recallAt10", "mrr"];

console.log(`run: ${path.relative(process.cwd(), runPath)}\n`);
for (const metric of metrics) {
	const comparison = comparePaired(run, before, after, metric);
	console.log(formatComparison(comparison));
	if (process.argv.includes("--verbose")) {
		for (const d of comparison.deltas) {
			const arrow = d.after > d.before ? "+" : "-";
			console.log(`    ${arrow} ${d.id} (${d.class}): ${d.before.toFixed(3)} -> ${d.after.toFixed(3)}`);
		}
	}
	console.log();
}
