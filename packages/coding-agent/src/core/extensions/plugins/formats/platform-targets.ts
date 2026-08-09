/**
 * Session-wide artifact platform targeting (`--platform`).
 *
 * hoocode reads resources from every vendor convention, but when it *writes*
 * artifacts it needs a target layout. The two kinds of write have **different
 * rules**, which is why this module exposes two resolvers rather than one:
 *
 *   Plugins            distribution units. A plugin only means anything as a
 *                      member of a platform ecosystem (there is no `.agents`
 *                      marketplace), so production is platform-specific and
 *                      `agents` is never a valid target.
 *                      → {@link resolvePluginPlatforms}
 *   Workspace scaffolds  `/new-skill`, `/new-agent`, `/new-command`. These are
 *                      consumed in place, never shipped, and `.agents/` is a
 *                      real cross-vendor convention for them.
 *                      → {@link getWorkspacePlatforms}
 *
 *   token vocabulary   `agents` (alias `native`), `claude`,
 *                      `github` (aliases `copilot`, `gh`)
 *   session state      set once at startup from the `--platform` flag or the
 *                      `platform` setting (main.ts)
 *
 * See docs/plugin-system-architecture.md §1 for the reasoning.
 *
 * Kept beside the format registry (and importing only `types.ts`) so the
 * platform vocabulary and the adapters stay a one-directory concern: adding a
 * vendor means a new adapter file plus a token here, nothing else.
 */

import type { MarketplacePlatform } from "./types.js";

/** The platforms a *plugin* can be produced for. `agents` is excluded by construction. */
export type PluginPlatform = Exclude<MarketplacePlatform, "agents">;

/** True for the platform tokens that own a plugin ecosystem. */
export function isPluginPlatform(platform: MarketplacePlatform): platform is PluginPlatform {
	return platform !== "agents";
}

/**
 * Default plugin production target when neither the call nor the session picked
 * one. Claude is the default because it is the only platform whose local loop
 * closes: an authored plugin lands in `~/.claude/skills/<id>/` and is live on the
 * next session, whereas Copilot has no in-place drop-in at all.
 */
export const DEFAULT_PLUGIN_PLATFORMS: readonly PluginPlatform[] = ["claude"];

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

export interface PlatformParse {
	/** Canonical platforms, deduped, in the order first mentioned. */
	platforms: MarketplacePlatform[];
	/** Tokens that matched no known platform (surfaced as diagnostics, never fatal). */
	invalid: string[];
}

/** Parse raw `--platform` / `platform` tokens into canonical platforms. */
export function parsePlatforms(tokens: readonly string[]): PlatformParse {
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

/** Session-wide targets. Undefined = not configured (fall back to per-consumer defaults). */
let sessionPlatforms: MarketplacePlatform[] | undefined;

/** Set (or clear, with undefined/empty) the session's artifact platform targets. */
export function setPlatforms(platforms: readonly MarketplacePlatform[] | undefined): void {
	sessionPlatforms = platforms && platforms.length > 0 ? [...platforms] : undefined;
}

/**
 * The session's configured targets for **workspace scaffolds**, or undefined when
 * `--platform` was not given — callers treat undefined as "use the hoocode-native
 * location" rather than substituting a default. `agents` is a legitimate value
 * here: a scaffolded skill or command is consumed in place, not distributed.
 */
export function getWorkspacePlatforms(): MarketplacePlatform[] | undefined {
	return sessionPlatforms ? [...sessionPlatforms] : undefined;
}

/**
 * Resolve the platforms a **plugin** should be produced for: an explicit per-call
 * selection wins, then the session's `--platform` targets, then
 * {@link DEFAULT_PLUGIN_PLATFORMS}.
 *
 * `agents` can never be the answer. An explicit `agents` throws — a caller asking
 * for a plugin in a format no marketplace accepts has made a mistake and should
 * hear about it. A session-level `agents` is *filtered* instead, because
 * `--platform` also drives workspace scaffolds, where `agents` is a perfectly
 * good choice; a user who set it for that reason should not have plugin authoring
 * blow up in their face. If filtering leaves nothing, the default applies.
 */
export function resolvePluginPlatforms(explicit?: readonly MarketplacePlatform[]): PluginPlatform[] {
	if (explicit && explicit.length > 0) {
		const rejected = explicit.filter((p) => !isPluginPlatform(p));
		if (rejected.length > 0) {
			throw new Error(
				`Cannot author a plugin for platform "${rejected.join(", ")}": plugins are distribution units and ` +
					"the native `agents` layout belongs to no marketplace, so it can never be published or installed. " +
					"Target `claude` or `github`, or author a workspace skill instead.",
			);
		}
		return explicit.filter(isPluginPlatform);
	}
	const fromSession = (sessionPlatforms ?? []).filter(isPluginPlatform);
	return fromSession.length > 0 ? fromSession : [...DEFAULT_PLUGIN_PLATFORMS];
}
