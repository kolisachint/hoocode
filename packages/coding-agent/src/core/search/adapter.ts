/**
 * Grep→chunk adapter (docs/hybrid-retrieval-design.md, Decision 3).
 *
 * Fusion needs a shared identity, and the embedding side already has one:
 * per-build chunk ids with line ranges in the sidecar. This adapter turns raw
 * grep line-hits into a gap-free ranked list of those ids:
 *
 *   1. map each line-hit to its enclosing indexed chunk;
 *   2. coalesce unindexed hits into per-file clusters with synthetic
 *      `rel#L<line>` ids, so lexical-only files still enter fusion without
 *      one file's adjacent lines flooding the candidate list;
 *   3. collapse multiple hits in the same chunk/cluster to one candidate;
 *   4. rank candidates by evidence — distinct query terms matched, then hit
 *      count — and cap candidates per file, before assigning gap-free ranks.
 *      RRF's `1/(k + rank)` assumes gap-free ranks, and grep output order
 *      carries no relevance, so evidence-based ordering here is what makes
 *      the lexical list a real ranking rather than a directory walk.
 */

import type { CandidateSpan, RankedHit } from "./types.js";

export interface GrepLineHit {
	/** Repo-relative POSIX path. */
	rel: string;
	/** 1-based line number of the match. */
	line: number;
	/** Lowercased query terms present on this line (retriever-computed). */
	terms?: readonly string[];
}

/** Resolves a line to its enclosing indexed chunk, or undefined when the file
 *  (or line) is not covered by the embedding index. */
export type ChunkLookup = (rel: string, line: number) => (CandidateSpan & { id: string }) | undefined;

/** Context padding around a fallback (unindexed) cluster, mirroring chunk-ish
 *  size without pretending to know real chunk boundaries. */
const FALLBACK_PAD_LINES = 5;
/** Unindexed hits within this many lines of each other merge into one cluster. */
const FALLBACK_MERGE_GAP = 10;
/** Hard cap on a cluster's own line span, so a file with hits every few lines
 *  still splits into readable windows. */
const FALLBACK_MAX_CLUSTER_LINES = 40;
/** Max candidates one file may contribute, to keep the fused list diverse. */
const PER_FILE_CANDIDATE_CAP = 8;

export interface AdaptedGrepHits {
	hits: RankedHit[];
	/** Span for every emitted id, for post-fusion expansion. */
	spans: Map<string, CandidateSpan>;
}

interface Candidate {
	id: string;
	span: CandidateSpan;
	terms: Set<string>;
	hitCount: number;
	/** Index of the candidate's earliest contributing hit — the deterministic
	 *  last-resort tie-break. */
	firstSeen: number;
}

export function adaptGrepHits(lineHits: readonly GrepLineHit[], lookupChunk?: ChunkLookup): AdaptedGrepHits {
	const candidates = new Map<string, Candidate>();
	// Unmapped hits, grouped per file for clustering.
	const unmapped = new Map<string, Array<{ line: number; terms?: readonly string[]; index: number }>>();

	lineHits.forEach(({ rel, line, terms }, index) => {
		const chunk = lookupChunk?.(rel, line);
		if (!chunk) {
			let list = unmapped.get(rel);
			if (!list) {
				list = [];
				unmapped.set(rel, list);
			}
			list.push({ line, terms, index });
			return;
		}
		const existing = candidates.get(chunk.id);
		if (existing) {
			existing.hitCount++;
			for (const t of terms ?? []) existing.terms.add(t);
		} else {
			candidates.set(chunk.id, {
				id: chunk.id,
				span: { path: chunk.path, startLine: chunk.startLine, endLine: chunk.endLine },
				terms: new Set(terms),
				hitCount: 1,
				firstSeen: index,
			});
		}
	});

	// Cluster unindexed hits: sort by line, merge while the gap stays small
	// and the cluster stays readable.
	for (const [rel, hits] of unmapped) {
		hits.sort((a, b) => a.line - b.line || a.index - b.index);
		let cluster: typeof hits = [];
		const flush = () => {
			if (cluster.length === 0) return;
			const first = cluster[0].line;
			const last = cluster[cluster.length - 1].line;
			const candidate: Candidate = {
				id: `${rel}#L${first}`,
				span: { path: rel, startLine: Math.max(1, first - FALLBACK_PAD_LINES), endLine: last + FALLBACK_PAD_LINES },
				terms: new Set(cluster.flatMap((h) => [...(h.terms ?? [])])),
				hitCount: cluster.length,
				firstSeen: Math.min(...cluster.map((h) => h.index)),
			};
			candidates.set(candidate.id, candidate);
			cluster = [];
		};
		for (const hit of hits) {
			const clusterStart = cluster[0]?.line;
			const prevLine = cluster[cluster.length - 1]?.line;
			if (
				cluster.length > 0 &&
				(hit.line - prevLine > FALLBACK_MERGE_GAP || hit.line - clusterStart > FALLBACK_MAX_CLUSTER_LINES)
			) {
				flush();
			}
			cluster.push(hit);
		}
		flush();
	}

	// Evidence-based ordering: distinct terms, hit count, earliest appearance,
	// then id — fully deterministic.
	const ordered = [...candidates.values()].sort(
		(a, b) =>
			b.terms.size - a.terms.size ||
			b.hitCount - a.hitCount ||
			a.firstSeen - b.firstSeen ||
			a.id.localeCompare(b.id),
	);

	const hits: RankedHit[] = [];
	const spans = new Map<string, CandidateSpan>();
	const perFile = new Map<string, number>();
	for (const candidate of ordered) {
		const count = perFile.get(candidate.span.path) ?? 0;
		if (count >= PER_FILE_CANDIDATE_CAP) continue;
		perFile.set(candidate.span.path, count + 1);
		spans.set(candidate.id, candidate.span);
		hits.push({ id: candidate.id, rank: hits.length + 1, source: "grep" });
	}

	return { hits, spans };
}
