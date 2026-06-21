import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { existsSync, readdirSync, statSync } from "fs";
import { readdir as fsReaddir } from "fs/promises";
import { minimatch } from "minimatch";
import nodePath from "path";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import { resolveToCwd } from "./path-utils.js";
import { getTextOutput, invalidArgText, shortenPath, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";
import { DEFAULT_MAX_BYTES, formatSize, type TruncationResult, truncateHead } from "./truncate.js";

const lsSchema = Type.Object({
	path: Type.Optional(Type.String({ description: "Directory to list (default: current directory)" })),
	limit: Type.Optional(Type.Number({ description: "Maximum number of entries to return (default: 500)" })),
	ignore: Type.Optional(
		Type.Array(Type.String(), {
			description:
				"Glob patterns matched against entry names to exclude, e.g. ['node_modules', '*.log', '.git']. Matching is on the entry name only (ls is non-recursive); dotfiles are matched.",
		}),
	),
});

export type LsToolInput = Static<typeof lsSchema>;

const DEFAULT_LIMIT = 500;

export interface LsToolDetails {
	truncation?: TruncationResult;
	entryLimitReached?: number;
}

/**
 * Pluggable operations for the ls tool.
 * Override these to delegate directory listing to remote systems (for example SSH).
 */
export interface LsOperations {
	/** Check if path exists */
	exists: (absolutePath: string) => Promise<boolean> | boolean;
	/** Get file or directory stats. Throws if not found. */
	stat: (absolutePath: string) => Promise<{ isDirectory: () => boolean }> | { isDirectory: () => boolean };
	/** Read directory entries as strings (used when readdirEntries is not provided) */
	readdir: (absolutePath: string) => Promise<string[]> | string[];
	/**
	 * Optional: read directory entries with type info in a single syscall.
	 * When provided, eliminates per-entry stat() calls for the isDirectory check.
	 */
	readdirEntries?: (
		absolutePath: string,
	) =>
		| Promise<Array<{ name: string; isDirectory: () => boolean }>>
		| Array<{ name: string; isDirectory: () => boolean }>;
}

const defaultLsOperations: LsOperations = {
	exists: existsSync,
	stat: statSync,
	readdir: readdirSync,
	readdirEntries: (p) => fsReaddir(p, { withFileTypes: true }),
};

export interface LsToolOptions {
	/** Custom operations for directory listing. Default: local filesystem */
	operations?: LsOperations;
}

function formatLsCall(
	args: { path?: string; limit?: number } | undefined,
	theme: typeof import("../../modes/interactive/theme/theme.js").theme,
): string {
	const rawPath = str(args?.path);
	const path = rawPath !== null ? shortenPath(rawPath || ".") : null;
	const limit = args?.limit;
	const invalidArg = invalidArgText(theme);
	let text = `${theme.fg("toolTitle", theme.bold("ls"))} ${path === null ? invalidArg : theme.fg("accent", path)}`;
	if (limit !== undefined) {
		text += theme.fg("toolOutput", ` (limit ${limit})`);
	}
	return text;
}

function formatLsResult(
	result: {
		content: Array<{ type: string; text?: string; data?: string; mimeType?: string }>;
		details?: LsToolDetails;
	},
	options: ToolRenderResultOptions,
	theme: typeof import("../../modes/interactive/theme/theme.js").theme,
	showImages: boolean,
): string {
	const output = getTextOutput(result, showImages).trim();
	let text = "";
	if (output) {
		const lines = output.split("\n");
		const maxLines = options.expanded ? lines.length : 20;
		const displayLines = lines.slice(0, maxLines);
		const remaining = lines.length - maxLines;
		text += `\n${displayLines.map((line) => theme.fg("toolOutput", line)).join("\n")}`;
		if (remaining > 0) {
			text += `${theme.fg("muted", `\n... (${remaining} more lines,`)} ${keyHint("app.tools.expand", "to expand")})`;
		}
	}

	const entryLimit = result.details?.entryLimitReached;
	const truncation = result.details?.truncation;
	if (entryLimit || truncation?.truncated) {
		const warnings: string[] = [];
		if (entryLimit) warnings.push(`${entryLimit} entries limit`);
		if (truncation?.truncated) warnings.push(`${formatSize(truncation.maxBytes ?? DEFAULT_MAX_BYTES)} limit`);
		text += `\n${theme.fg("warning", `[Truncated: ${warnings.join(", ")}]`)}`;
	}
	return text;
}

export function createLsToolDefinition(
	cwd: string,
	options?: LsToolOptions,
): ToolDefinition<typeof lsSchema, LsToolDetails | undefined> {
	const ops = options?.operations ?? defaultLsOperations;
	return {
		name: "ls",
		label: "ls",
		description: `List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. Lists a single directory (not recursive) and shows everything on disk; pass 'ignore' glob patterns to skip entries like node_modules or .git. Output is truncated to ${DEFAULT_LIMIT} entries or ${DEFAULT_MAX_BYTES / 1024}KB (whichever is hit first).`,
		promptSnippet: "List directory contents",
		parameters: lsSchema,
		async execute(
			_toolCallId,
			{ path, limit, ignore }: { path?: string; limit?: number; ignore?: string[] },
			signal?: AbortSignal,
			_onUpdate?,
			_ctx?,
		) {
			return new Promise((resolve, reject) => {
				if (signal?.aborted) {
					reject(new Error("Operation aborted"));
					return;
				}

				const onAbort = () => reject(new Error("Operation aborted"));
				signal?.addEventListener("abort", onAbort, { once: true });

				(async () => {
					try {
						const dirPath = resolveToCwd(path || ".", cwd);
						const effectiveLimit = limit ?? DEFAULT_LIMIT;

						// Check if path exists.
						if (!(await ops.exists(dirPath))) {
							reject(new Error(`Path not found: ${dirPath}`));
							return;
						}

						// Check if path is a directory.
						const stat = await ops.stat(dirPath);
						if (!stat.isDirectory()) {
							reject(new Error(`Not a directory: ${dirPath}`));
							return;
						}

						// Exclude entries matching any ignore glob. Match on the bare entry
						// name (ls is non-recursive) with dot:true so patterns can target
						// dotfiles like '.git'. Default behavior (no patterns) still shows
						// everything on disk.
						const ignorePatterns = (ignore ?? []).filter((p) => typeof p === "string" && p.length > 0);

						// Fast path: use readdirEntries (single syscall with type info).
						// Slow path: fall back to readdir + per-entry stat.
						const results: string[] = [];
						let entryLimitReached = false;

						if (ops.readdirEntries) {
							let dirents: Array<{ name: string; isDirectory: () => boolean }>;
							try {
								dirents = await ops.readdirEntries(dirPath);
							} catch {
								reject(new Error(`Cannot read directory: ${dirPath}`));
								return;
							}
							let filtered = dirents;
							if (ignorePatterns.length > 0) {
								filtered = dirents.filter(
									(d) => !ignorePatterns.some((pattern) => minimatch(d.name, pattern, { dot: true })),
								);
							}
							filtered.sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
							for (const entry of filtered) {
								if (results.length >= effectiveLimit) {
									entryLimitReached = true;
									break;
								}
								results.push(entry.name + (entry.isDirectory() ? "/" : ""));
							}
						} else {
							// Fallback: readdir returns strings, stat each entry.
							let entries: string[];
							try {
								entries = await ops.readdir(dirPath);
							} catch (e) {
								if (e instanceof Error) {
									reject(new Error(`Cannot read directory: ${e.message}`));
								} else {
									reject(new Error(`Cannot read directory: ${String(e)}`));
								}
								return;
							}
							const ignoreFiltered =
								ignorePatterns.length > 0
									? entries.filter(
											(entry) => !ignorePatterns.some((pattern) => minimatch(entry, pattern, { dot: true })),
										)
									: entries;
							ignoreFiltered.sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase()));
							for (const entry of ignoreFiltered) {
								if (results.length >= effectiveLimit) {
									entryLimitReached = true;
									break;
								}
								const fullPath = nodePath.join(dirPath, entry);
								let suffix = "";
								try {
									const entryStat = await ops.stat(fullPath);
									if (entryStat.isDirectory()) suffix = "/";
								} catch {
									continue;
								}
								results.push(entry + suffix);
							}
						}

						signal?.removeEventListener("abort", onAbort);

						if (results.length === 0) {
							resolve({ content: [{ type: "text", text: "(empty directory)" }], details: undefined });
							return;
						}

						const rawOutput = results.join("\n");
						// Apply byte truncation. There is no separate line limit because entry count is already capped.
						const truncation = truncateHead(rawOutput, { maxLines: Number.MAX_SAFE_INTEGER });
						let output = truncation.content;
						const details: LsToolDetails = {};
						// Build actionable notices for truncation and entry limits.
						const notices: string[] = [];
						if (entryLimitReached) {
							notices.push(`${effectiveLimit} entries limit reached. Use limit=${effectiveLimit * 2} for more`);
							details.entryLimitReached = effectiveLimit;
						}
						if (truncation.truncated) {
							notices.push(`${formatSize(DEFAULT_MAX_BYTES)} limit reached`);
							details.truncation = truncation;
						}
						if (notices.length > 0) {
							output += `\n\n[${notices.join(". ")}]`;
						}

						resolve({
							content: [{ type: "text", text: output }],
							details: Object.keys(details).length > 0 ? details : undefined,
						});
					} catch (e) {
						signal?.removeEventListener("abort", onAbort);
						reject(e);
					}
				})();
			});
		},
		renderCall(args, theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatLsCall(args, theme));
			return text;
		},
		renderResult(result, options, theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatLsResult(result as any, options, theme, context.showImages));
			return text;
		},
	};
}

export function createLsTool(cwd: string, options?: LsToolOptions): AgentTool<typeof lsSchema> {
	return wrapToolDefinition(createLsToolDefinition(cwd, options));
}
