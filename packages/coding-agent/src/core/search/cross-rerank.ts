/**
 * Cross-encoder reranking via the embsearch daemon.
 *
 * The deterministic reranker in `rerank.ts` scores candidates with lexical
 * evidence — term coverage, path affinity, whether the window declares a query
 * term. That took Recall@1 from 0.10 to 0.24 and then stopped paying: a
 * term-proximity signal aimed at the classes it still handles worst moved
 * nothing (p = 1.00 on every metric).
 *
 * What is left is a ranking problem the lexical view cannot see. Across the
 * 62-query gold set the right span reaches the fused top-50 far more often
 * than the top-10, so the candidates are in hand and merely ordered badly. A
 * cross-encoder reads the query and the candidate *together*, which is exactly
 * the evidence term counting lacks.
 *
 * It is also far more expensive — one model pass per candidate, with no
 * precomputation possible — so it runs over a shortlist and its depth is
 * capped separately from the fused window.
 */

import type { EmbsearchService } from "../embsearch/embsearch-service.js";
import { readCandidateWindows } from "./rerank.js";
import type { FusedCandidate } from "./types.js";

/**
 * Candidates sent to the cross-encoder per query.
 *
 * Each one is a model pass, so this is a latency dial, not a quality dial:
 * every candidate past here keeps its fused order rather than being scored.
 */
export const CROSS_ENCODER_DEPTH = 30;

export interface CrossRerankResult {
	candidates: FusedCandidate[];
	latencyMs: number;
	/** How many candidates the model actually scored. */
	scored: number;
}

/**
 * Reorder `candidates` by cross-encoder relevance.
 *
 * Only the first {@link CROSS_ENCODER_DEPTH} are scored; the remainder keep
 * their incoming order and follow. Candidates whose window cannot be read are
 * left unscored for the same reason — there is no text to give the model, and
 * inventing one would score a fiction.
 */
export async function crossEncoderRerank(
	query: string,
	candidates: readonly FusedCandidate[],
	cwd: string,
	service: EmbsearchService,
): Promise<CrossRerankResult> {
	const startedMs = Date.now();
	if (candidates.length < 2) {
		return { candidates: [...candidates], latencyMs: Date.now() - startedMs, scored: 0 };
	}

	const head = candidates.slice(0, CROSS_ENCODER_DEPTH);
	const tail = candidates.slice(CROSS_ENCODER_DEPTH);
	const windows = readCandidateWindows(head, cwd);

	const passages: Array<{ id: string; text: string }> = [];
	const byId = new Map<string, FusedCandidate>();
	head.forEach((candidate, i) => {
		const text = windows[i];
		if (text === undefined) return;
		// Fused ids are unique per query, so they round-trip as passage ids.
		passages.push({ id: candidate.id, text });
		byId.set(candidate.id, candidate);
	});

	if (passages.length === 0) {
		return { candidates: [...candidates], latencyMs: Date.now() - startedMs, scored: 0 };
	}

	const scored = await service.rerank(query, passages, passages.length);

	// Reassemble: scored candidates in model order, then anything the model did
	// not see, in fused order. An id the daemon did not return would otherwise
	// vanish from the results entirely.
	const ordered: FusedCandidate[] = [];
	const seen = new Set<string>();
	for (const result of scored) {
		const candidate = byId.get(result.id);
		if (candidate && !seen.has(result.id)) {
			ordered.push(candidate);
			seen.add(result.id);
		}
	}
	for (const candidate of head) {
		if (!seen.has(candidate.id)) ordered.push(candidate);
	}
	ordered.push(...tail);

	return { candidates: ordered, latencyMs: Date.now() - startedMs, scored: passages.length };
}
