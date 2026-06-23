import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import { theme as appTheme } from "../../modes/interactive/theme/theme.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import { getTextOutput, invalidArgText, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";
import {
	hostnameOf,
	isHostBlocked,
	loadWebtoolsIgnore,
	runWebtools,
	WEBTOOLS_DEFAULT_TIMEOUT_SECS,
	type WebSearchOutput,
	type WebSearchResultItem,
	WebToolsCache,
} from "./webtools-shared.js";

const DEFAULT_MAX_RESULTS = 5;
const MAX_RESULTS_CAP = 10;

const websearchSchema = Type.Object({
	query: Type.String({ description: "The search query" }),
	maxResults: Type.Optional(
		Type.Number({
			description: `Maximum number of results to return (default: ${DEFAULT_MAX_RESULTS}, max: ${MAX_RESULTS_CAP})`,
		}),
	),
	safeSearch: Type.Optional(
		Type.Union([Type.Literal("on"), Type.Literal("off")], {
			description: "Safe search filter. Omit to use the search engine's default.",
		}),
	),
});

export type WebSearchToolInput = Static<typeof websearchSchema>;

export interface WebSearchToolDetails {
	resultCount?: number;
	hiddenCount?: number;
	tokenEstimate?: number;
}

export interface WebSearchToolOptions {
	/** Override the result cache (mainly for tests). */
	cache?: WebToolsCache<WebSearchOutput>;
}

/**
 * Render the kept results in reference style: title + snippet with a trailing
 * [N] marker, then a `References:` block. Indices are renumbered after filtering
 * so the visible list stays contiguous.
 */
function renderSearchText(query: string, kept: WebSearchResultItem[], hiddenCount: number): string {
	if (kept.length === 0) {
		const suffix = hiddenCount > 0 ? ` (${hiddenCount} blocked by .webtoolsignore policy)` : "";
		return `No results for "${query}"${suffix}`;
	}
	const blocks: string[] = [];
	const refs: string[] = [];
	kept.forEach((item, i) => {
		const n = i + 1;
		blocks.push(`${item.title} [${n}]\n${item.snippet}`);
		refs.push(`[${n}] ${item.url}`);
	});
	let text = `${blocks.join("\n\n")}\n\nReferences:\n${refs.join("\n")}`;
	if (hiddenCount > 0) {
		text += `\n\n[${hiddenCount} result${hiddenCount === 1 ? "" : "s"} hidden by .webtoolsignore policy]`;
	}
	return text;
}

function formatWebsearchCall(args: { query?: string; maxResults?: number } | undefined): string {
	const query = str(args?.query);
	const queryDisplay =
		query === null ? invalidArgText(appTheme) : query ? `"${query}"` : appTheme.fg("toolOutput", "...");
	const limit = args?.maxResults;
	let text = appTheme.fg("toolTitle", appTheme.bold("websearch ")) + appTheme.fg("accent", queryDisplay);
	if (limit !== undefined) text += appTheme.fg("muted", ` (${limit})`);
	return text;
}

function formatWebsearchResult(
	result: { content: Array<{ type: string; text?: string }>; details?: WebSearchToolDetails },
	options: ToolRenderResultOptions,
	showImages: boolean,
): string {
	const output = getTextOutput(result as any, showImages).trim();
	let text = "";
	if (output) {
		const lines = output.split("\n");
		const maxLines = options.expanded ? lines.length : 15;
		const displayLines = lines.slice(0, maxLines);
		const remaining = lines.length - maxLines;
		text += `\n${displayLines.map((line) => appTheme.fg("toolOutput", line)).join("\n")}`;
		if (remaining > 0) {
			text += `${appTheme.fg("muted", `\n... (${remaining} more lines,`)} ${keyHint("app.tools.expand", "to expand")})`;
		}
	}
	return text;
}

export function createWebSearchToolDefinition(
	cwd: string,
	options?: WebSearchToolOptions,
): ToolDefinition<typeof websearchSchema, WebSearchToolDetails | undefined> {
	const cache = options?.cache ?? new WebToolsCache<WebSearchOutput>();
	return {
		name: "websearch",
		label: "websearch",
		description:
			"Search the web (DuckDuckGo, no API key) and return ranked results as titles + snippets with reference-style [N] links. Use to discover URLs, then webfetch to read a result in full. Off by default; enabled with --enable-webtools.",
		promptSnippet: "Search the web and return ranked results with links",
		promptGuidelines: [
			"Use websearch to discover URLs when you do not already have one, then webfetch the most relevant result to read it in full.",
		],
		parameters: websearchSchema,
		async execute(_toolCallId, { query, maxResults, safeSearch }: WebSearchToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");

			const effectiveMax = Math.min(MAX_RESULTS_CAP, Math.max(1, maxResults ?? DEFAULT_MAX_RESULTS));
			const cacheKey = `${effectiveMax}:${safeSearch ?? "default"}:${query}`;

			const args = ["--query", query, "--max-results", String(effectiveMax)];
			if (safeSearch) args.push("--safe-search", safeSearch);
			const output = await cache.getOrCompute(cacheKey, signal, (sig) =>
				runWebtools<WebSearchOutput>("search", args, cwd, sig, WEBTOOLS_DEFAULT_TIMEOUT_SECS),
			);

			// Filter result links through .webtoolsignore policy.
			const matcher = loadWebtoolsIgnore(cwd);
			const allResults = output.results ?? [];
			const kept = matcher
				? allResults.filter((item) => {
						const host = hostnameOf(item.url);
						return host ? !isHostBlocked(matcher, host) : true;
					})
				: allResults;
			const hiddenCount = allResults.length - kept.length;

			return {
				content: [{ type: "text" as const, text: renderSearchText(query, kept, hiddenCount) }],
				details: {
					resultCount: kept.length,
					hiddenCount,
					tokenEstimate: output.token_estimate,
				},
			};
		},
		renderCall(args, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatWebsearchCall(args));
			return text;
		},
		renderResult(result, options, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatWebsearchResult(result as any, options, context.showImages));
			return text;
		},
	};
}

export function createWebSearchTool(cwd: string, options?: WebSearchToolOptions): AgentTool<typeof websearchSchema> {
	return wrapToolDefinition(createWebSearchToolDefinition(cwd, options));
}
