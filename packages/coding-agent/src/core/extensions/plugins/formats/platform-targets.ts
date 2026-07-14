/**
 * Session-wide artifact platform targeting (`--support-platform`).
 *
 * hoocode reads resources from every vendor convention, but when it *writes*
 * artifacts — authored plugins via ProposePlugin, workspace scaffolds via
 * /new-skill //new-agent //new-command — it needs a target layout. This module
 * owns that choice for the whole process:
 *
 *   token vocabulary   `agents` (alias `native`), `claude`,
 *                      `github` (aliases `copilot`, `gh`)
 *   session state      set once at startup from the `--support-platform` flag
 *                      or the `supportPlatform` setting (main.ts)
 *   resolution         explicit per-call platforms → session targets →
 *                      {@link DEFAULT_AUTHORING_PLATFORMS}
 *
 * Kept beside the format registry (and importing only `types.ts`) so the
 * platform vocabulary and the adapters stay a one-directory concern: adding a
 * vendor means a new adapter file plus a token here, nothing else.
 */

import type { MarketplacePlatform } from "./types.js";

/** Default authoring targets when neither the call nor the session picked any. */
export const DEFAULT_AUTHORING_PLATFORMS: readonly MarketplacePlatform[] = ["claude", "github"];

/** Canonicalize one platform token, folding the user-facing aliases. */
export function normalizePlatformToken(token: string): MarketplacePlatform | undefined {
	switch (token.trim().toLowerCase()) {
		case "agents":
		case "native":
			return "agents";
		case "claude":
			return "claude";
		case "github":
		case "copilot":
		case "gh":
			return "github";
		default:
			return undefined;
	}
}

export interface SupportPlatformParse {
	/** Canonical platforms, deduped, in the order first mentioned. */
	platforms: MarketplacePlatform[];
	/** Tokens that matched no known platform (surfaced as diagnostics, never fatal). */
	invalid: string[];
}

/** Parse raw `--support-platform` / `supportPlatform` tokens into canonical platforms. */
export function parseSupportPlatforms(tokens: readonly string[]): SupportPlatformParse {
	const platforms: MarketplacePlatform[] = [];
	const invalid: string[] = [];
	for (const token of tokens) {
		const trimmed = token.trim();
		if (!trimmed) continue;
		const canonical = normalizePlatformToken(trimmed);
		if (!canonical) invalid.push(trimmed);
		else if (!platforms.includes(canonical)) platforms.push(canonical);
	}
	return { platforms, invalid };
}

/** Session-wide targets. Undefined = not configured (fall back to defaults). */
let sessionSupportPlatforms: MarketplacePlatform[] | undefined;

/** Set (or clear, with undefined/empty) the session's artifact platform targets. */
export function setSupportPlatforms(platforms: readonly MarketplacePlatform[] | undefined): void {
	sessionSupportPlatforms = platforms && platforms.length > 0 ? [...platforms] : undefined;
}

/** The session's configured targets, or undefined when `--support-platform` was not given. */
export function getSupportPlatforms(): MarketplacePlatform[] | undefined {
	return sessionSupportPlatforms ? [...sessionSupportPlatforms] : undefined;
}

/**
 * Resolve the platforms an authoring write should target: an explicit per-call
 * selection wins, then the session's `--support-platform` targets, then
 * {@link DEFAULT_AUTHORING_PLATFORMS}.
 */
export function resolveAuthoringPlatforms(explicit?: readonly MarketplacePlatform[]): MarketplacePlatform[] {
	if (explicit && explicit.length > 0) return [...explicit];
	return [...(sessionSupportPlatforms ?? DEFAULT_AUTHORING_PLATFORMS)];
}
