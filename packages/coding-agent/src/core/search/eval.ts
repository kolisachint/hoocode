/**
 * Retrieval evaluation gate (docs/hybrid-retrieval-design.md, step 6 of the
 * shipping order).
 *
 * Measures Recall@K for lexical, semantic, and hybrid retrieval across an
 * RRF `k` sweep, against a gold set keyed by **path + optional line range
 * matched by span overlap** — never by chunkId, which is only stable per
 * index build. Recall@50 doubles as the reranker gate: a gold span that
 * never reaches the fused top-50 cannot be rescued by any reranker.
 *
 * Driven by `scripts/search-eval.mjs`; the scoring logic lives here so it is
 * unit-testable without an embedding index.
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
}

/** The sweep from the design doc — single retrievers, hybrid across k, the
 *  routed auto mode — plus reranked (`+rr`) variants for the step 7 gate. */
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
];

/** Candidates fetched per eval query — deep enough for the reranker gate. */
const EVAL_FETCH_LIMIT = 50;

export function spanMatchesGold(span: CandidateSpan, gold: EvalGoldSpan): boolean {
	if (span.path !== gold.path) return false;
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

export interface EvalQueryResult {
	label: string;
	resolvedMode: ResolvedSearchMode;
	degraded: boolean;
	recallAt5: number;
	recallAt10: number;
	recallAt50: number;
}

export async function evaluateQuery(
	cwd: string,
	evalQuery: EvalQuery,
	configs: readonly EvalConfig[] = EVAL_CONFIGS,
	service?: EmbsearchService,
): Promise<EvalQueryResult[]> {
	const results: EvalQueryResult[] = [];
	for (const config of configs) {
		const retrieved = await retrieveCandidates({
			cwd,
			query: evalQuery.query,
			mode: config.mode,
			rrfK: config.rrfK,
			rerank: config.rerank ?? false,
			limit: EVAL_FETCH_LIMIT,
			service,
		});
		results.push({
			label: config.label,
			resolvedMode: retrieved.resolvedMode,
			degraded: retrieved.degradedReason !== undefined,
			recallAt5: recallAtK(retrieved.candidates, evalQuery.gold, 5),
			recallAt10: recallAtK(retrieved.candidates, evalQuery.gold, 10),
			recallAt50: recallAtK(retrieved.candidates, evalQuery.gold, 50),
		});
	}
	return results;
}
