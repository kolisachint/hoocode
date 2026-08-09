/**
 * Capability retrieval: BM25 and dense, fused by RRF.
 *
 * Hybrid rather than dense-only because capability descriptions are short.
 * Pure dense retrieval over eight-word strings is weak, and — more decisively —
 * an exact name lookup has to stay exact: `create_pull_request` must find
 * `mcp_github_create_pull_request` every time, which is a lexical guarantee, not
 * a nearest-neighbour one. Fusion gets both: the lexical leg answers "the tool
 * called roughly this", the dense leg "something that does this".
 *
 * Reuses the repo search's {@link rrfFuse} rather than reimplementing fusion.
 * The tie-breaking and duplicate handling there are already worked out, and a
 * second, subtly different fusion is how two retrieval paths drift apart.
 */

import { rrfFuse } from "../search/rrf.js";
import type { RankedHit } from "../search/types.js";
import { denseSearch, denseState } from "./dense.js";
import { LexicalIndex } from "./lexical.js";
import { type CapabilityDoc, type CapabilityKind, capabilitySetHash, getCapabilities } from "./registry.js";

export interface CapabilityHit {
	doc: CapabilityDoc;
	/** Fused score. Comparable within one result set only. */
	score: number;
	/** Which legs found it — useful when explaining a surprising ranking. */
	matchedBy: Array<"lexical" | "dense">;
}

export interface CapabilitySearchOptions {
	/** Restrict to these kinds. Omit for everything. */
	kinds?: readonly CapabilityKind[];
	/** Results to return. */
	limit?: number;
	/** Restrict to capabilities whose expensive part is currently withheld. */
	deferredOnly?: boolean;
}

export interface CapabilitySearchResult {
	hits: CapabilityHit[];
	/** How the answer was produced, so a caller can say "lexical only" honestly. */
	legs: Array<"lexical" | "dense">;
}

/** Cached index, rebuilt when the capability set changes. */
let cached: { hash: string; index: LexicalIndex; docs: CapabilityDoc[] } | undefined;

function lexicalIndexFor(docs: CapabilityDoc[]): LexicalIndex {
	const hash = capabilitySetHash(docs);
	if (cached?.hash !== hash) cached = { hash, index: new LexicalIndex(docs), docs };
	return cached.index;
}

/** Drop the cached lexical index. Tests, and anything that rewrites the registry wholesale. */
export function resetCapabilitySearch(): void {
	cached = undefined;
}

/**
 * Find capabilities matching `query`.
 *
 * Over-fetches from each leg before fusing: RRF works on rank, so a document
 * ranked 8th by one retriever and absent from the other still deserves to be
 * considered — truncating each list to the final `limit` first would throw that
 * away before fusion could use it.
 */
export async function searchCapabilities(
	query: string,
	options: CapabilitySearchOptions = {},
): Promise<CapabilitySearchResult> {
	const limit = options.limit ?? 10;
	let docs = getCapabilities(options.kinds);
	if (options.deferredOnly) docs = docs.filter((d) => d.deferred);
	if (docs.length === 0 || !query.trim()) return { hits: [], legs: [] };

	const byId = new Map(docs.map((d) => [d.id, d]));
	const fetchK = Math.max(limit * 3, 20);

	const lexical = lexicalIndexFor(docs).search(query, fetchK);
	const dense = denseState().status === "ready" ? await denseSearch(query, fetchK) : [];

	const legs: Array<"lexical" | "dense"> = [];
	const lists: RankedHit[][] = [];
	if (lexical.length > 0) {
		legs.push("lexical");
		lists.push(lexical.map((h, i) => ({ id: h.id, rank: i + 1, score: h.score, source: "bm25" as const })));
	}
	// The dense store spans every registered capability, so its hits can fall
	// outside a kind- or deferred-filtered set; drop those before ranking rather
	// than leaving gaps in the rank sequence RRF expects.
	const denseInScope = dense.filter((h) => byId.has(h.id));
	if (denseInScope.length > 0) {
		legs.push("dense");
		lists.push(denseInScope.map((h, i) => ({ id: h.id, rank: i + 1, score: h.score, source: "embed" as const })));
	}
	if (lists.length === 0) return { hits: [], legs: [] };

	const hits: CapabilityHit[] = [];
	for (const fused of rrfFuse(lists)) {
		const doc = byId.get(fused.id);
		if (!doc) continue;
		const matchedBy: Array<"lexical" | "dense"> = [];
		if (fused.ranks.bm25 !== undefined) matchedBy.push("lexical");
		if (fused.ranks.embed !== undefined) matchedBy.push("dense");
		hits.push({ doc, score: fused.rrfScore, matchedBy });
		if (hits.length >= limit) break;
	}
	return { hits, legs };
}
