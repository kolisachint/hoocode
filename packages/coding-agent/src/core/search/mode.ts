/**
 * Availability-first search mode resolution
 * (docs/hybrid-retrieval-design.md, Decision 4).
 *
 * No clever query router: with a hot local daemon, running both retrievers
 * costs one extra embedding query, while misrouting costs recall. `auto`
 * therefore means hybrid whenever the index is available, dropping to lexical
 * only on strong lexical signals. Requested semantic/hybrid degrade to
 * lexical (with a recorded reason) when the index is unavailable — never an
 * error, unlike the old semantic_search tool.
 */

import type { ResolvedSearchMode, SearchMode } from "./types.js";

export interface ModeResolution {
	mode: ResolvedSearchMode;
	/** Set when the resolved mode is a forced degradation of the request. */
	degradedReason?: string;
}

/**
 * Regex metacharacters *outside* any quoted segment — the only remaining
 * signal that the caller wants exact matching rather than ranked discovery.
 *
 * Being quoted is deliberately NOT such a signal any more. It used to route
 * every quoted query to lexical-only, which is where all ten error-fragment
 * queries in the gold set land, and lexical is the worst leg for them: R@1
 * 0.000 and MRR 0.483, against 0.600 and 0.800 for the same queries in
 * hybrid. Recall survived the routing but rank did not, and rank is what an
 * agent reads.
 *
 * Metacharacters inside a quoted segment carry no information either, because
 * `buildLexicalQueryPlan` searches a quoted segment verbatim — it escapes the
 * content, so `"initTheme() first."` is matched literally and its parentheses
 * say nothing about the caller's intent. Strip quoted spans before looking.
 *
 * Path-like queries also do NOT count: the eval gate showed them scoring 0%
 * lexically (content grep cannot find a file by its own name) and far better
 * in hybrid, where the embedding side and the reranker's path-affinity signal
 * carry them.
 */
export function hasStrongLexicalSignals(query: string): boolean {
	const unquoted = query.replace(/["'`][^"'`]*["'`]/g, " ");
	return /[\\^$|()[\]{}*+?]/.test(unquoted);
}

export function resolveSearchMode(
	query: string,
	requested: SearchMode,
	embedAvailable: boolean,
	embedUnavailableReason?: string,
): ModeResolution {
	if (requested === "lexical") return { mode: "lexical" };

	if (!embedAvailable) {
		const reason = embedUnavailableReason ?? "semantic index unavailable";
		return requested === "auto"
			? { mode: "lexical" }
			: { mode: "lexical", degradedReason: `${requested} requested but ${reason}` };
	}

	if (requested === "semantic" || requested === "hybrid") return { mode: requested };

	return hasStrongLexicalSignals(query) ? { mode: "lexical" } : { mode: "hybrid" };
}
