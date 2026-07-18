/**
 * Shared types for hybrid code retrieval (see docs/hybrid-retrieval-design.md).
 *
 * Identity note: candidate ids are per-index-build chunk ids (`relpath#index`)
 * from the embedding sidecar, or synthetic `relpath#L<line>` fallback ids for
 * grep hits in files the index does not cover. They are stable within one
 * query (fusion and span expansion consult the same sidecar snapshot) but not
 * across edits or rebuilds — never persist them as durable references.
 */

export type RetrieverSource = "grep" | "embed";

export type SearchMode = "auto" | "lexical" | "semantic" | "hybrid";
export type ResolvedSearchMode = Exclude<SearchMode, "auto">;

export interface RankedHit {
	id: string;
	/** 1-indexed, gap-free rank within its retriever's list. */
	rank: number;
	/** Retriever-local score (BM25-ish, cosine, …). Diagnostics only — never fused. */
	score?: number;
	source: RetrieverSource;
}

export interface FusedHit {
	id: string;
	rrfScore: number;
	/** Best rank per contributing retriever. */
	ranks: Partial<Record<RetrieverSource, number>>;
	/** Raw score at the best rank per retriever. Diagnostics only. */
	rawScores: Partial<Record<RetrieverSource, number>>;
}

/** Line-span identity a candidate id resolves to, for post-fusion expansion. */
export interface CandidateSpan {
	path: string;
	/** 1-based inclusive. */
	startLine: number;
	/** 1-based inclusive. May exceed the file's length; readers clamp. */
	endLine: number;
}

export interface FusedCandidate extends FusedHit, CandidateSpan {}

/** Per-call diagnostic record, written to the store-dir trace jsonl — never
 *  into model context. */
export interface SearchTrace {
	timestampMs: number;
	query: string;
	requestedMode: SearchMode;
	resolvedMode: ResolvedSearchMode;
	/** Set when the resolved mode is a degradation of the requested one. */
	degradedReason?: string;
	indexPhase: "ready" | "indexing" | "unavailable";
	rrfK?: number;
	retrievers: Partial<Record<RetrieverSource, { latencyMs: number; hitCount: number }>>;
	fused: FusedHit[];
}
