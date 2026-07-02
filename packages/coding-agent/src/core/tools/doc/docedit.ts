import { mkdtempSync, renameSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Container, Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { APP_NAME } from "../../../config.js";
import { theme as appTheme } from "../../../modes/interactive/theme/theme.js";
import type { ToolDefinition } from "../../extensions/types.js";
import { withFileMutationQueue } from "../file-mutation-queue.js";
import {
	type DocNode,
	ensureExtractRecord,
	extractDocument,
	findNodeById,
	invalidateExtractRecord,
	patchOpNodeId,
	patchOpsSchema,
	reconstructDocument,
	StalePatchError,
	toPatch,
} from "../filetools-shared.js";
import { resolveReadPath } from "../path-utils.js";
import { invalidArgText, shortenPath, str } from "../render-utils.js";
import { wrapToolDefinition } from "../tool-definition-wrapper.js";
import { renderEnvelopeText } from "./docread.js";

const docEditSchema = Type.Object({
	path: Type.String({
		description: "Path to the document to edit in place. Must have been opened with DocRead first.",
	}),
	patch: patchOpsSchema,
});

export type DocEditToolInput = Static<typeof docEditSchema>;

export interface DocEditToolDetails {
	ops?: number;
	affected?: DocNode[];
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
			"Apply an id-based patch to a structured/binary document IN PLACE, losslessly. Targets node ids from a DocRead extract; if the cache is missing or the file changed on disk (e.g. an external script rewrote it) the document is re-extracted automatically, so a separate DocRead is not required. If the patch references ids that no longer exist, the call fails and returns the current structure with fresh ids so you can re-issue. Untouched bytes are preserved exactly; the file is re-extracted after the write so ids stay current for further edits. This is the canonical way to edit these formats: never fall back to ad-hoc scripts (python/openpyxl, docx, PyPDF2, unzip, sed) to rewrite them — that bypasses the lossless id-map and corrupts the file. Off by default; enabled with --enable-filetools.",
		promptSnippet: "Patch a structured/binary document in place by node id",
		promptGuidelines: [
			"Use DocEdit to modify a document opened with DocRead; pass a patch of id-based ops (replace/add/remove) targeting the #ids from the extract. It edits in place and re-extracts.",
			"Keep patches minimal — only the ops you actually need. DocEdit re-extracts after writing and refreshes ids, so do not DocRead again between edits; the full extract is token-heavy. Reach for a fresh writable DocRead only when ids have gone stale and the failure output isn't enough to continue.",
		],
		parameters: docEditSchema,
		async execute(_toolCallId, { path, patch }: DocEditToolInput, signal?: AbortSignal) {
			if (signal?.aborted) throw new Error("Operation aborted");
			const absolutePath = resolveReadPath(path, cwd);

			return withFileMutationQueue(absolutePath, async () => {
				// Resolve affected nodes from the pre-reconstruct extract: the patch's
				// ids are valid against THIS extract, and a later re-extract
				// regenerates ids that would not match the patch. reconstructDocument
				// auto-extracts when the cache is missing/stale, so capture the record
				// the same way to stay in sync with what the patch is applied against.
				const preRecord = await ensureExtractRecord(absolutePath, cwd, signal, {
					timeoutSecs: options?.timeoutSecs,
				});
				const affectedIds = new Set<string>();
				for (const op of toPatch(patch).patch) {
					const id = patchOpNodeId(op);
					if (id) affectedIds.add(id);
				}
				const affectedNodes: DocNode[] = [];
				for (const id of affectedIds) {
					const node = findNodeById(preRecord.envelope.structure, id);
					if (node) affectedNodes.push(node);
				}

				// Reconstruct to a temp file, then atomically swap it into place so a
				// failed reconstruct never leaves a half-written document behind.
				const stageDir = mkdtempSync(join(tmpdir(), `${APP_NAME}-docedit-`));
				const stagePath = join(stageDir, basename(absolutePath));
				try {
					await reconstructDocument(absolutePath, toPatch(patch), stagePath, cwd, signal, {
						timeoutSecs: options?.timeoutSecs,
					});
					renameSync(stagePath, absolutePath);
				} catch (err) {
					if (err instanceof StalePatchError) {
						// The document was rewritten out-of-band: the cache auto-refreshed
						// but the patch targets ids that no longer exist. Surface the
						// current structure so the agent can re-issue without a DocRead.
						throw new Error(`${err.message}\n\n${renderEnvelopeText(err.envelope, false)}`);
					}
					throw err;
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
					// auto-extract (or fall back to DocRead), which is the correct path.
				}

				return {
					content: [
						{
							type: "text" as const,
							text: `Applied ${patch.length} op${patch.length === 1 ? "" : "s"} to ${path} (re-extracted; ids refreshed).`,
						},
					],
					details: { ops: patch.length, affected: affectedNodes.length > 0 ? affectedNodes : undefined },
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
