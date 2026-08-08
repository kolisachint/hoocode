/**
 * Retrieval evaluation gate (docs/hybrid-retrieval-design.md, step 6 of the
 * shipping order).
 *
 * Measures Recall@K and MRR for lexical, semantic, and hybrid retrieval across
 * an RRF `k` sweep, against a gold set keyed by **path + line range matched by
 * span overlap** — never by chunkId, which is only stable per index build.
 * Recall@50 doubles as the reranker gate: a gold span that never reaches the
 * fused top-50 cannot be rescued by any reranker.
 *
 * Rank-sensitive metrics are the point of Recall@1 and MRR: an agent reads the
 * top result or two, so "the gold span is somewhere in the top 5" understates
 * how much ordering matters. Recall@5/10/50 are kept for continuity with the
 * numbers already published in the design note.
 *
 * Driven by `scripts/search-eval.ts` via `eval-harness.ts`; the scoring logic
 * lives here so it is unit-testable without an embedding index.
 */

import type { EmbsearchService } from "../embsearch/embsearch-service.js";
import { retrieveCandidates } from "./hybrid-search.js";
import type { CandidateSpan, ResolvedSearchMode, SearchMode } from "./types.js";

export interface EvalGoldSpan {
	/** Repo-relative POSIX path. */
	path: string;
	/** 1-based inclusive; omit both to accept any span in the file. */
	startLine?: number;
	endLine?: number;
	/**
	 * Literal source text that must occur inside `[startLine, endLine]`. Not
	 * used for scoring — it is how the gold set survives the corpus moving
	 * underneath it: `scripts/search-eval-gold.ts` re-resolves the line range
	 * from this anchor, and the fixture test fails when an anchor no longer
	 * sits inside its recorded range. Omitted for `scope: "file"` entries.
	 */
	anchor?: string;
	/**
	 * `"span"` (default) scores by overlap with the recorded line range;
	 * `"file"` accepts any span in the file. File scope is deliberate for
	 * path-class queries, where the whole file *is* the answer — it is
	 * recorded explicitly so a file-level score is never mistaken for a
	 * span-level one.
	 */
	scope?: "span" | "file";
}

export interface EvalQuery {
	id: string;
	/** Query class from the design doc (exact-symbol, path, error-fragment,
	 *  conceptual, cross-file, boundary). Reporting only. */
	class: string;
	query: string;
	gold: EvalGoldSpan[];
}

export interface EvalConfig {
	label: string;
	mode: SearchMode;
	rrfK?: number;
	rerank?: boolean;
	/**
	 * Score this config against a daemon-side hybrid store (BM25 fused with
	 * vectors inside the Rust daemon) instead of the dense-only index. Skipped
	 * when the harness has no hybrid service, so records without one simply
	 * omit the row rather than silently scoring it as plain semantic.
	 */
	daemonHybrid?: boolean;
	/**
	 * Fetch the daemon's BM25 index as a separate leg and fuse it here with
	 * dense and grep. Like `daemonHybrid` it needs a hybrid store, so it is
	 * skipped when the harness has no hybrid service.
	 */
	bm25Leg?: boolean;
}

/**
 * The sweep from the design doc — single retrievers, hybrid across k, the
 * routed auto mode — plus reranked (`+rr`) variants for the step 7 gate.
 *
 * Exported because `scripts/search-eval.ts` and `test/search-eval.test.ts`
 * both consume it. It previously lost its `export` to a knip-driven
 * dead-export sweep (02efaab): the only importer was a `.mjs` script reading
 * from `dist/`, which static analysis cannot see, and the eval gate silently
 * stopped running. The TypeScript importers are the fix — do not "clean up"
 * this export without checking them.
 */
