/**
 * The unified `search` tool: ranked code discovery in lexical, semantic, or
 * hybrid (rank-fused) mode. Replaces the old `semantic_search` tool — see
 * docs/hybrid-retrieval-design.md, Decision 1. `grep` stays separate for
 * exact line-level mechanics; this tool answers "find where X lives".
 */

import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import type { EmbsearchService } from "../embsearch/embsearch-service.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import { runSearch } from "../search/hybrid-search.js";
import type { ResolvedSearchMode } from "../search/types.js";
import { getTextOutput, invalidArgText, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";

// Default of 5 balances recall against context cost: eval showed limit=5
// preserving the top-rank hits of limit=10 at roughly half the tokens.
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
				"Retrieval mode (default: auto). auto picks hybrid when the semantic index is available; lexical = exact-text only; semantic = embedding index only; hybrid = both, fused by rank.",
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

export type SearchToolInput = Static<typeof searchSchema>;

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
	let text = theme.fg("toolTitle", theme.bold("search ")) + theme.fg("accent", queryDisplay);
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
		name: "search",
		label: "search",
		description:
			"Find where code lives: ranked file:line-range results from exact-text and semantic (local embedding index) retrieval, fused by rank when both are available. Use search when you want to locate something — a concept, a behavior, or an identifier. Use grep when you want exact matching lines (call sites, counts, context). The query is plain text, not a regex — regex metacharacters are matched literally. Falls back to exact-text retrieval automatically when the semantic index is unavailable.",
		promptSnippet: "Ranked code search (exact + semantic, rank-fused)",
		promptGuidelines: [
			"Start with search when you need to find where code lives, identify relevant files, or match a concept, behavior, or half-known name. Use grep when you need exact-line enumeration, regexes, call-site counts, or raw context.",
			"The default mode (auto) is almost always right — only force lexical/semantic/hybrid deliberately. Use limit=3 for very targeted lookups; raise limit to 10–20 when exploring a broad topic.",
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

export function createSearchTool(cwd: string, options?: SearchToolOptions): AgentTool<typeof searchSchema> {
	return wrapToolDefinition(createSearchToolDefinition(cwd, options));
}
