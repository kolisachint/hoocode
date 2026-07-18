/**
 * Reciprocal Rank Fusion — the transparent, rank-only recall layer of hybrid
 * search (docs/hybrid-retrieval-design.md, Decision 2).
 *
 * Rank-only on purpose: BM25 and cosine scores are not comparable across
 * retrievers, so raw scores are carried through as diagnostics but never
 * enter the fused score.
 */

import type { FusedHit, RankedHit } from "./types.js";

/**
 * Default RRF constant. The literature folklore default is 60; the eval gate
 * (scripts/search-eval.mjs, 12-query gold set) measured k ∈ {0, 2} beating
 * k = 60 on every differing query, twice, with reranking on top of either —
 * small k keeps fusion top-heavy toward each retriever's best hits, and the
 * reranker corrects the tail. Small sample: re-sweep when the gold set grows.
 */
export const DEFAULT_RRF_K = 2;

/**
 * Fuse ranked lists into one deterministic ordering by summed `1/(k + rank)`.
 *
 * Ties break by number of agreeing retrievers, then lexicographic id, so the
 * same inputs always produce the same context.
 *
 * Duplicate `source:id` pairs within one list are counted once (best rank
 * wins). The adapter dedupes upstream, so this guard should never fire in
 * practice — it exists so a misbehaving retriever cannot inflate its vote.
 */
export function rrfFuse(lists: readonly (readonly RankedHit[])[], k = DEFAULT_RRF_K): FusedHit[] {
	if (!Number.isFinite(k) || k < 0) {
		throw new Error(`RRF k must be a finite non-negative number; got ${k}`);
	}

	const acc = new Map<string, FusedHit>();

	for (const list of lists) {
		// Collapse duplicates to their best rank first, so the single vote a
		// duplicated id gets is cast at the best rank regardless of emit order.
		const collapsed = new Map<string, RankedHit>();
		for (const hit of list) {
			if (!Number.isInteger(hit.rank) || hit.rank < 1) {
				throw new Error(`RRF rank must be a positive integer; got ${hit.rank}`);
			}
			const dedupeKey = `${hit.source}:${hit.id}`;
			const existing = collapsed.get(dedupeKey);
			if (!existing || hit.rank < existing.rank) collapsed.set(dedupeKey, hit);
		}

		for (const hit of collapsed.values()) {
			let current = acc.get(hit.id);
			if (!current) {
				current = { id: hit.id, rrfScore: 0, ranks: {}, rawScores: {} };
				acc.set(hit.id, current);
			}

			current.rrfScore += 1 / (k + hit.rank);

			const oldRank = current.ranks[hit.source];
			if (oldRank === undefined || hit.rank < oldRank) {
				current.ranks[hit.source] = hit.rank;
				// rawScores follows the best rank; a best-ranked hit without a
				// score leaves any earlier score in place rather than erasing it.
				if (hit.score !== undefined) current.rawScores[hit.source] = hit.score;
			}
		}
	}

	return [...acc.values()].sort(
		(a, b) =>
			b.rrfScore - a.rrfScore ||
			Object.keys(b.ranks).length - Object.keys(a.ranks).length ||
			a.id.localeCompare(b.id),
	);
}
