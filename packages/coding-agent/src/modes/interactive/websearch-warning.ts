/**
 * The "websearch has no API key" startup notice.
 *
 * `websearch` works with no configuration at all, but the keyless default is
 * scraped DuckDuckGo HTML: it is rate-limited hard and can fail outright, and
 * the failure surfaces as a bad search rather than as a setup problem. So when
 * the tool is active and no keyed backend is configured, the session says so
 * once — the same shape as the Anthropic extra-usage notice, including the
 * `/settings` switch that turns it off.
 */

import { CONFIG_DIR_NAME } from "../../config.js";
import type { WarningSettings } from "../../core/settings-types.js";
import { resolveWebSearchCredentials, type WebtoolsSearchSettings } from "../../core/tools/webtools-shared.js";

export const WEBSEARCH_API_KEY_TITLE = "Web search has no API key";

export interface WebSearchApiKeyNoticeInput {
	/** Tools the session actually exposes; the notice is pointless without `websearch`. */
	activeToolNames: readonly string[];
	/** Warning switches from settings (`warnings.websearchApiKey`). */
	warnings: WarningSettings;
	/** The `webtools.search` block, as the binary would read it. */
	search?: WebtoolsSearchSettings;
}

export interface WebSearchApiKeyNotice {
	title: string;
	body: string[];
}

/**
 * The notice to show, or undefined when there is nothing to say: the tool is
 * off, a keyed backend is configured, the user pinned the keyless backend on
 * purpose, or the warning is switched off.
 *
 * Pure, so the decision is testable without a TUI; the caller owns the
 * once-per-session latch.
 */
export function websearchApiKeyNotice(input: WebSearchApiKeyNoticeInput): WebSearchApiKeyNotice | undefined {
	if (input.warnings.websearchApiKey === false) return undefined;
	if (!input.activeToolNames.includes("websearch")) return undefined;

	const credentials = resolveWebSearchCredentials(input.search);
	if (credentials.configured || credentials.explicitKeyless) return undefined;

	return {
		title: WEBSEARCH_API_KEY_TITLE,
		body: [
			"Searching keyless DuckDuckGo: scraped HTML, rate-limited, and it can fail outright.",
			`Set BRAVE_API_KEY or TAVILY_API_KEY, or webtools.search in ~/${CONFIG_DIR_NAME}/settings.json.`,
			"Turn off in /settings → Web search API key.",
		],
	};
}
