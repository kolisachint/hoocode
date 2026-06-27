import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import { theme as appTheme } from "../../modes/interactive/theme/theme.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import {
	type ReadView,
	readDocumentBlocks,
	renderDocNodeLines,
	truncateRenderToTokenBudget,
} from "./filetools-shared.js";
import { resolveReadPath } from "./path-utils.js";
import { getTextOutput, invalidArgText, shortenPath, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";

const docPeekSchema = Type.Object({
	path: Type.String({
		description: "Path to the document to read (relative or absolute). XML, drawio, OOXML (docx/xlsx/pptx), or PDF.",
	}),
	id: Type.Optional(
		Type.Array(Type.String(), {
			description:
				"Block id(s) to hydrate — use the EXACT ids returned by a prior DocScan (e.g. 'node[title:0]', 'paragraph[3]', 'page[1]'); do NOT hand-construct sub-ranges (an invented 'rows[0-2]' returns nothing — pass the exact 'rows[0-99]' DocScan emitted). These are DocScan's path ids, not DocGrep's el_ node ids. Omit to read a paginated slice of all blocks.",
		}),
	),
	offset: Type.Optional(Type.Number({ description: "Pagination start when no ids are given. Default 0." })),
	limit: Type.Optional(Type.Number({ description: "Pagination size when no ids are given. Default: all remaining." })),
});

export type DocPeekToolInput = Static<typeof docPeekSchema>;

export interface DocPeekToolDetails {
	returned?: number;
	total?: number;
}

export interface DocPeekToolOptions {
	/** Timeout (seconds) for the filetools invocation. */
	timeoutSecs?: number;
}

/** Render hydrated read blocks using the same id-addressed node dialect as DocRead. */
export function renderReadView(view: ReadView): string {
	const header = `document — ${view.returned}/${view.total} blocks (offset ${view.offset})`;
	const lines: string[] = [header, "", ...renderDocNodeLines(view.nodes)];
	const { text, droppedLines } = truncateRenderToTokenBudget(lines);
	const seen = view.offset + view.returned;
	const remaining = view.total - seen;
	const parts = [text];
	if (droppedLines > 0) {
		parts.push(
			`\n[Truncated: ${droppedLines} more node line${droppedLines === 1 ? "" : "s"} omitted; ` +
				`request fewer ids or a smaller limit.]`,
		);
	}
	if (remaining > 0) {
		parts.push(
			`\n[${remaining} more block${remaining === 1 ? "" : "s"} not shown — re-run with offset:${seen} to continue.]`,
		);
	}
	return parts.join("\n");
}

function formatDocPeekCall(
	args: { path?: string; id?: string[]; offset?: number; limit?: number } | undefined,
): string {
	const path = str(args?.path);
	const pathDisplay =
		path === null ? invalidArgText(appTheme) : path ? shortenPath(path) : appTheme.fg("toolOutput", "...");
	let text = appTheme.fg("toolTitle", appTheme.bold("DocPeek ")) + appTheme.fg("accent", pathDisplay);
	const ids = Array.isArray(args?.id) ? args.id.length : 0;
	if (ids > 0) {
		text += appTheme.fg("muted", ` (${ids} id${ids === 1 ? "" : "s"})`);
	} else {
		const range: string[] = [];
		if (args?.offset !== undefined) range.push(`offset ${args.offset}`);
		if (args?.limit !== undefined) range.push(`limit ${args.limit}`);
		if (range.length > 0) text += appTheme.fg("muted", ` (${range.join(", ")})`);
	}
	return text;
}

function formatDocPeekResult(
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

export function createDocPeekToolDefinition(
	cwd: string,
	options?: DocPeekToolOptions,
): ToolDefinition<typeof docPeekSchema, DocPeekToolDetails | undefined> {
	return {
		name: "DocPeek",
		label: "DocPeek",
		description:
			"Hydrate specific blocks of a structured/binary document (XML, drawio, docx/xlsx/pptx, PDF) by structural-path id — the read half of the token-sensitive loop. Pass the path ids from a prior DocScan (e.g. node[title:0]) to read only those blocks (or omit ids to page through all with offset/limit), instead of dumping the whole document with DocRead. The hydrated nodes carry the editable el_ #ids, which you then patch with DocEdit/DocWrite. Read-only. Off by default; enabled with --enable-filetools.",
		promptSnippet: "Read only specific blocks of a structured/binary document by id",
		promptGuidelines: [
			"Use DocPeek to read just the blocks you need (by structural-path id from DocScan, e.g. node[title:0]) instead of a full DocRead — it is much cheaper in tokens. Omit ids and use offset/limit to page through a large document.",
			"Pass the EXACT ids DocScan returned; DocPeek does not accept invented sub-ranges (e.g. rows[0-2] when DocScan emitted rows[0-99]).",
			"DocPeek takes DocScan's path ids, NOT DocGrep's el_ node ids. The nodes it returns carry the editable el_ #ids — patch those with DocEdit/DocWrite (which auto-extract). If you already have an el_ id from DocGrep, go straight to DocEdit; you do not need DocPeek.",
			"Works across all supported formats including spreadsheet cells (hydrating a sheet's rows[a-b] block returns the cell values) and slide text (pptx).",
		],
		parameters: docPeekSchema,
		async execute(_toolCallId, { path, id, offset, limit }: DocPeekToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");
			const absolutePath = resolveReadPath(path, cwd);
			const view = await readDocumentBlocks(absolutePath, cwd, signal, {
				ids: id,
				offset,
				limit,
				timeoutSecs: options?.timeoutSecs,
			});
			return {
				content: [{ type: "text" as const, text: renderReadView(view) }],
				details: { returned: view.returned, total: view.total },
			};
		},
		renderCall(args, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocPeekCall(args));
			return text;
		},
		renderResult(result, options, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocPeekResult(result as any, options, context.showImages));
			return text;
		},
	};
}

export function createDocPeekTool(cwd: string, options?: DocPeekToolOptions): AgentTool<typeof docPeekSchema> {
	return wrapToolDefinition(createDocPeekToolDefinition(cwd, options));
}
