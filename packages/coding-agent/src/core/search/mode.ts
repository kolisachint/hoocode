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

/** Regex metacharacters, quoted strings, or explicit path-with-extension —
 *  queries where exact matching is clearly what the caller wants. */
export function hasStrongLexicalSignals(query: string): boolean {
	if (/["'`]/.test(query)) return true;
	if (/[\\^$|()[\]{}*+?]/.test(query)) return true;
	// A path segment ending in a file extension, e.g. src/core/sdk.ts
	if (/[\w-]+\/[\w./-]*\.\w{1,8}(\s|$)/.test(query)) return true;
	return false;
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