export const EVAL_CONFIGS: readonly EvalConfig[] = [
	{ label: "lexical", mode: "lexical" },
	{ label: "semantic", mode: "semantic" },
	{ label: "hybrid k=0", mode: "hybrid", rrfK: 0 },
	{ label: "hybrid k=2", mode: "hybrid", rrfK: 2 },
	{ label: "hybrid k=10", mode: "hybrid", rrfK: 10 },
	{ label: "hybrid k=60", mode: "hybrid", rrfK: 60 },
	{ label: "auto", mode: "auto" },
	{ label: "lexical +rr", mode: "lexical", rerank: true },
	{ label: "semantic +rr", mode: "semantic", rerank: true },
	{ label: "hybrid k=2 +rr", mode: "hybrid", rrfK: 2, rerank: true },
	{ label: "hybrid k=60 +rr", mode: "hybrid", rrfK: 60, rerank: true },
	{ label: "auto +rr", mode: "auto", rerank: true },
	// Is BM25 a better lexical leg than ripgrep? These two run the daemon's own
	// vector+BM25 fusion and no ripgrep at all, so comparing them against
	// "semantic" isolates what BM25 adds, and against "hybrid k=2" compares the
	// two lexical legs at the system level.
	{ label: "daemon-hybrid", mode: "semantic", daemonHybrid: true },
	{ label: "daemon-hybrid +rr", mode: "semantic", rerank: true, daemonHybrid: true },
	// Three-way fusion: dense + BM25 + grep, all fused here so `k` is ours and
	// the per-leg ranks survive into the trace. `bm25+dense` isolates what the
	// grep leg still contributes once BM25 is present.
	{ label: "bm25+dense +rr", mode: "semantic", rerank: true, bm25Leg: true },
	{ label: "3-way +rr", mode: "hybrid", rerank: true, bm25Leg: true },
];

/** Candidates fetched per eval query — deep enough for the reranker gate. */
const EVAL_FETCH_LIMIT = 50;

export function spanMatchesGold(span: CandidateSpan, gold: EvalGoldSpan): boolean {
	if (span.path !== gold.path) return false;
	if (gold.scope === "file") return true;
	if (gold.startLine === undefined || gold.endLine === undefined) return true;
	return span.startLine <= gold.endLine && span.endLine >= gold.startLine;
}

/** Fraction of gold spans matched by at least one of the top-`k` candidates. */
export function recallAtK(candidates: readonly CandidateSpan[], gold: readonly EvalGoldSpan[], k: number): number {
	if (gold.length === 0) return 0;
	const top = candidates.slice(0, k);
	let matched = 0;
	for (const g of gold) {
		if (top.some((c) => spanMatchesGold(c, g))) matched++;
	}
	return matched / gold.length;
}

/**
 * Mean reciprocal rank, averaged over gold spans: for each gold span, `1/rank`
 * of the first candidate matching it (0 when it never appears).
 *
 * Averaging per gold span rather than taking the single first hit keeps
 * multi-span queries (the cross-file class) honest — finding one of two
 * required sites should not score like finding both. For single-span queries
 * this reduces to textbook MRR.
 */
export function mrr(candidates: readonly CandidateSpan[], gold: readonly EvalGoldSpan[]): number {
	if (gold.length === 0) return 0;
	let total = 0;
	for (const g of gold) {
		const index = candidates.findIndex((c) => spanMatchesGold(c, g));
		if (index >= 0) total += 1 / (index + 1);
	}
	return total / gold.length;
}

export interface EvalQueryResult {
	label: string;
	resolvedMode: ResolvedSearchMode;
	degraded: boolean;
	recallAt1: number;
	recallAt5: number;
	recallAt10: number;
	recallAt50: number;
	mrr: number;
}

export async function evaluateQuery(
	cwd: string,
	evalQuery: EvalQuery,
	configs: readonly EvalConfig[] = EVAL_CONFIGS,
	service?: EmbsearchService,
	hybridService?: EmbsearchService,
): Promise<EvalQueryResult[]> {
	const results: EvalQueryResult[] = [];
	for (const config of configs) {
		// A daemon-hybrid config against the plain store would error in the
		// daemon (`query_hybrid requires a hybrid store`); omit the row instead.
		if ((config.daemonHybrid || config.bm25Leg) && !hybridService) continue;
		const retrieved = await retrieveCandidates({
			cwd,
			query: evalQuery.query,
			mode: config.mode,
			rrfK: config.rrfK,
			rerank: config.rerank ?? false,
			limit: EVAL_FETCH_LIMIT,
			service: config.daemonHybrid || config.bm25Leg ? hybridService : service,
			daemonHybrid: config.daemonHybrid,
			bm25Leg: config.bm25Leg,
		});
		results.push({
			label: config.label,
			resolvedMode: retrieved.resolvedMode,
			degraded: retrieved.degradedReason !== undefined,
			recallAt1: recallAtK(retrieved.candidates, evalQuery.gold, 1),
			recallAt5: recallAtK(retrieved.candidates, evalQuery.gold, 5),
			recallAt10: recallAtK(retrieved.candidates, evalQuery.gold, 10),
			recallAt50: recallAtK(retrieved.candidates, evalQuery.gold, 50),
			mrr: mrr(retrieved.candidates, evalQuery.gold),
		});
	}
	return results;
}
