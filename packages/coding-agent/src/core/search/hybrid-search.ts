/**
 * Hybrid search orchestrator: resolve mode, run retrievers in parallel, fuse
 * by rank, expand within budget, trace everything
 * (docs/hybrid-retrieval-design.md).
 *
 * Single-retriever modes flow through the same pipeline — rrfFuse over one
 * list preserves its order — so lexical, semantic, and hybrid all produce the
 * same result shape and the same trace record.
 */

import type { EmbsearchService } from "../embsearch/embsearch-service.js";
import { adaptGrepHits, type ChunkLookup } from "./adapter.js";
import { assembleContext } from "./context-assembler.js";
import { runLexicalRetriever } from "./lexical-retriever.js";
import { resolveSearchMode } from "./mode.js";
import { DEFAULT_RRF_K, rrfFuse } from "./rrf.js";
import { writeSearchTrace } from "./trace.js";
import type { CandidateSpan, FusedCandidate, RankedHit, ResolvedSearchMode, SearchMode, SearchTrace } from "./types.js";

/** Raw grep line-hits fetched per query (pre-collapse). */
const LEXICAL_MATCH_LIMIT = 200;
/** Embedding hits fetched per query — deep enough for fusion to matter. */
const EMBED_TOP_K = 50;

export interface RunSearchOptions {
	cwd: string;
	query: string;
	mode?: SearchMode;
	/** Maximum fused results returned. */
	limit?: number;
	/** Approximate token budget for the result text. */
	tokenBudget?: number;
	service?: EmbsearchService;
	signal?: AbortSignal;
}

export interface RunSearchResult {
	text: string;
	resolvedMode: ResolvedSearchMode;
	degradedReason?: string;
	resultCount: number;
	/** Set while the embedding index is still building. */
	indexing?: { done: number; total: number };
}

export async function runSearch(options: RunSearchOptions): Promise<RunSearchResult> {
	const { cwd, query, service, signal } = options;
	const requestedMode = options.mode ?? "auto";
	const limit = Math.max(1, options.limit ?? 10);

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
			const lineHits = await runLexicalRetriever(cwd, query, LEXICAL_MATCH_LIMIT, signal);
			const adapted = adaptGrepHits(lineHits, lookupChunk);
			for (const [id, span] of adapted.spans) if (!spans.has(id)) spans.set(id, span);
			lists.push(adapted.hits);
			retrieverStats.grep = { latencyMs: Date.now() - startedMs, hitCount: adapted.hits.length };
		} catch (e) {
			errors.push(e instanceof Error ? e : new Error(String(e)));
			retrieverStats.grep = { latencyMs: Date.now() - startedMs, hitCount: 0 };
		}
	};

	const runEmbed = async (): Promise<void> => {
		const startedMs = Date.now();
		try {
			const chunkHits = await service!.searchChunks(query, EMBED_TOP_K);
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

	const runs: Promise<void>[] = [];
	if (mode === "lexical" || mode === "hybrid") runs.push(runLexical());
	if (mode === "semantic" || mode === "hybrid") runs.push(runEmbed());
	await Promise.all(runs);
	if (signal?.aborted) throw new Error("Operation aborted");
	// A partial failure in hybrid degrades to whichever retriever survived;
	// only a total loss is an error.
	if (lists.length === 0) throw errors[0] ?? new Error("search produced no retriever results");

	const fused = rrfFuse(lists, DEFAULT_RRF_K).slice(0, limit);
	const candidates: FusedCandidate[] = [];
	for (const hit of fused) {
		const span = spans.get(hit.id);
		if (span) candidates.push({ ...hit, ...span });
	}

	const assembled = assembleContext(candidates, { cwd, tokenBudget: options.tokenBudget });

	writeSearchTrace(cwd, {
		timestampMs: Date.now(),
		query,
		requestedMode,
		resolvedMode: mode,
		degradedReason: resolution.degradedReason,
		indexPhase: state?.phase === "ready" ? "ready" : state?.phase === "indexing" ? "indexing" : "unavailable",
		rrfK: mode === "hybrid" ? DEFAULT_RRF_K : undefined,
		retrievers: retrieverStats,
		fused,
	});

	return {
		text: assembled.text,
		resolvedMode: mode,
		degradedReason: resolution.degradedReason,
		resultCount: candidates.length,
		indexing: state?.phase === "indexing" ? { done: state.done, total: state.total } : undefined,
	};
}
