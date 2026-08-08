#!/usr/bin/env bun
/**
 * Gold-set maintenance for the retrieval eval.
 *
 * The eval corpus is hoocode itself, so every refactor moves the spans the
 * gold set points at. Without a re-pin step the fixture silently rots and
 * every recall number drifts down for reasons unrelated to retrieval.
 *
 *   bun scripts/search-eval-gold.ts          # check (exit 1 on drift)
 *   bun scripts/search-eval-gold.ts --fix    # re-pin line ranges from anchors
 *
 * Check mode runs in CI. Fix mode rewrites `startLine`/`endLine` from each
 * span's `anchor`; an anchor that has vanished or become ambiguous is
 * reported rather than guessed at, because that means the gold *target* needs
 * a human decision, not a new line number.
 */

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { loadGoldSet, resolveGoldSet, validateGoldSet } from "../src/core/search/eval-gold.js";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(scriptDir, "..");
const repoRoot = path.resolve(packageRoot, "..", "..");
const fixturePath = path.join(packageRoot, "test", "fixtures", "search-eval.json");

const fix = process.argv.includes("--fix");
const dataset = loadGoldSet(fixturePath);

if (!fix) {
	const issues = validateGoldSet(repoRoot, dataset);
	if (issues.length === 0) {
		console.log(`gold set ok: ${dataset.length} queries, ${dataset.reduce((n, q) => n + q.gold.length, 0)} spans`);
		process.exit(0);
	}
	console.error(`gold set has ${issues.length} issue(s):`);
	for (const issue of issues) console.error(`  ${issue.queryId} [${issue.path}]: ${issue.problem}`);
	console.error("\nre-pin with: bun scripts/search-eval-gold.ts --fix");
	process.exit(1);
}

const { dataset: resolved, issues } = resolveGoldSet(repoRoot, dataset);
if (issues.length > 0) {
	console.error(`${issues.length} span(s) could not be resolved and need a human:`);
	for (const issue of issues) console.error(`  ${issue.queryId} [${issue.path}]: ${issue.problem}`);
	process.exit(1);
}

const next = `${JSON.stringify(resolved, null, 2)}\n`;
const previous = readFileSync(fixturePath, "utf-8");
if (next === previous) {
	console.log("gold set already up to date");
} else {
	writeFileSync(fixturePath, next);
	console.log(`re-pinned ${resolved.length} queries in ${path.relative(repoRoot, fixturePath)}`);
}
