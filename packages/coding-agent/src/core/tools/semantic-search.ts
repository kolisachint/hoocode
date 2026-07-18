import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import type { EmbsearchService } from "../embsearch/embsearch-service.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import { getTextOutput, invalidArgText, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";

const DEFAULT_RESULTS = 10;
const MAX_RESULTS = 30;

const semanticSearchSchema = Type.Object({
	query: Type.String({
		description:
			"Natural-language description of the code you are looking for, e.g. 'where sessions are persisted to disk'",
	}),
	limit: Type.Optional(
		Type.Number({ description: `Maximum number of results (default: ${DEFAULT_RESULTS}, max: ${MAX_RESULTS})` }),
	),
});

export type SemanticSearchToolInput = Static<typeof semanticSearchSchema>;

export interface SemanticSearchToolDetails {
	resultCount?: number;
	indexing?: { done: number; total: number };
}

export interface SemanticSearchToolOptions {
	/**
	 * Provider for the per-session embsearch service. Resolved lazily at call
	 * time because the service is created (and its index built) after tool
	 * registration.
	 */
	getService?: () => EmbsearchService | undefined;
}

function formatSemanticSearchCall(
	args: { query?: string; limit?: number } | undefined,
	theme: typeof import("../../modes/interactive/theme/theme.js").theme,
): string {
	const query = str(args?.query);
	const queryDisplay = query === null ? invalidArgText(theme) : `"${query}"`;
	let text = theme.fg("toolTitle", theme.bold("semantic_search ")) + theme.fg("accent", queryDisplay);
	if (args?.limit !== undefined) text += theme.fg("muted", ` (${args.limit})`);
	return text;
}

function formatSemanticSearchResult(
	result: { content: Array<{ type: string; text?: string }>; details?: SemanticSearchToolDetails },
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

export function createSemanticSearchToolDefinition(
	_cwd: string,
	options?: SemanticSearchToolOptions,
): ToolDefinition<typeof semanticSearchSchema, SemanticSearchToolDetails | undefined> {
	return {
		name: "semantic_search",
		label: "semantic_search",
		description:
			"Search the repository by meaning rather than exact text: describe what the code does and get ranked file:line-range hits from a local embedding index. Complements grep (exact patterns) — use this when you don't know the identifier or wording. Results are ranked by similarity; read the top hits to verify. Off by default; enabled with --enable-embsearchtools on sufficiently large repos.",
		promptSnippet: "Search code by meaning (local embedding index)",
		promptGuidelines: [
			"Use semantic_search when you know what the code does but not what it is called; use grep when you know the exact identifier or pattern.",
		],
		parameters: semanticSearchSchema,
		async execute(_toolCallId, { query, limit }: SemanticSearchToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");

			const service = options?.getService?.();
			if (!service || !service.isAvailable()) {
				const state = service?.getState();
				const reason =
					state?.phase === "unavailable"
						? state.reason
						: state?.phase === "skipped"
							? state.reason
							: "semantic index is not available for this repository";
				throw new Error(`semantic_search unavailable: ${reason}. Use grep/find instead.`);
			}

			const effectiveLimit = Math.min(MAX_RESULTS, Math.max(1, limit ?? DEFAULT_RESULTS));
			const hits = await service.search(query, effectiveLimit);
			if (signal?.aborted) throw new Error("Operation aborted");

			const details: SemanticSearchToolDetails = { resultCount: hits.length };
			const state = service.getState();
			let notice = "";
			if (state.phase === "indexing") {
				details.indexing = { done: state.done, total: state.total };
				notice = `\n\n[Index still building: ${state.done}/${state.total} chunks embedded — results may be incomplete]`;
			}

			if (hits.length === 0) {
				return {
					content: [{ type: "text" as const, text: `No semantically similar code found for "${query}"${notice}` }],
					details,
				};
			}

			const lines = hits.map((h) => `${h.path}:${h.startLine}-${h.endLine} (${h.score.toFixed(3)})`);
			return {
				content: [{ type: "text" as const, text: lines.join("\n") + notice }],
				details,
			};
		},
		renderCall(args, theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatSemanticSearchCall(args, theme));
			return text;
		},
		renderResult(result, options, theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatSemanticSearchResult(result as any, options, theme, context.showImages));
			return text;
		},
	};
}

export function createSemanticSearchTool(
	cwd: string,
	options?: SemanticSearchToolOptions,
): AgentTool<typeof semanticSearchSchema> {
	return wrapToolDefinition(createSemanticSearchToolDefinition(cwd, options));
}
