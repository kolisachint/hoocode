import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import { theme as appTheme } from "../../modes/interactive/theme/theme.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import {
	DOCREAD_MAX_RENDER_TOKENS,
	type DocNode,
	type Envelope,
	extractDocument,
	renderDocNodeLines,
	truncateRenderToTokenBudget,
} from "./filetools-shared.js";
import { resolveReadPath } from "./path-utils.js";
import { getTextOutput, invalidArgText, shortenPath, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";

const docReadSchema = Type.Object({
	path: Type.String({
		description:
			"Path to the document to extract (relative or absolute). XML, drawio, OOXML (docx/xlsx/pptx), or PDF.",
	}),
	readonly: Type.Optional(
		Type.Boolean({
			description:
				"Analysis-only projection: strips node ids for a smaller view that CANNOT be edited. Omit (default false) when you intend to DocEdit/DocWrite afterwards.",
		}),
	),
});

export type DocReadToolInput = Static<typeof docReadSchema>;

export interface DocReadToolDetails {
	type?: string;
	fidelity?: string;
	writable?: boolean;
	nodeCount?: number;
}

export interface DocReadToolOptions {
	/** Timeout (seconds) for the filetools invocation. */
	timeoutSecs?: number;
}

function countNodes(nodes: DocNode[]): number {
	let total = 0;
	for (const node of nodes) {
		total += 1 + (node.children ? countNodes(node.children) : 0);
	}
	return total;
}

/**
 * Render the envelope as compact, id-addressed text the model edits against.
 * Each line carries the node id so DocEdit/DocWrite patches can target it.
 *
 * The filetools binary has no pagination, so a dense document can project into
 * a huge dump. We truncate the rendered view to a token budget and append an
 * actionable notice instead of flooding the model context.
 */
export function renderEnvelopeText(envelope: Envelope, readonly: boolean | undefined): string {
	const header =
		`document ${envelope.source.path} [${envelope.source.type}, ${envelope.fidelity}, ` +
		`${envelope.writable ? "writable" : "read-only"}]`;
	const lines: string[] = [header, "", ...renderDocNodeLines(envelope.structure)];

	const { text, droppedLines } = truncateRenderToTokenBudget(lines);
	if (droppedLines === 0) return text;

	const hint = readonly
		? "narrow to a smaller or more targeted file"
		: "re-run with readonly:true for a smaller analysis-only view, or target a smaller file";
	return (
		`${text}\n\n[Truncated: document exceeds the ~${DOCREAD_MAX_RENDER_TOKENS} token render budget; ` +
		`${droppedLines} more node line${droppedLines === 1 ? "" : "s"} omitted. The ids shown are still valid for ` +
		`DocEdit/DocWrite — ${hint}.]`
	);
}

function formatDocReadCall(args: { path?: string; readonly?: boolean } | undefined): string {
	const path = str(args?.path);
	const pathDisplay =
		path === null ? invalidArgText(appTheme) : path ? shortenPath(path) : appTheme.fg("toolOutput", "...");
	let text = appTheme.fg("toolTitle", appTheme.bold("DocRead ")) + appTheme.fg("accent", pathDisplay);
	if (args?.readonly) text += appTheme.fg("muted", " (readonly)");
	return text;
}

function formatDocReadResult(
	result: { content: Array<{ type: string; text?: string }>; details?: DocReadToolDetails },
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

export function createDocReadToolDefinition(
	cwd: string,
	options?: DocReadToolOptions,
): ToolDefinition<typeof docReadSchema, DocReadToolDetails | undefined> {
	return {
		name: "DocRead",
		label: "DocRead",
		description:
			"Extract a structured or binary document (XML, drawio, docx/xlsx/pptx, PDF) into editable, id-addressed JSON the agent can patch losslessly. Each node has a stable #id; edit with DocEdit (in place) or DocWrite (to a new path), passing a patch that targets those ids. This is the canonical way to read and edit these formats: never fall back to ad-hoc scripts (python/openpyxl, docx, PyPDF2, unzip, sed) to parse or rewrite them — that loses the lossless id-map and corrupts formatting. Off by default; enabled with --enable-filetools.",
		promptSnippet: "Extract a structured/binary document into editable, id-addressed JSON",
		promptGuidelines: [
			"Use DocRead to open structured/binary documents (XML, drawio, docx/xlsx/pptx, PDF) instead of read; it returns id-addressed nodes you can patch with DocEdit/DocWrite. Never fall back to ad-hoc scripts (python/openpyxl, docx, PyPDF2, unzip, sed) to parse or edit these formats — that loses the lossless id-map and corrupts the file.",
			"Flow: scan first, edit second. To understand a document, start with DocRead readonly:true — an analysis-only projection that strips node ids and is much cheaper in tokens. Only do a full (writable) DocRead when you actually intend to edit, since that view carries the whole id-map and is token-heavy.",
			"Read once, then edit. A writable DocRead establishes the id-map; DocEdit/DocWrite re-extract on their own, so do not re-run DocRead between edits. If a patch targets stale ids the edit fails and returns the current structure with fresh ids to re-issue against.",
		],
		parameters: docReadSchema,
		async execute(_toolCallId, { path, readonly }: DocReadToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");
			const absolutePath = resolveReadPath(path, cwd);
			const envelope = await extractDocument(absolutePath, cwd, signal, {
				readonly,
				timeoutSecs: options?.timeoutSecs,
			});
			return {
				content: [{ type: "text" as const, text: renderEnvelopeText(envelope, readonly) }],
				details: {
					type: envelope.source.type,
					fidelity: envelope.fidelity,
					writable: envelope.writable,
					nodeCount: countNodes(envelope.structure),
				},
			};
		},
		renderCall(args, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocReadCall(args));
			return text;
		},
		renderResult(result, options, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocReadResult(result as any, options, context.showImages));
			return text;
		},
	};
}

export function createDocReadTool(cwd: string, options?: DocReadToolOptions): AgentTool<typeof docReadSchema> {
	return wrapToolDefinition(createDocReadToolDefinition(cwd, options));
}
