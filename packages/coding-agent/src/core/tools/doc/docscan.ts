import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { keyHint } from "../../../modes/interactive/components/keybinding-hints.js";
import { theme as appTheme } from "../../../modes/interactive/theme/theme.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../../extensions/types.js";
import { type ScanView, scanDocument, truncateRenderToTokenBudget } from "../filetools-shared.js";
import { resolveReadPath } from "../path-utils.js";
import { getTextOutput, invalidArgText, shortenPath, str } from "../render-utils.js";
import { wrapToolDefinition } from "../tool-definition-wrapper.js";

const docScanSchema = Type.Object({
	path: Type.String({
		description: "Path to the document to scan (relative or absolute). XML, drawio, OOXML (docx/xlsx/pptx), or PDF.",
	}),
	offset: Type.Optional(Type.Number({ description: "Skip the first N blocks (pagination). Default 0." })),
	limit: Type.Optional(Type.Number({ description: "Maximum number of blocks to return. Default: all remaining." })),
});

export type DocScanToolInput = Static<typeof docScanSchema>;

export interface DocScanToolDetails {
	fileType?: string;
	returned?: number;
	total?: number;
}

export interface DocScanToolOptions {
	/** Timeout (seconds) for the filetools invocation. */
	timeoutSecs?: number;
}

/**
 * Render a scan manifest as compact, id-addressed preview lines. Each line
 * carries the block id (feeds DocPeek/DocGrep) plus a section label, token
 * estimate, and a short preview — structure without hydrating full content.
 */
export function renderScanView(view: ScanView): string {
	const header =
		`document ${view.file_type} — ${view.returned}/${view.total} blocks ` +
		`(offset ${view.offset}, ~${view.total_tokens} tok total)`;
	const lines: string[] = [header, ""];
	for (const block of view.blocks) {
		const section = block.section_name ? ` ${block.section_name}#${block.section_number}` : "";
		const preview = block.preview ? ` :: ${JSON.stringify(block.preview)}` : "";
		lines.push(`#${block.id} [${block.block_type}]${section} ~${block.token_estimate}tok${preview}`);
	}

	const { text, droppedLines } = truncateRenderToTokenBudget(lines);
	const seen = view.offset + view.returned;
	const remaining = view.total - seen;
	const parts = [text];
	if (droppedLines > 0) {
		parts.push(
			`\n[Truncated: ${droppedLines} more block line${droppedLines === 1 ? "" : "s"} omitted from this render. ` +
				`Re-run with a tighter limit, or DocGrep/DocPeek to target.]`,
		);
	}
	if (remaining > 0) {
		parts.push(
			`\n[${remaining} more block${remaining === 1 ? "" : "s"} not shown — re-run with offset:${seen} to continue.]`,
		);
	}
	return parts.join("\n");
}

function formatDocScanCall(args: { path?: string; offset?: number; limit?: number } | undefined): string {
	const path = str(args?.path);
	const pathDisplay =
		path === null ? invalidArgText(appTheme) : path ? shortenPath(path) : appTheme.fg("toolOutput", "...");
	let text = appTheme.fg("toolTitle", appTheme.bold("DocScan ")) + appTheme.fg("accent", pathDisplay);
	const range: string[] = [];
	if (args?.offset !== undefined) range.push(`offset ${args.offset}`);
	if (args?.limit !== undefined) range.push(`limit ${args.limit}`);
	if (range.length > 0) text += appTheme.fg("muted", ` (${range.join(", ")})`);
	return text;
}

function formatDocScanResult(
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

export function createDocScanToolDefinition(
	cwd: string,
	options?: DocScanToolOptions,
): ToolDefinition<typeof docScanSchema, DocScanToolDetails | undefined> {
	return {
		name: "DocScan",
		label: "DocScan",
		description:
			"Cheaply outline a structured/binary document (XML, drawio, docx/xlsx/pptx, PDF) without hydrating it: returns a paginated manifest of blocks, each with a structural-path id, type, section label, token estimate, and a short preview. This is the cheap first step of the token-sensitive loop — far smaller than a full DocRead. Pass the path ids it returns to DocPeek to hydrate just those blocks (the hydrated nodes carry the editable el_ #ids for DocEdit). Off by default; enabled with --enable-filetools.",
		promptSnippet: "Outline a structured/binary document into a cheap, paginated block manifest",
		promptGuidelines: [
			"Start here for large structured/binary documents instead of a full DocRead: DocScan returns a paginated outline (structural-path block ids + previews) that is much cheaper in tokens. Page through it with offset/limit.",
			"Flow: DocScan to see structure → DocPeek the path ids you want (hydrates them and reveals their editable el_ #ids) → DocEdit. Or jump straight in with DocGrep, which finds blocks by text and returns editable el_ #ids directly.",
		],
		parameters: docScanSchema,
		async execute(_toolCallId, { path, offset, limit }: DocScanToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");
			const absolutePath = resolveReadPath(path, cwd);
			const view = await scanDocument(absolutePath, cwd, signal, {
				offset,
				limit,
				timeoutSecs: options?.timeoutSecs,
			});
			return {
				content: [{ type: "text" as const, text: renderScanView(view) }],
				details: { fileType: view.file_type, returned: view.returned, total: view.total },
			};
		},
		renderCall(args, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocScanCall(args));
			return text;
		},
		renderResult(result, options, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocScanResult(result as any, options, context.showImages));
			return text;
		},
	};
}

export function createDocScanTool(cwd: string, options?: DocScanToolOptions): AgentTool<typeof docScanSchema> {
	return wrapToolDefinition(createDocScanToolDefinition(cwd, options));
}
