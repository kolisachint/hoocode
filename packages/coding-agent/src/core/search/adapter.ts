/**
 * Grep→chunk adapter (docs/hybrid-retrieval-design.md, Decision 3).
 *
 * Fusion needs a shared identity, and the embedding side already has one:
 * per-build chunk ids with line ranges in the sidecar. This adapter turns raw
 * grep line-hits into a gap-free ranked list of those ids:
 *
 *   1. map each line-hit to its enclosing indexed chunk;
 *   2. synthesize `rel#L<line>` fallback ids for hits in files the index
 *      does not cover, so lexical-only files still enter fusion;
 *   3. collapse multiple hits in the same chunk to one candidate;
 *   4. re-rank 1..N after collapsing — RRF's `1/(k + rank)` assumes gap-free
 *      ranks, so feeding it raw line-hit ranks would quietly penalize files
 *      with many matches.
 */

import type { CandidateSpan, RankedHit } from "./types.js";

export interface GrepLineHit {
	/** Repo-relative POSIX path. */
	rel: string;
	/** 1-based line number of the match. */
	line: number;
}

/** Resolves a line to its enclosing indexed chunk, or undefined when the file
 *  (or line) is not covered by the embedding index. */
export type ChunkLookup = (rel: string, line: number) => (CandidateSpan & { id: string }) | undefined;

/** Context window around a fallback (unindexed) hit, mirroring chunk-ish size
 *  without pretending to know real chunk boundaries. */
const FALLBACK_SPAN_LINES = 5;

export interface AdaptedGrepHits {
	hits: RankedHit[];
	/** Span for every emitted id, for post-fusion expansion. */
	spans: Map<string, CandidateSpan>;
}

/**
 * Adapt grep line-hits (in retriever output order) into a ranked, collapsed,
 * gap-free candidate list. Order of first appearance decides rank — grep has
 * no relevance ordering, so appearance order is the deterministic choice.
 */
export function adaptGrepHits(lineHits: readonly GrepLineHit[], lookupChunk?: ChunkLookup): AdaptedGrepHits {
	const hits: RankedHit[] = [];
	const spans = new Map<string, CandidateSpan>();

	for (const { rel, line } of lineHits) {
		const chunk = lookupChunk?.(rel, line);
		const id = chunk ? chunk.id : `${rel}#L${line}`;
		if (spans.has(id)) continue; // collapse: same chunk (or same fallback line)

		spans.set(
			id,
			chunk
				? { path: chunk.path, startLine: chunk.startLine, endLine: chunk.endLine }
				: {
						path: rel,
						startLine: Math.max(1, line - FALLBACK_SPAN_LINES),
						// May exceed the file length; span readers clamp.
						endLine: line + FALLBACK_SPAN_LINES,
					},
		);
		hits.push({ id, rank: hits.length + 1, source: "grep" });
	}

	return { hits, spans };
}
