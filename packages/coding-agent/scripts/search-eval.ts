#!/usr/bin/env bun
/**
 * Retrieval eval gate (docs/hybrid-retrieval-design.md, step 6).
 *
 * Scores the gold set in `test/fixtures/search-eval.json` against the search
 * pipeline and writes a machine-readable run record. The scoring lives in
 * `src/core/search/eval.ts`, the corpus pinning and provenance in
 * `src/core/search/eval-harness.ts` — this file is only the CLI.
 *
 * Usage:
 *   bun scripts/search-eval.ts --corpus-ref HEAD --out runs/baseline.json
 *   bun scripts/search-eval.ts --no-embed          # fast lexical-only check
 *   bun scripts/search-eval.ts --embsearch-binary ./embsearch --corpus-ref HEAD
 *
 * `--corpus-ref` checks the corpus out into a detached worktree so the run is
 * reproducible. Without it the live working tree is used and the record is
 * stamped `corpusFromWorkingTree: true` — fine for iterating, not for a
 * baseline anyone will later compare against.
 *
 * Semantic and hybrid rows need the embsearch binary. When it is unavailable
 * they degrade to lexical; the record says so in `provenance.embedder` and
 * every affected row carries a `degraded` count, so a degraded run can never
 * be mistaken for a full sweep.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { EmbsearchService } from "../src/core/embsearch/embsearch-service.js";
import { getEmbsearchStoreDir } from "../src/core/embsearch/index-meta.js";
import { EVAL_CONFIGS } from "../src/core/search/eval.js";
import { loadGoldSet, validateGoldSet } from "../src/core/search/eval-gold.js";
import {
	collectProvenance,
	type EvalRunRecord,
	formatAggregateTable,
	pinCorpus,
	runEvalSuite,
	summarizeGoldSet,
} from "../src/core/search/eval-harness.js";
import { applyLiveEdits, loadLiveEditFixture } from "../src/core/search/eval-live.js";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const packageRoot = path.resolve(scriptDir, "..");
const repoRoot = path.resolve(packageRoot, "..", "..");

function flag(name: string): string | undefined {
	const index = process.argv.indexOf(`--${name}`);
	return index !== -1 ? process.argv[index + 1] : undefined;
}
const has = (name: string): boolean => process.argv.includes(`--${name}`);

const corpusRef = flag("corpus-ref");
const outPath = flag("out");
const skipEmbed = has("no-embed");
// Explicit binary beats the on-demand release download, so a semantic run is
// reproducible from a known artifact rather than "whatever latest resolved to".
const embsearchBinary = flag("embsearch-binary");
// Builds a SECOND index whose store carries the daemon's BM25 lexical index,
// enabling the `daemon-hybrid` configs. Separate store dir because hybrid-ness
// is fixed at store creation; costs a full re-index the first time.
const withDaemonHybrid = has("daemon-hybrid");
// Applies edits to the corpus AFTER indexing and scores queries only those
// edits can answer — the grep leg's unique contribution, which the main sweep
// cannot see because its corpus matches the index exactly.
const withLiveEdits = has("live-edits");

const fixturePath = path.join(packageRoot, "test", "fixtures", "search-eval.json");
const dataset = loadGoldSet(fixturePath);

const corpus = pinCorpus(repoRoot, corpusRef);
try {
	console.error(
		`search-eval: ${dataset.length} queries, corpus=${corpus.sha.slice(0, 12)}` +
			`${corpus.fromWorkingTree ? " (WORKING TREE — not reproducible)" : " (pinned worktree)"}` +
			`${corpus.dirty ? " [dirty]" : ""}`,
	);

	// Gold must be valid against the corpus actually being scored — a stale
	// fixture depresses every number without failing anything.
	const issues = validateGoldSet(corpus.cwd, dataset);
	if (issues.length > 0) {
		console.error(`\ngold set is stale against this corpus (${issues.length} issue(s)):`);
		for (const issue of issues) console.error(`  ${issue.queryId} [${issue.path}]: ${issue.problem}`);
		console.error("\nregenerate with: bun scripts/search-eval-gold.ts --fix");
		process.exit(1);
	}

	const PROGRESS_STEP = 2000;
	let nextProgressAt = 0;
	let nextHybridProgressAt = 0;
	let service: EmbsearchService | undefined;
	if (!skipEmbed) {
		service = new EmbsearchService({
			cwd: corpus.cwd,
			thresholdBytes: 0,
			binaryPath: embsearchBinary,
			// `done` advances by the bulk batch size, so a modulo test on a
			// non-multiple silently never fires — track a threshold instead.
			onProgress: (state) => {
				if (state.phase === "indexing" && state.done >= nextProgressAt) {
					nextProgressAt = state.done + PROGRESS_STEP;
					console.error(`  embsearch: indexing ${state.done}/${state.total}`);
				}
			},
		});
		await service.start();
		const state = service.getState();
		console.error(`  embsearch: ${state.phase}${"reason" in state ? ` (${state.reason})` : ""}`);
	} else {
		console.error("  embsearch: skipped (--no-embed)");
	}

	let hybridService: EmbsearchService | undefined;
	if (withDaemonHybrid && !skipEmbed) {
		hybridService = new EmbsearchService({
			cwd: corpus.cwd,
			thresholdBytes: 0,
			binaryPath: embsearchBinary,
			storeDir: `${getEmbsearchStoreDir(corpus.cwd)}-bm25`,
			hybridStore: true,
			onProgress: (state) => {
				if (state.phase === "indexing" && state.done >= nextHybridProgressAt) {
					nextHybridProgressAt = state.done + PROGRESS_STEP;
					console.error(`  embsearch(bm25): indexing ${state.done}/${state.total}`);
				}
			},
		});
		await hybridService.start();
		const hs = hybridService.getState();
		console.error(`  embsearch(bm25): ${hs.phase}${"reason" in hs ? ` (${hs.reason})` : ""}`);
		if (!hybridService.isAvailable()) {
			console.error("  daemon-hybrid rows will be omitted: BM25 store did not come up");
			hybridService = undefined;
		}
	}

	const { aggregates, perQuery } = await runEvalSuite({
		cwd: corpus.cwd,
		dataset,
		configs: EVAL_CONFIGS,
		service,
		hybridService,
		onQuery: (index, query) => {
			if (index % 10 === 0) console.error(`  query ${index + 1}/${dataset.length} (${query.id})`);
		},
	});

	const record: EvalRunRecord & { liveEdits?: unknown } = {
		provenance: collectProvenance(
			repoRoot,
			corpus,
			corpusRef ?? "(working tree)",
			service,
			embsearchBinary,
			hybridService,
		),
		goldSet: summarizeGoldSet(dataset),
		configs: EVAL_CONFIGS,
		aggregates,
		perQuery,
	};

	if (withLiveEdits) {
		const livePath = path.join(packageRoot, "test", "fixtures", "search-eval-live.json");
		const fixture = loadLiveEditFixture(livePath);
		const applied = applyLiveEdits(corpus.cwd, fixture);
		if (applied.issues.length > 0) {
			console.error(`\nlive-edit gold did not resolve:\n  ${applied.issues.join("\n  ")}`);
			process.exit(1);
		}
		// Indexing already ran, so these files exist on disk but not in any
		// index — exactly the state an agent's own edits leave behind.
		const live = await runEvalSuite({
			cwd: corpus.cwd,
			dataset: applied.queries,
			configs: EVAL_CONFIGS,
			service,
			hybridService,
		});
		console.log(
			`\nlive-edit queries (${applied.queries.length}), written after indexing — ` +
				"index-backed retrievers score 0 by construction:",
		);
		console.log(formatAggregateTable(live.aggregates));
		record.liveEdits = { queryCount: applied.queries.length, aggregates: live.aggregates, perQuery: live.perQuery };
	}

	await service?.dispose();
	await hybridService?.dispose();

	console.log(`\n${formatAggregateTable(aggregates)}`);
	if (!record.provenance.embedder.available) {
		console.log(
			`\nNOTE: no embedding backend (${record.provenance.embedder.reason ?? record.provenance.embedder.phase}).` +
				"\nEvery semantic and hybrid row above is a degraded lexical run — this is a lexical-only baseline.",
		);
	}

	// Queries no non-degraded config places in the top 10 — the actionable list.
	const misses = perQuery.filter((q) => q.results.some((r) => !r.degraded && r.recallAt10 < 1));
	if (misses.length > 0) {
		console.log(`\nqueries below full R@10 (non-degraded configs): ${misses.length}/${perQuery.length}`);
		for (const q of misses) {
			const worst = q.results
				.filter((r) => !r.degraded)
				.map((r) => `${r.label}=${Math.round(r.recallAt10 * 100)}%`)
				.join(", ");
			console.log(`  ${q.id} (${q.class}): ${worst}`);
		}
	}

	if (outPath) {
		const resolved = path.resolve(outPath);
		mkdirSync(path.dirname(resolved), { recursive: true });
		writeFileSync(resolved, `${JSON.stringify(record, null, 2)}\n`);
		console.log(`\nrun record written to ${path.relative(repoRoot, resolved)}`);
	}
} finally {
	corpus.dispose();
}
