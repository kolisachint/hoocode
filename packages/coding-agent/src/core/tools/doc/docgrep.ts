import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { keyHint } from "../../../modes/interactive/components/keybinding-hints.js";
import { theme as appTheme } from "../../../modes/interactive/theme/theme.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../../extensions/types.js";
import { type GrepView, grepDocument, truncateRenderToTokenBudget } from "../filetools-shared.js";
import { resolveReadPath } from "../path-utils.js";
import { getTextOutput, invalidArgText, shortenPath, str } from "../render-utils.js";
import { wrapToolDefinition } from "../tool-definition-wrapper.js";

const docGrepSchema = Type.Object({
	path: Type.String({
		description:
			"Path to the document to search (relative or absolute). XML, drawio, OOXML (docx/xlsx/pptx), or PDF.",
	}),
	pattern: Type.String({
		description: "Literal substring to match per line (NOT a regex).",
	}),
	ignoreCase: Type.Optional(Type.Boolean({ description: "Case-insensitive matching. Default false." })),
	limit: Type.Optional(Type.Number({ description: "Stop after N matches. Default: all matches." })),
});

export type DocGrepToolInput = Static<typeof docGrepSchema>;

export interface DocGrepToolDetails {
	pattern?: string;
	matches?: number;
}

export interface DocGrepToolOptions {
	/** Timeout (seconds) for the filetools invocation. */
	timeoutSecs?: number;
}

/**
 * Render grep matches as `#block_id:line :: "snippet"` lines. Each block id
 * feeds straight back into DocPeek (to hydrate) or a DocEdit patch.
 */
export function renderGrepView(view: GrepView): string {
	const header = `grep ${JSON.stringify(view.pattern)} — ${view.returned} match${view.returned === 1 ? "" : "es"}`;
	if (view.matches.length === 0) return header;
	const lines: string[] = [header, ""];
	for (const m of view.matches) {
		const flag = m.writable ? "" : " (read-only)";
		lines.push(`#${m.block_id}:${m.line}${flag} :: ${JSON.stringify(m.snippet)}`);
	}
	const { text, droppedLines } = truncateRenderToTokenBudget(lines);
	if (droppedLines === 0) return text;
	return (
		`${text}\n\n[Truncated: ${droppedLines} more match line${droppedLines === 1 ? "" : "s"} omitted; ` +
		`re-run with a smaller limit or a more specific pattern.]`
	);
}

function formatDocGrepCall(args: { path?: string; pattern?: string; ignoreCase?: boolean } | undefined): string {
	const path = str(args?.path);
	const pathDisplay =
		path === null ? invalidArgText(appTheme) : path ? shortenPath(path) : appTheme.fg("toolOutput", "...");
	let text = appTheme.fg("toolTitle", appTheme.bold("DocGrep "));
	const pattern = str(args?.pattern);
	if (pattern) text += `${appTheme.fg("accent", JSON.stringify(pattern))} ${appTheme.fg("muted", "in")} `;
	text += appTheme.fg("accent", pathDisplay);
	if (args?.ignoreCase) text += appTheme.fg("muted", " (i)");
	return text;
}

function formatDocGrepResult(
	result: { content: Array<{ type: string; text?: string }> },
	options: ToolRenderResultOptions,
	showImages: boolean,
): string {
	const output = getTextOutput(result as any, showImages).trim();
	if (!output) return "";
	const lines = output.split("\n");
	const maxLines = options.expanded ? lines.length : 15;
	const displayLines = lines.slice(0, maxLines);
	const remaining = lines.length - maxLines;
	let text = `\n${displayLines.map((line) => appTheme.fg("toolOutput", line)).join("\n")}`;
	if (remaining > 0) {
		text += `${appTheme.fg("muted", `\n... (${remaining} more lines,`)} ${keyHint("app.tools.expand", "to expand")})`;
	}
	return text;
}

export function createDocGrepToolDefinition(
	cwd: string,
	options?: DocGrepToolOptions,
): ToolDefinition<typeof docGrepSchema, DocGrepToolDetails | undefined> {
	return {
		name: "DocGrep",
		label: "DocGrep",
		description:
			"Locate blocks in a structured/binary document (XML, drawio, docx/xlsx/pptx, PDF) by literal text, WITHOUT hydrating the whole document. Returns each match as an editable el_ node #id, line number, and snippet. Those el_ #ids target a DocEdit/DocWrite patch directly — DocGrep is the fast path from 'find text' to 'edit it' (far cheaper than a full DocRead). Off by default; enabled with --enable-filetools.",
		promptSnippet: "Find blocks in a structured/binary document by literal text, without hydrating it",
		promptGuidelines: [
			"Use DocGrep to find where something is in a large structured/binary document instead of reading it all: it returns editable el_ node #ids + snippets for a literal substring match. Patch those el_ #ids straight with DocEdit/DocWrite.",
			"DocGrep's el_ #ids are the edit id-space; they are NOT DocPeek's structural-path ids. To read more context around a match, DocScan/DocPeek by structural path instead.",
			"pattern is a literal substring (not a regex); pass ignoreCase for case-insensitive matching.",
			"Works across all supported formats including spreadsheet cell values (xlsx) and slide text (pptx); patch a matched el_ #id with DocEdit, or read more context with DocPeek.",
		],
		parameters: docGrepSchema,
		async execute(_toolCallId, { path, pattern, ignoreCase, limit }: DocGrepToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");
			const absolutePath = resolveReadPath(path, cwd);
			const view = await grepDocument(absolutePath, pattern, cwd, signal, {
				ignoreCase,
				limit,
				timeoutSecs: options?.timeoutSecs,
			});
			return {
				content: [{ type: "text" as const, text: renderGrepView(view) }],
				details: { pattern: view.pattern, matches: view.returned },
			};
		},
		renderCall(args, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocGrepCall(args));
			return text;
		},
		renderResult(result, options, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocGrepResult(result as any, options, context.showImages));
			return text;
		},
	};
}

export function createDocGrepTool(cwd: string, options?: DocGrepToolOptions): AgentTool<typeof docGrepSchema> {
	return wrapToolDefinition(createDocGrepToolDefinition(cwd, options));
}
