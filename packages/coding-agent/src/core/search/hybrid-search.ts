/**
 * Hybrid search orchestrator: resolve mode, run retrievers in parallel, fuse
 * by rank, expand within budget, trace everything
 * (docs/hybrid-retrieval-design.md).
 *
 * Single-retriever modes flow through the same pipeline — rrfFuse over one
 * list preserves its order — so lexical, semantic, and hybrid all produce the
 * same result shape and the same trace record.
 *
 * `retrieveCandidates` is the candidate-level core (also used by the eval
 * harness, which needs forced modes, a configurable `k`, and no trace
 * pollution); `runSearch` wraps it with span expansion and tracing for the
 * tool.
 */

import type { EmbsearchService } from "../embsearch/embsearch-service.js";
import { adaptGrepHits, type ChunkLookup } from "./adapter.js";
import { assembleContext } from "./context-assembler.js";
import { runLexicalRetriever } from "./lexical-retriever.js";
import { resolveSearchMode } from "./mode.js";
import { rerankCandidates } from "./rerank.js";
import { DEFAULT_RRF_K, rrfFuse } from "./rrf.js";
import { writeSearchTrace } from "./trace.js";
import type { CandidateSpan, FusedCandidate, RankedHit, ResolvedSearchMode, SearchMode, SearchTrace } from "./types.js";

/** Raw grep line-hits fetched per query (pre-collapse). */
const LEXICAL_MATCH_LIMIT = 200;
/** Adapted lexical candidates entering fusion. The eval gate showed the
 *  uncapped lexical tail diluting hybrid below plain semantic: lexical
 *  precision is front-loaded by the adapter's term-evidence ranking, while
 *  RRF weighs a rank-30 lexical candidate like a rank-30 embedding hit. */
const LEXICAL_FUSION_CAP = 20;
/**
 * Embedding hits fetched per query when fusing with the grep leg.
 *
 * Deliberately shallow. The grep list is capped at {@link LEXICAL_FUSION_CAP}
 * because its tail is unranked noise, so a deep dense pool here simply
 * outnumbers it: raising this to 200 alongside a 20-candidate grep list cost
 * `auto +rr` 0.472 -> 0.450 MRR.
 */
const EMBED_TOP_K = 50;
/**
 * Per-leg depth when both retrievers are ranked ones (dense + BM25).
 *
 * Four times {@link FUSED_WINDOW}, matching what the daemon's `query_hybrid`
 * does internally (`pool = 4·k`). Fusing at the same depth as the window loses
 * any candidate ranked well by one retriever but just outside the other's
 * top-50 — worth 12pp R@10 and 8pp R@50 for `bm25+dense`, which is exactly
 * what closed the gap to daemon-side fusion.
 */
const FUSION_POOL_TOP_K = 200;
/** BM25 depth, matching {@link FUSION_POOL_TOP_K} so neither ranked leg is
 *  structurally advantaged by pool size. The daemon returns only documents
 *  sharing a query term, so this is an upper bound, not a fill. */
const BM25_TOP_K = FUSION_POOL_TOP_K;
/** Fused candidates kept for reranking / final slicing. */
const FUSED_WINDOW = 50;

export interface RetrieveOptions {
	cwd: string;
	query: string;
	mode?: SearchMode;
	/** Optional glob filter applied to file paths. */
	glob?: string;
	/** Maximum fused candidates returned. */
	limit?: number;
	/** RRF constant override (eval harness sweeps this). Default: {@link DEFAULT_RRF_K}. */
	rrfK?: number;
	/** Rerank the fused top-50 before slicing to `limit`. Default: true. */
	rerank?: boolean;
	/**
	 * Ask the daemon to fuse its own BM25 index with the vectors and return one
	 * already-fused ranking, instead of taking a dense-only list. Needs a store
	 * built with `--hybrid`. The fused list arrives as a single "embed" leg,
	 * because a pre-fused ranking has no per-retriever structure left to record.
	 *
	 * Prefer {@link bm25Leg}: fusing here keeps the legs separable in the trace
	 * and lets the grep leg participate.
	 */
	daemonHybrid?: boolean;
	/**
	 * Fetch the daemon's BM25 index as its own ranked list and fuse it here,
	 * alongside dense and grep. Needs a store built with `--hybrid` and an
	 * embsearch new enough to serve `retriever: "lexical"`.
	 */
	bm25Leg?: boolean;
	service?: EmbsearchService;
	signal?: AbortSignal;
}

