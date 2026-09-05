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
 *   bun scripts/search-eval.ts --embsearch-binary ./embsearch --model-dir ./pack/bge-small
 *   bun scripts/search-eval.ts --corpus-ref HEAD --fast 6000   # screening sweep
 *
 * `--fast <chunks>` shrinks the corpus to a chunk budget, keeping every
 * gold-bearing file, so a model sweep costs minutes instead of hours. It makes
 * retrieval easier and inflates every metric, so it compares arms to each
 * other and never states an absolute; records carry `corpusSubsample` to keep
 * the two kinds apart.
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

import { createHash } from "node:crypto";
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
// Overrides the model bundled in the binary, so one binary can score several
// embedding models. Requires an onnx build and a directory holding
// `model.onnx`, `tokenizer.json` and `model.json`.
const modelDir = flag("model-dir");
/**
 * Store suffix isolating one model's index from another's.
 *
 * The store directory is keyed by the corpus path alone, so without this two
 * model arms on the same corpus would land in the same store — and the second
 * would find vectors built by the first. The daemon refuses to open a store
 * whose recorded model disagrees, so the arm would not silently mix models; it
 * would fail to start at all. Either way the suffix is what makes back-to-back
 * arms work.
 *
 * Basename for legibility, plus a hash of the resolved path so two directories
 * that happen to share a name cannot share a store. Empty without `--model-dir`,
 * so an ordinary run reuses exactly the store it always did.
 */
/**
 * Chunker character cap, for arms that sweep the window.
 *
 * Changing this changes what every vector is, so it is folded into the store
 * key below and recorded in provenance. Note what it does *not* do: raise the
 * number of tokens a model will read. At cap 2000 the median chunk is 581
 * tokens against bge-small's 512-token window, so two thirds of chunks are
 * truncated and 17.6% of the corpus's tokens are dropped — a cap sweep past
 * ~1100 chars is partly a truncation experiment, whatever it is labelled.
 */
const chunkMaxChars = flag("chunk-max-chars") ? Number(flag("chunk-max-chars")) : undefined;
if (chunkMaxChars !== undefined && (!Number.isFinite(chunkMaxChars) || chunkMaxChars <= 0)) {
	console.error("--chunk-max-chars takes a positive character cap, e.g. --chunk-max-chars 1500");
	process.exit(1);
}
const storeSuffix =
	(modelDir
		? `-${path.basename(path.resolve(modelDir))}-${createHash("sha256")
				.update(path.resolve(modelDir))
				.digest("hex")
				.slice(0, 6)}`
		: "") +
	// The cap belongs in the key for the same reason the model does: vectors
	// built under a different one are a different index, and nothing in the
	// store says which cap produced it.
	(chunkMaxChars ? `-c${chunkMaxChars}` : "");
// Enables the `daemon-hybrid` configs, which need a store carrying the
// daemon's BM25 lexical index. Hybrid-ness is fixed at store creation, so this
// selects which kind of store the run builds; see `shareOneStore` below for
// why it no longer builds two.
const withDaemonHybrid = has("daemon-hybrid");
// Restores the old two-store layout: a dense-only index plus a separate hybrid
// one. Kept as an escape hatch so the equivalence that `shareOneStore` relies
// on can be re-verified against a future daemon rather than taken on trust.
const separateStores = has("separate-stores");
// Applies edits to the corpus AFTER indexing and scores queries only those
// edits can answer — the grep leg's unique contribution, which the main sweep
// cannot see because its corpus matches the index exactly.
const withLiveEdits = has("live-edits");
// Scores only the named configs. Iterating on the reranker means re-running
// five rows, not nineteen; the record still says which configs it holds, so a
// filtered run can never be mistaken for a full sweep.
const configFilter = flag("configs")
	?.split(",")
	.map((s) => s.trim())
	.filter(Boolean);
/**
 * Screening mode: shrink the corpus to roughly this many chunks.
 *
 * A full arm costs about 5 minutes of indexing on MiniLM, 22 on bge-small and
 * nearly two hours on nomic, which is too slow to iterate against. This trades
 * the thing that makes those numbers expensive — corpus size — for turnaround.
 *
 * It is not a cheaper version of the real eval. Every gold-bearing file is
 * kept but most distractors are gone, so retrieval gets easier and every
 * metric reads high; more queries also tie, which costs the paired sign test
 * the power it was already short of. Use it to compare arms against each other
 * under identical conditions, never to make a claim about absolute quality.
 * The record carries `corpusSubsample` so the two kinds cannot be confused.
 */
const fastChunks = has("fast") ? Number(flag("fast") ?? 6000) : undefined;
if (fastChunks !== undefined && (!Number.isFinite(fastChunks) || fastChunks <= 0)) {
	console.error("--fast takes a positive chunk budget, e.g. --fast 6000");
	process.exit(1);
}
// Fixed rather than exposed: two arms drawing different distractors would not
// be comparable, and that is the whole purpose of this mode. Change it only to
// check that a result is not an artifact of one particular draw.
const fastSeed = Number(flag("fast-seed") ?? 1);

const fixturePath = path.join(packageRoot, "test", "fixtures", "search-eval.json");
const dataset = loadGoldSet(fixturePath);

const configs = configFilter ? EVAL_CONFIGS.filter((c) => configFilter.includes(c.label)) : EVAL_CONFIGS;
if (configs.length === 0) {
	console.error(`--configs matched nothing. Known labels:\n  ${EVAL_CONFIGS.map((c) => c.label).join("\n  ")}`);
	process.exit(1);
}

