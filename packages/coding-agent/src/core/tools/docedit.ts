import { mkdtempSync, renameSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Container, Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { APP_NAME } from "../../config.js";
import { theme as appTheme } from "../../modes/interactive/theme/theme.js";
import type { ToolDefinition } from "../extensions/types.js";
import { withFileMutationQueue } from "./file-mutation-queue.js";
import {
	extractDocument,
	getExtractRecord,
	invalidateExtractRecord,
	patchOpsSchema,
	reconstructDocument,
	toPatch,
} from "./filetools-shared.js";
import { resolveReadPath } from "./path-utils.js";
import { invalidArgText, shortenPath, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";

const docEditSchema = Type.Object({
	path: Type.String({
		description: "Path to the document to edit in place. Must have been opened with DocRead first.",
	}),
	patch: patchOpsSchema,
});

export type DocEditToolInput = Static<typeof docEditSchema>;

export interface DocEditToolDetails {
	ops?: number;
}

export interface DocEditToolOptions {
	/** Timeout (seconds) for the filetools invocations. */
	timeoutSecs?: number;
}

function formatDocEditCall(args: { path?: string; patch?: unknown[] } | undefined): string {
	const path = str(args?.path);
	const pathDisplay =
		path === null ? invalidArgText(appTheme) : path ? shortenPath(path) : appTheme.fg("toolOutput", "...");
	let text = appTheme.fg("toolTitle", appTheme.bold("DocEdit ")) + appTheme.fg("accent", pathDisplay);
	const ops = Array.isArray(args?.patch) ? args.patch.length : undefined;
	if (ops !== undefined) text += appTheme.fg("muted", ` (${ops} op${ops === 1 ? "" : "s"})`);
	return text;
}

export function createDocEditToolDefinition(
	cwd: string,
	options?: DocEditToolOptions,
): ToolDefinition<typeof docEditSchema, DocEditToolDetails | undefined> {
	return {
		name: "DocEdit",
		label: "DocEdit",
		description:
			"Apply an id-based patch to a structured/binary document IN PLACE, losslessly. Requires a prior DocRead of the same file (the patch targets node ids from that extract). Untouched bytes are preserved exactly; the file is then re-extracted so ids stay current for further edits. Off by default; enabled with --enable-filetools.",
		promptSnippet: "Patch a structured/binary document in place by node id",
		promptGuidelines: [
			"Use DocEdit to modify a document opened with DocRead; pass a patch of id-based ops (replace/add/remove) targeting the #ids from the extract. It edits in place and re-extracts.",
		],
		parameters: docEditSchema,
		async execute(_toolCallId, { path, patch }: DocEditToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");
			const absolutePath = resolveReadPath(path, cwd);
			if (!getExtractRecord(absolutePath)) {
				throw new Error(
					`no extracted envelope for ${basename(absolutePath)} — run DocRead on it first, then DocEdit`,
				);
			}

			return withFileMutationQueue(absolutePath, async () => {
				// Reconstruct to a temp file, then atomically swap it into place so a
				// failed reconstruct never leaves a half-written document behind.
				const stageDir = mkdtempSync(join(tmpdir(), `${APP_NAME}-docedit-`));
				const stagePath = join(stageDir, basename(absolutePath));
				try {
					await reconstructDocument(absolutePath, toPatch(patch), stagePath, cwd, signal, {
						timeoutSecs: options?.timeoutSecs,
					});
					renameSync(stagePath, absolutePath);
				} finally {
					rmSync(stageDir, { recursive: true, force: true });
				}

				// The source changed, so the cached id-map is stale. Re-extract to
				// refresh ids for subsequent edits (best-effort; a failure here does
				// not undo the successful write).
				invalidateExtractRecord(absolutePath);
				try {
					await extractDocument(absolutePath, cwd, signal, { timeoutSecs: options?.timeoutSecs });
				} catch {
					// Re-extract failure leaves no cached record; the next DocEdit will
					// require a fresh DocRead, which is the correct fallback.
				}

				return {
					content: [
						{
							type: "text" as const,
							text: `Applied ${patch.length} op${patch.length === 1 ? "" : "s"} to ${path} (re-extracted; ids refreshed).`,
						},
					],
					details: { ops: patch.length },
				};
			});
		},
		renderCall(args, _theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatDocEditCall(args));
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

export function createDocEditTool(cwd: string, options?: DocEditToolOptions): AgentTool<typeof docEditSchema> {
	return wrapToolDefinition(createDocEditToolDefinition(cwd, options));
}