export interface RetrieveResult {
	candidates: FusedCandidate[];
	resolvedMode: ResolvedSearchMode;
	degradedReason?: string;
	indexPhase: SearchTrace["indexPhase"];
	retrievers: SearchTrace["retrievers"];
	/** Set while the embedding index is still building. */
	indexing?: { done: number; total: number };
	rrfK: number;
	rerank?: SearchTrace["rerank"];
}

export interface RunSearchOptions extends RetrieveOptions {
	/** Approximate token budget for the result text. */
	tokenBudget?: number;
}

export interface RunSearchResult {
	text: string;
	resolvedMode: ResolvedSearchMode;
	degradedReason?: string;
	resultCount: number;
	/** Set while the embedding index is still building. */
	indexing?: { done: number; total: number };
}

function normalizeSearchGlob(glob: string | undefined): string | undefined {
	if (!glob) return undefined;
	// Match fd/rg semantics: a slash-containing glob is anchored anywhere in
	// the tree, so prepend "**/" unless it already starts with a slash or "**/".
	if (glob.includes("/") && !glob.startsWith("/") && !glob.startsWith("**/")) {
		return `**/${glob}`;
	}
	return glob;
}

export async function retrieveCandidates(options: RetrieveOptions): Promise<RetrieveResult> {
	const { cwd, query, service, signal } = options;
	const glob = normalizeSearchGlob(options.glob);
	const requestedMode = options.mode ?? "auto";
	const limit = Math.max(1, options.limit ?? 10);
	const rrfK = options.rrfK ?? DEFAULT_RRF_K;

	const state = service?.getState();
	const embedAvailable = service?.isAvailable() ?? false;
	const embedUnavailableReason =
		state === undefined
			? "semantic index is not enabled"
			: state.phase === "unavailable" || state.phase === "skipped"
				? state.reason
				: state.phase === "idle"
					? "semantic index has not started"
					: undefined;

	const resolution = resolveSearchMode(query, requestedMode, embedAvailable, embedUnavailableReason);
	const mode = resolution.mode;

	// Map lexical hits onto indexed chunk ids whenever the sidecar is usable,
	// even in lexical-only mode, so identities line up across modes.
	const lookupChunk: ChunkLookup | undefined = embedAvailable
		? (rel, line) => service!.findEnclosingChunk(rel, line)
		: undefined;

	const spans = new Map<string, CandidateSpan>();
	const lists: RankedHit[][] = [];
	const retrieverStats: SearchTrace["retrievers"] = {};
	const errors: Error[] = [];

	const runLexical = async (): Promise<void> => {
		const startedMs = Date.now();
		try {
			const lineHits = await runLexicalRetriever({ cwd, query, limit: LEXICAL_MATCH_LIMIT, glob, signal });
			const adapted = adaptGrepHits(lineHits, lookupChunk);
			// In single-retriever lexical mode the full list is the result; in
			// hybrid, only the front-loaded head is trustworthy enough to vote.
			const hits = mode === "hybrid" ? adapted.hits.slice(0, LEXICAL_FUSION_CAP) : adapted.hits;
			for (const [id, span] of adapted.spans) if (!spans.has(id)) spans.set(id, span);
			lists.push(hits);
			retrieverStats.grep = { latencyMs: Date.now() - startedMs, hitCount: hits.length };
		} catch (e) {
			errors.push(e instanceof Error ? e : new Error(String(e)));
			retrieverStats.grep = { latencyMs: Date.now() - startedMs, hitCount: 0 };
		}
	};

	const runEmbed = async (): Promise<void> => {
		const startedMs = Date.now();
		try {
			// The flat index pads top-k with whatever exists; with the cosine
			// metric the store uses, score <= 0 means "no relation at all", so
			// those padding hits would cast RRF votes on pure noise.
			// A BM25 leg is itself ranked, so the pair can afford — and needs — the
			// deeper pool; the grep leg cannot (see EMBED_TOP_K).
			const topK = options.bm25Leg ? FUSION_POOL_TOP_K : EMBED_TOP_K;
			const chunkHits = (
				await service!.searchChunks(query, topK, glob, options.daemonHybrid ? "hybrid" : "dense")
			).filter((hit) => hit.score > 0);
			const hits: RankedHit[] = chunkHits.map((hit, i) => ({
				id: hit.id,
				rank: i + 1,
				score: hit.score,
				source: "embed",
			}));
			for (const hit of chunkHits) {
				spans.set(hit.id, { path: hit.path, startLine: hit.startLine, endLine: hit.endLine });
			}
			lists.push(hits);
			retrieverStats.embed = { latencyMs: Date.now() - startedMs, hitCount: hits.length };
		} catch (e) {
			errors.push(e instanceof Error ? e : new Error(String(e)));
			retrieverStats.embed = { latencyMs: Date.now() - startedMs, hitCount: 0 };
		}
	};

	const runBm25 = async (): Promise<void> => {
		const startedMs = Date.now();
		try {
			// Raw BM25 sums; only the ordering enters fusion, the score is a
			// diagnostic. Unlike the dense leg there is no zero-score padding to
			// filter — the daemon omits documents sharing no query term.
			const chunkHits = await service!.searchChunks(query, BM25_TOP_K, glob, "lexical");
			const hits: RankedHit[] = chunkHits.map((hit, i) => ({
				id: hit.id,
				rank: i + 1,
				score: hit.score,
				source: "bm25",
			}));
			for (const hit of chunkHits) {
				if (!spans.has(hit.id)) {
					spans.set(hit.id, { path: hit.path, startLine: hit.startLine, endLine: hit.endLine });
				}
			}
			lists.push(hits);
			retrieverStats.bm25 = { latencyMs: Date.now() - startedMs, hitCount: hits.length };
		} catch (e) {
			errors.push(e instanceof Error ? e : new Error(String(e)));
			retrieverStats.bm25 = { latencyMs: Date.now() - startedMs, hitCount: 0 };
		}
	};

	const runs: Promise<void>[] = [];
	if (mode === "lexical" || mode === "hybrid") runs.push(runLexical());
	if (mode === "semantic" || mode === "hybrid") runs.push(runEmbed());
	if (options.bm25Leg && embedAvailable && mode !== "lexical") runs.push(runBm25());
	await Promise.all(runs);
	if (signal?.aborted) throw new Error("Operation aborted");
	// A partial failure in hybrid degrades to whichever retriever survived;
	// only a total loss is an error.
	if (lists.length === 0) throw errors[0] ?? new Error("search produced no retriever results");

	const fused = rrfFuse(lists, rrfK).slice(0, FUSED_WINDOW);
	let candidates: FusedCandidate[] = [];
	for (const hit of fused) {
		const span = spans.get(hit.id);
		if (span) candidates.push({ ...hit, ...span });
	}

	let rerankInfo: SearchTrace["rerank"];
	if (options.rerank !== false) {
		const reranked = rerankCandidates(query, candidates, cwd);
		rerankInfo = { applied: true, candidateCount: candidates.length, latencyMs: reranked.latencyMs };
		candidates = reranked.candidates;
	}
	candidates = candidates.slice(0, limit);

	return {
		candidates,
		resolvedMode: mode,
		degradedReason: resolution.degradedReason,
		indexPhase: state?.phase === "ready" ? "ready" : state?.phase === "indexing" ? "indexing" : "unavailable",
		retrievers: retrieverStats,
		indexing: state?.phase === "indexing" ? { done: state.done, total: state.total } : undefined,
		rrfK,
		rerank: rerankInfo,
	};
}

export async function runSearch(options: RunSearchOptions): Promise<RunSearchResult> {
	const retrieved = await retrieveCandidates(options);

	const assembled = assembleContext(retrieved.candidates, { cwd: options.cwd, tokenBudget: options.tokenBudget });

	writeSearchTrace(options.cwd, {
		timestampMs: Date.now(),
		query: options.query,
		requestedMode: options.mode ?? "auto",
		resolvedMode: retrieved.resolvedMode,
		degradedReason: retrieved.degradedReason,
		indexPhase: retrieved.indexPhase,
		rrfK: retrieved.resolvedMode === "hybrid" ? retrieved.rrfK : undefined,
		retrievers: retrieved.retrievers,
		fused: retrieved.candidates.map(({ id, rrfScore, ranks, rawScores }) => ({ id, rrfScore, ranks, rawScores })),
		rerank: retrieved.rerank,
	});

	return {
		text: assembled.text,
		resolvedMode: retrieved.resolvedMode,
		degradedReason: retrieved.degradedReason,
		resultCount: retrieved.candidates.length,
		indexing: retrieved.indexing,
	};
}
