/**
 * Deterministic reranker over the fused top-50
 * (docs/hybrid-retrieval-design.md, step 7 of the shipping order).
 *
 * The eval gate showed fused Recall@50 well above Recall@5/10 — the right
 * candidates survive fusion but sit too deep. This reranker re-orders them
 * using evidence that is only cheap to compute *after* fusion, when there
 * are ≤50 candidates instead of thousands of lines:
 *
 *   - term coverage: how many distinct query terms appear in the candidate's
 *     actual expanded window (read from disk);
 *   - path affinity: query terms appearing in the candidate's file path —
 *     this is what lets a query like `core/search/hybrid-search.ts` rank the
 *     file itself first, which content grep alone cannot do;
 *   - fused prior: the RRF ordering, so retriever consensus still counts.
 *
 * Purely lexical-statistical and deterministic — no model, no I/O beyond
 * reading candidate windows. A cross-encoder can later replace the scoring
 * function behind the same signature; that model work belongs to
 * `kolisachint/embeddingsearchtools`, not here.
 */

import { readFileSync } from "fs";
import path from "path";
import { buildLexicalQueryPlan } from "./lexical-retriever.js";
import type { FusedCandidate } from "./types.js";

/** Weights of the scoring blend. The eval harness (scripts/search-eval.mjs)
 *  is the instrument for changing them — don't tune blind. */
const WEIGHT_FUSED_PRIOR = 0.4;
const WEIGHT_TERM_COVERAGE = 0.35;
const WEIGHT_PATH_AFFINITY = 0.25;
/** Additive bonus when the query *is* the candidate's path (or its suffix):
 *  the caller named the file, so no amount of content evidence elsewhere
 *  should outrank it. */
const EXACT_PATH_BONUS = 0.5;

export interface RerankResult {
	candidates: FusedCandidate[];
	latencyMs: number;
}

export function rerankCandidates(query: string, candidates: readonly FusedCandidate[], cwd: string): RerankResult {
	const startedMs = Date.now();
	const plan = buildLexicalQueryPlan(query);
	if (!plan || candidates.length < 2) {
		return { candidates: [...candidates], latencyMs: Date.now() - startedMs };
	}
	const terms = plan.terms;
	const queryPath = query.trim().toLowerCase();

	const fileCache = new Map<string, string[] | undefined>();
	const readLines = (rel: string): string[] | undefined => {
		if (!fileCache.has(rel)) {
			try {
				const content = readFileSync(path.resolve(cwd, rel), "utf-8");
				fileCache.set(rel, content.toLowerCase().split("\n"));
			} catch {
				fileCache.set(rel, undefined);
			}
		}
		return fileCache.get(rel);
	};

	const scored = candidates.map((candidate, index) => {
		// Fused prior: normalized RRF ordering, 1 for the top candidate.
		const fusedPrior = 1 - index / candidates.length;

		const lines = readLines(candidate.path);
		let termCoverage = 0;
		if (lines && terms.length > 0) {
			const window = lines
				.slice(Math.max(0, candidate.startLine - 1), Math.min(lines.length, candidate.endLine))
				.join("\n");
			termCoverage = terms.filter((t) => window.includes(t)).length / terms.length;
		}

		const lowerPath = candidate.path.toLowerCase();
		// A quoted phrase rarely names a file; split it into path-ish tokens so
		// `"token budget exceeded"` still gets partial path credit.
		const pathTerms = terms.length === 1 ? terms[0].split(/[^a-z0-9_$]+/).filter((t) => t.length >= 3) : terms;
		const pathAffinity =
			pathTerms.length > 0 ? pathTerms.filter((t) => lowerPath.includes(t)).length / pathTerms.length : 0;

		const exactPath =
			queryPath.length >= 3 && (lowerPath === queryPath || lowerPath.endsWith(`/${queryPath}`)) ? 1 : 0;

		const score =
			WEIGHT_FUSED_PRIOR * fusedPrior +
			WEIGHT_TERM_COVERAGE * termCoverage +
			WEIGHT_PATH_AFFINITY * pathAffinity +
			EXACT_PATH_BONUS * exactPath;
		return { candidate, index, score };
	});

	// Stable, deterministic: score desc, fused order as tie-break.
	scored.sort((a, b) => b.score - a.score || a.index - b.index);
	return { candidates: scored.map((s) => s.candidate), latencyMs: Date.now() - startedMs };
}