const corpus = pinCorpus(
	repoRoot,
	corpusRef,
	fastChunks === undefined
		? undefined
		: {
				targetChunks: fastChunks,
				// Every file the gold set points at, so no query is made
				// unanswerable by the draw.
				keepRelPaths: [...new Set(dataset.flatMap((q) => q.gold.map((g) => g.path)))],
				seed: fastSeed,
			},
);
try {
	console.error(
		`search-eval: ${dataset.length} queries, corpus=${corpus.sha.slice(0, 12)}` +
			`${corpus.fromWorkingTree ? " (WORKING TREE — not reproducible)" : " (pinned worktree)"}` +
			`${corpus.dirty ? " [dirty]" : ""}`,
	);
	if (corpus.subsample) {
		const s = corpus.subsample;
		console.error(
			`  SUBSAMPLED: ${s.chunkCount} chunks from ${s.filesKept} files ` +
				`(${s.goldFilesKept} gold-bearing, ${s.filesDropped} files dropped, seed ${s.seed})`,
		);
		console.error("  metrics are inflated by the smaller distractor pool — compare only to another --fast run");
	}

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
	/**
	 * One store serving both roles, instead of a dense-only index plus a hybrid
	 * one.
	 *
	 * Building both embedded the whole corpus twice to produce the *same*
	 * vectors — half of every arm's indexing budget spent on a duplicate. A
	 * hybrid store answers `retriever: "dense"` from the same vector index a
	 * dense-only store uses, so the dense rows are unchanged: verified over
	 * 1,500 chunks and six queries, where both stores returned bit-identical
	 * ids and scores at k=10.
	 *
	 * This is what the old comment here worried about ("leave no dense-only
	 * baseline to compare the BM25 rows against"). The baseline survives
	 * because it was never a property of the *store* — the dense configs still
	 * query the dense leg alone, and only the `daemon-hybrid`/`bm25Leg` configs
	 * ask for BM25. `--separate-stores` restores the two-store layout to
	 * re-check that if the daemon's query path ever changes.
	 */
	const shareOneStore = withDaemonHybrid && !skipEmbed && !separateStores;
	// Indexing and query cost are billed separately. A model comparison is
	// mostly a question about indexing, and one wall-clock total hides it behind
	// 62 queries' worth of work — which is exactly why the last run could only
	// report "17 -> 60 min total" and could not say how much of that was either.
	let indexSeconds = 0;
	let service: EmbsearchService | undefined;
	if (!skipEmbed) {
		const startedAt = Date.now();
		service = new EmbsearchService({
			cwd: corpus.cwd,
			thresholdBytes: 0,
			binaryPath: embsearchBinary,
			modelDir,
			chunkMaxChars,
			storeDir: shareOneStore
				? `${getEmbsearchStoreDir(corpus.cwd)}${storeSuffix}-bm25`
				: `${getEmbsearchStoreDir(corpus.cwd)}${storeSuffix}`,
			hybridStore: shareOneStore,
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
		indexSeconds += (Date.now() - startedAt) / 1000;
		const state = service.getState();
		console.error(`  embsearch: ${state.phase}${"reason" in state ? ` (${state.reason})` : ""}`);
	} else {
		console.error("  embsearch: skipped (--no-embed)");
	}

	let hybridService: EmbsearchService | undefined;
	if (shareOneStore) {
		hybridService = service;
		if (hybridService?.isAvailable()) {
			console.error("  embsearch(bm25): served by the same store (one index, both roles)");
		} else {
			console.error("  daemon-hybrid rows will be omitted: hybrid store did not come up");
			hybridService = undefined;
		}
	} else if (withDaemonHybrid && !skipEmbed) {
		const startedAt = Date.now();
		hybridService = new EmbsearchService({
			cwd: corpus.cwd,
			thresholdBytes: 0,
			binaryPath: embsearchBinary,
			modelDir,
			chunkMaxChars,
			storeDir: `${getEmbsearchStoreDir(corpus.cwd)}${storeSuffix}-bm25`,
			hybridStore: true,
			onProgress: (state) => {
				if (state.phase === "indexing" && state.done >= nextHybridProgressAt) {
					nextHybridProgressAt = state.done + PROGRESS_STEP;
					console.error(`  embsearch(bm25): indexing ${state.done}/${state.total}`);
				}
			},
		});
		await hybridService.start();
		indexSeconds += (Date.now() - startedAt) / 1000;
		const hs = hybridService.getState();
		console.error(`  embsearch(bm25): ${hs.phase}${"reason" in hs ? ` (${hs.reason})` : ""}`);
		if (!hybridService.isAvailable()) {
			console.error("  daemon-hybrid rows will be omitted: BM25 store did not come up");
			hybridService = undefined;
		}
	}

	const querySeconds0 = Date.now();
	const { aggregates, perQuery } = await runEvalSuite({
		cwd: corpus.cwd,
		dataset,
		configs,
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
			modelDir,
			{
				indexSeconds: Number(indexSeconds.toFixed(1)),
				querySeconds: Number(((Date.now() - querySeconds0) / 1000).toFixed(1)),
				sharedStore: shareOneStore,
			},
			chunkMaxChars,
		),
		goldSet: summarizeGoldSet(dataset),
		configs,
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
			configs,
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
