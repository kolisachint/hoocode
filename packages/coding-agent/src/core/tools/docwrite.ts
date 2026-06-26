import { mkdir as fsMkdir } from "node:fs/promises";
import { dirname } from "node:path";
import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Container, Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { theme as appTheme } from "../../modes/interactive/theme/theme.js";
import type { ToolDefinition } from "../extensions/types.js";
import { withFileMutationQueue } from "./file-mutation-queue.js";
import { getExtractRecord, patchOpsSchema, reconstructDocument, toPatch } from "./filetools-shared.js";
import { resolveReadPath, resolveToCwd } from "./path-utils.js";
import { invalidArgText, shortenPath, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";

const docWriteSchema = Type.Object({
	path: Type.String({
		description: "Path to the SOURCE document. Must have been opened with DocRead first. Left untouched.",
	}),
	out: Type.String({
		description: "Output path for the reconstructed document (relative or absolute). Created/overwritten.",
	}),
	patch: patchOpsSchema,
});

export type DocWriteToolInput = Static<typeof docWriteSchema>;

export interface DocWriteToolDetails {
	ops?: number;
	out?: string;
}

export interface DocWriteToolOptions {
	/** Timeout (seconds) for the filetools invocation. */
	timeoutSecs?: number;
}

function formatDocWriteCall(args: { path?: string; out?: string; patch?: unknown[] } | undefined): string {
	const path = str(args?.path);
	const out = str(args?.out);
	const pathDisplay =
		path === null ? invalidArgText(appTheme) : path ? shortenPath(path) : appTheme.fg("toolOutput", "...");
	const outDisplay =
		out === null ? invalidArgText(appTheme) : out ? shortenPath(out) : appTheme.fg("toolOutput", "...");
	let text =
		appTheme.fg("toolTitle", appTheme.bold("DocWrite ")) +
		appTheme.fg("accent", pathDisplay) +
		appTheme.fg("muted", " -> ") +
		appTheme.fg("accent", outDisplay);
	const ops = Array.isArray(args?.patch) ? args.patch.length : undefined;
	if (ops !== undefined) text += appTheme.fg("muted", ` (${ops} op${ops === 1 ? "" : "s"})`);
	return text;
}

export function createDocWriteToolDefinition(
	cwd: string,
	options?: DocWriteToolOptions,
): ToolDefinition<typeof docWriteSchema, DocWriteToolDetails | undefined> {
	return {
		name: "DocWrite",
		label: "DocWrite",
		description:
			"Apply an id-based patch to a structured/binary document and write the result to a NEW path, leaving the source untouched (save-as). Requires a prior DocRead of the source (the patch targets node ids from that extract). Off by default; enabled with --enable-filetools.",
		promptSnippet: "Reconstruct a patched structured/binary document to a new path",
		promptGuidelines: [
			"Use DocWrite to save an edited document to a different file: pass the source path (opened with DocRead), an `out` path, and an id-based patch. The source is left unchanged.",
		],
		parameters: docWriteSchema,
		async execute(_toolCallId, { path, out, patch }: DocWriteToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");
			const absolutePath = resolveReadPath(path, cwd);
			if (!getExtractRecord(absolutePath)) {
				throw new Error(`no extracted envelope for the source — run DocRead on ${path} first, then DocWrite`);
			}
			const outPath = resolveToCwd(out, cwd);

			return withFileMutationQueue(outPath, async () => {
				await fsMkdir(dirname(outPath), { recursive: true });
				await reconstructDocument(absolutePath, toPatch(patch), outPath, cwd, signal, {
					timeoutSecs: options?.timeoutSecs,
				});
				return {
					content: [
						{
							type: "text" as const,
							text: `Wrote ${patch.length} op${patch.length === 1 ? "" : "s"} from ${path} to ${out}.`,
						},
					],
					details: { ops: patch.length, out },
				};
			});
		},
		renderCall(args, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocWriteCall(args));
			return text;
		},
		renderResult(result, _options, _theme, context) {
			if (!context.isError) {
				const component = (context.lastComponent as Container | undefined) ?? new Container();
				component.clear();
				return component;
			}
			const output = (result.content as Array<{ type: string; text?: string }>)
				.filter((c) => c.type === "text")
				.map((c) => c.text || "")
				.join("\n");
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(output ? `\n${appTheme.fg("error", output)}` : "");
			return text;
		},
	};
}

export function createDocWriteTool(cwd: string, options?: DocWriteToolOptions): AgentTool<typeof docWriteSchema> {
	return wrapToolDefinition(createDocWriteToolDefinition(cwd, options));
}
