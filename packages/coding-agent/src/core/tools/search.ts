/**
 * The unified `SearchCodebase` tool: ranked code discovery in lexical, semantic, or
 * hybrid (rank-fused) mode. Replaces the old `semantic_search` tool — see
 * docs/hybrid-retrieval-design.md, Decision 1. It is the only dedicated
 * discovery tool: this tool answers "find where X lives", and exact
 * line-level mechanics are a shell job (rg/find/ls via bash).
 */

import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import type { EmbsearchService } from "../embsearch/embsearch-service.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import { runSearch } from "../search/hybrid-search.js";
import type { ResolvedSearchMode } from "../search/types.js";
import { getTextOutput, invalidArgText, str } from "./render-utils.js";

// Default of 5 balances recall against context cost. Measured on the 62-query
// eval: limit=5 costs ~1000 tokens and puts a gold span in the results 68% of
// the time; limit=10 costs ~1600 tokens for 74%. The extra 6 points are real
// but not worth 60% more context on every call, and the model can ask for more.
const DEFAULT_RESULTS = 5;
const MAX_RESULTS = 30;

const searchSchema = Type.Object({
	query: Type.String({
		description:
			"What to find: an identifier, error text, or a natural-language description of the code, e.g. 'where sessions are persisted to disk'",
	}),
	mode: Type.Optional(
		Type.Union([Type.Literal("auto"), Type.Literal("lexical"), Type.Literal("semantic"), Type.Literal("hybrid")], {
			description:
				"Retrieval mode (default: auto, which is almost always right). auto = hybrid when the index is available, else lexical; hybrid = keyword and meaning over the index plus exact text for anything indexed later; semantic = index only, skipping exact text; lexical = exact text only, the one mode that works with no index.",
		}),
	),
	glob: Type.Optional(
		Type.String({
			description:
				"Optional glob filter applied to file paths. Only file paths matching the glob are searched. Supports both slashless patterns (match base name anywhere) and slash patterns (match full path).",
		}),
	),
	limit: Type.Optional(
		Type.Number({ description: `Maximum number of results (default: ${DEFAULT_RESULTS}, max: ${MAX_RESULTS})` }),
	),
});

type SearchToolInput = Static<typeof searchSchema>;

export interface SearchToolDetails {
	resultCount?: number;
	resolvedMode?: ResolvedSearchMode;
	indexing?: { done: number; total: number };
}

export interface SearchToolOptions {
	/**
	 * Provider for the per-session embsearch service. Resolved lazily at call
	 * time because the service is created (and its index built) after tool
	 * registration. When absent/unavailable the tool degrades to lexical mode.
	 */
	getService?: () => EmbsearchService | undefined;
}

function formatSearchCall(
	args: { query?: string; mode?: string; limit?: number } | undefined,
	theme: typeof import("../../modes/interactive/theme/theme.js").theme,
): string {
	const query = str(args?.query);
	const queryDisplay = query === null ? invalidArgText(theme) : `"${query}"`;
	let text = theme.fg("toolTitle", theme.bold("SearchCodebase ")) + theme.fg("accent", queryDisplay);
	const extras: string[] = [];
	if (args?.mode && args.mode !== "auto") extras.push(args.mode);
	if (args?.limit !== undefined) extras.push(String(args.limit));
	if (extras.length > 0) text += theme.fg("muted", ` (${extras.join(", ")})`);
	return text;
}

function formatSearchResult(
	result: { content: Array<{ type: string; text?: string }>; details?: SearchToolDetails },
	options: ToolRenderResultOptions,
	theme: typeof import("../../modes/interactive/theme/theme.js").theme,
	showImages: boolean,
): string {
	const output = getTextOutput(result as any, showImages).trim();
	let text = "";
	if (output) {
		const lines = output.split("\n");
		const maxLines = options.expanded ? lines.length : 15;
		const displayLines = lines.slice(0, maxLines);
		const remaining = lines.length - maxLines;
		text += `\n${displayLines.map((line) => theme.fg("toolOutput", line)).join("\n")}`;
		if (remaining > 0) {
			text += `${theme.fg("muted", `\n... (${remaining} more lines,`)} ${keyHint("app.tools.expand", "to expand")})`;
		}
	}
	return text;
}

export function createSearchToolDefinition(
	cwd: string,
	options?: SearchToolOptions,
): ToolDefinition<typeof searchSchema, SearchToolDetails | undefined> {
	return {
		name: "SearchCodebase",
		label: "SearchCodebase",
		// The search-vs-bash split is stated once, by buildSystemPrompt, whenever both
		// are registered — so it is deliberately absent here. What stays is what only
		// this tool knows: that the query is not a regex, and that it degrades to
		// exact-text when the index is missing.
		description:
			"Find where code lives: ranked file:line-range results, fusing keyword and semantic retrieval over a local index with exact-text search of files the index has not read yet. The query is plain text, not a regex — regex metacharacters are matched literally. Falls back to exact-text retrieval automatically when the index is unavailable, and still finds code written moments ago that no index has seen.",
		promptSnippet: "Ranked code search (keyword + semantic, rank-fused)",
		promptGuidelines: [
			"SearchCodebase defaults to mode=auto, which is almost always right. Use limit=3 for targeted lookups, 10–20 when exploring a broad topic — past ~15 results the deeper ones arrive as ranked file:line-range headers without a snippet, which is still enough to choose what to read.",
		],
		parameters: searchSchema,
		async execute(_toolCallId, { query, mode, glob, limit }: SearchToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");

			const result = await runSearch({
				cwd,
				query,
				mode,
				glob,
				limit: Math.min(MAX_RESULTS, Math.max(1, limit ?? DEFAULT_RESULTS)),
				service: options?.getService?.(),
				signal,
			});
			if (signal?.aborted) throw new Error("Operation aborted");

			const details: SearchToolDetails = {
				resultCount: result.resultCount,
				resolvedMode: result.resolvedMode,
				indexing: result.indexing,
			};

			const notices: string[] = [];
			if (result.degradedReason) notices.push(result.degradedReason);
			if (result.indexing) {
				notices.push(
					`index still building: ${result.indexing.done}/${result.indexing.total} chunks embedded — results may be incomplete`,
				);
			}
			const notice = notices.length > 0 ? `\n\n[${notices.join(". ")}]` : "";

			if (result.resultCount === 0) {
				return {
					content: [
						{ type: "text" as const, text: `No results for "${query}" (${result.resolvedMode})${notice}` },
					],
					details,
				};
			}
			return {
				content: [{ type: "text" as const, text: result.text + notice }],
				details,
			};
		},
		renderCall(args, theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatSearchCall(args, theme));
			return text;
		},
		renderResult(result, options, theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatSearchResult(result as any, options, theme, context.showImages));
			return text;
		},
	};
}
