import { createInterface } from "node:readline";
import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { spawn } from "child_process";
import { existsSync } from "fs";
import path from "path";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import { ensureTool } from "../../utils/tools-manager.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import { applyFdGlobPattern, relativizeFdLine, toPosixPath } from "./fd-utils.js";
import { resolveToCwd } from "./path-utils.js";
import { getTextOutput, invalidArgText, shortenPath, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";
import { DEFAULT_MAX_BYTES, formatSize, type TruncationResult, truncateHead } from "./truncate.js";

const globSchema = Type.Object({
	patterns: Type.Union([Type.String(), Type.Array(Type.String())], {
		description:
			"One or more glob patterns to match files (OR logic), e.g. '*.ts', '**/*.json', or ['src/**/*.ts', 'test/**/*.ts']",
	}),
	path: Type.Optional(Type.String({ description: "Root directory to search from (default: current directory)" })),
	exclude: Type.Optional(
		Type.Union([Type.String(), Type.Array(Type.String())], {
			description: "Additional exclusion patterns, e.g. '**/*.test.ts' or ['**/dist/**', '**/build/**']",
		}),
	),
	type: Type.Optional(
		Type.Union([Type.Literal("f"), Type.Literal("d"), Type.Literal("l")], {
			description: "Filter by file type: 'f' for files, 'd' for directories, 'l' for symlinks (default: 'f')",
		}),
	),
	depth: Type.Optional(Type.Number({ description: "Maximum directory depth to search" })),
	limit: Type.Optional(Type.Number({ description: "Maximum number of results (default: 2000)" })),
	compress: Type.Optional(Type.Boolean({ description: "Compress output by grouping files (default: true)" })),
});

export type GlobToolInput = Static<typeof globSchema>;

const DEFAULT_LIMIT = 2000;

export interface GlobToolDetails {
	truncation?: TruncationResult;
	resultLimitReached?: number;
	totalMatches?: number;
	uniqueDirectories?: number;
}

/**
 * Pluggable operations for the glob tool.
 * Override these to delegate file search to remote systems (for example SSH).
 */
export interface GlobOperations {
	/** Check if path exists */
	exists: (absolutePath: string) => Promise<boolean> | boolean;
	/** Optional custom glob resolver (used for SSH/remote) */
	glob?: (
		patterns: string[],
		cwd: string,
		options: { ignore: string[]; limit: number; type?: string },
	) => Promise<string[]> | string[];
}

const defaultGlobOperations: GlobOperations = {
	exists: existsSync,
};

export interface GlobToolOptions {
	/** Custom operations for glob. Default: local filesystem plus fd */
	operations?: GlobOperations;
}

/**
 * Compress a list of file paths by grouping files in the same directory.
 *
 * Compression rules:
 * - If a directory has >=3 entries all sharing the same extension: fold into `dir/{stem1,stem2,stem3}.ext`
 * - If >=3 entries with mixed extensions: sub-group by extension; fold sub-groups >=3
 * - If directory has >6 total files: summarize as `dir/ [N files]`
 * - 1-2 entries: emit verbatim
 */
function compressPaths(paths: string[]): string {
	if (paths.length === 0) return "";

	// Sort paths for stable output
	const sorted = [...paths].sort();

	// Group by directory
	const dirMap = new Map<string, string[]>();
	for (const p of sorted) {
		const dir = path.dirname(p);
		const base = path.basename(p);
		if (!dirMap.has(dir)) {
			dirMap.set(dir, []);
		}
		dirMap.get(dir)!.push(base);
	}

	const result: string[] = [];

	for (const [dir, files] of dirMap) {
		if (files.length <= 2) {
			// 1-2 entries: emit verbatim
			for (const f of files) {
				result.push(dir === "." ? f : `${dir}/${f}`);
			}
			continue;
		}

		// Check if all files share the same extension
		const extMap = new Map<string, string[]>();
		for (const f of files) {
			const ext = path.extname(f);
			const stem = path.basename(f, ext);
			if (!extMap.has(ext)) {
				extMap.set(ext, []);
			}
			extMap.get(ext)!.push(stem);
		}

		// If all files have the same extension and >=3, fold
		if (extMap.size === 1 && files.length >= 3) {
			const [ext, stems] = extMap.entries().next().value!;
			const displayDir = dir === "." ? "" : `${dir}/`;
			result.push(`${displayDir}{${stems.join(",")}}${ext}`);
			continue;
		}

		// Mixed extensions: try to sub-group
		let anyFolded = false;
		const subResults: string[] = [];

		for (const [ext, stems] of extMap) {
			if (stems.length >= 3) {
				// Fold this sub-group
				const displayDir = dir === "." ? "" : `${dir}/`;
				subResults.push(`${displayDir}{${stems.join(",")}}${ext}`);
				anyFolded = true;
			} else {
				// Emit verbatim
				for (const stem of stems) {
					subResults.push(dir === "." ? `${stem}${ext}` : `${dir}/${stem}${ext}`);
				}
			}
		}

		if (anyFolded || files.length > 6) {
			result.push(...subResults);
		} else {
			// Too mixed, emit verbatim
			for (const f of files) {
				result.push(dir === "." ? f : `${dir}/${f}`);
			}
		}
	}

	return result.join("\n");
}

function formatGlobCall(
	args: { patterns?: string | string[]; path?: string; type?: string; depth?: number; limit?: number } | undefined,
	theme: typeof import("../../modes/interactive/theme/theme.js").theme,
): string {
	const patterns = args?.patterns;
	const patternStr = Array.isArray(patterns) ? patterns.join(", ") : (str(patterns) ?? "");
	const rawPath = str(args?.path);
	const displayPath = rawPath !== null ? shortenPath(rawPath || ".") : null;
	const type = args?.type;
	const depth = args?.depth;
	const limit = args?.limit;
	const invalidArg = invalidArgText(theme);

	let text =
		theme.fg("toolTitle", theme.bold("glob")) +
		" " +
		(patternStr === "" ? invalidArg : theme.fg("accent", patternStr)) +
		theme.fg("toolOutput", ` in ${displayPath === null ? invalidArg : displayPath}`);

	if (type && type !== "f") {
		const typeLabel = type === "d" ? "dirs" : type === "l" ? "symlinks" : type;
		text += theme.fg("toolOutput", ` (${typeLabel})`);
	}
	if (depth !== undefined) {
		text += theme.fg("toolOutput", ` (depth ${depth})`);
	}
	if (limit !== undefined) {
		text += theme.fg("toolOutput", ` (limit ${limit})`);
	}
	return text;
}

function formatGlobResult(
	result: {
		content: Array<{ type: string; text?: string; data?: string; mimeType?: string }>;
		details?: GlobToolDetails;
	},
	options: ToolRenderResultOptions,
	theme: typeof import("../../modes/interactive/theme/theme.js").theme,
	showImages: boolean,
): string {
	const output = getTextOutput(result, showImages).trim();
	let text = "";
	if (output) {
		const lines = output.split("\n");
		const maxLines = options.expanded ? lines.length : 25;
		const displayLines = lines.slice(0, maxLines);
		const remaining = lines.length - maxLines;
		text += `\n${displayLines.map((line) => theme.fg("toolOutput", line)).join("\n")}`;
		if (remaining > 0) {
			text += `${theme.fg("muted", `\n... (${remaining} more lines,`)} ${keyHint("app.tools.expand", "to expand")})`;
		}
	}

	// Stats footer
	const totalMatches = result.details?.totalMatches;
	const uniqueDirectories = result.details?.uniqueDirectories;
	if (totalMatches !== undefined && uniqueDirectories !== undefined) {
		text += `\n${theme.fg("muted", `[${totalMatches} matches, ${uniqueDirectories} dirs]`)}`;
	}

	// Truncation/limit warnings
	const resultLimit = result.details?.resultLimitReached;
	const truncation = result.details?.truncation;
	if (resultLimit || truncation?.truncated) {
		const warnings: string[] = [];
		if (resultLimit) {
			warnings.push(`${resultLimit} results limit reached. Use limit=${resultLimit * 2} for more`);
		}
		if (truncation?.truncated) {
			warnings.push(`${formatSize(DEFAULT_MAX_BYTES)} limit reached`);
		}
		text += `\n${theme.fg("warning", `[Truncated: ${warnings.join(". ")}]`)}`;
	}
	return text;
}

export function createGlobToolDefinition(
	cwd: string,
	options?: GlobToolOptions,
): ToolDefinition<typeof globSchema, GlobToolDetails | undefined> {
	const customOps = options?.operations;
	return {
		name: "glob",
		label: "glob",
		description: `Search for files matching one or more glob patterns. Supports OR logic across patterns. Returns matching file paths grouped and compressed for readability. Respects .gitignore. Output is truncated to ${DEFAULT_LIMIT} results or ${DEFAULT_MAX_BYTES / 1024}KB (whichever is hit first).`,
		promptSnippet: "Search for files by glob pattern (supports multiple patterns, respects .gitignore)",
		parameters: globSchema,
		async execute(
			_toolCallId,
			{
				patterns: rawPatterns,
				path: searchDir,
				exclude: rawExclude,
				type,
				depth,
				limit,
				compress = true,
			}: {
				patterns: string | string[];
				path?: string;
				exclude?: string | string[];
				type?: "f" | "d" | "l";
				depth?: number;
				limit?: number;
				compress?: boolean;
			},
			signal?: AbortSignal,
			_onUpdate?,
			_ctx?,
		) {
			return new Promise((resolve, reject) => {
				if (signal?.aborted) {
					reject(new Error("Operation aborted"));
					return;
				}

				let settled = false;
				let stopChild: (() => void) | undefined;
				const settle = (fn: () => void) => {
					if (settled) return;
					settled = true;
					signal?.removeEventListener("abort", onAbort);
					stopChild = undefined;
					fn();
				};
				const onAbort = () => {
					stopChild?.();
					settle(() => reject(new Error("Operation aborted")));
				};
				signal?.addEventListener("abort", onAbort, { once: true });

				(async () => {
					try {
						const searchPath = resolveToCwd(searchDir || ".", cwd);
						const effectiveLimit = limit ?? DEFAULT_LIMIT;
						const ops = customOps ?? defaultGlobOperations;

						// Normalize patterns to array
						const patterns = Array.isArray(rawPatterns) ? rawPatterns : [rawPatterns];

						// Normalize exclude to array
						const excludePatterns = rawExclude ? (Array.isArray(rawExclude) ? rawExclude : [rawExclude]) : [];

						// Always exclude node_modules and .git
						const allIgnore = ["**/node_modules/**", "**/.git/**", ...excludePatterns];

						// Build type filter for fd
						const typeFilter = type === "d" ? "d" : type === "l" ? "l" : "f";

						// If custom operations provide glob(), use that instead of fd.
						if (customOps?.glob) {
							if (!(await ops.exists(searchPath))) {
								settle(() => reject(new Error(`Path not found: ${searchPath}`)));
								return;
							}
							if (signal?.aborted) {
								settle(() => reject(new Error("Operation aborted")));
								return;
							}
							const results = await ops.glob!(patterns, searchPath, {
								ignore: allIgnore,
								limit: effectiveLimit,
								type: typeFilter,
							});
							if (signal?.aborted) {
								settle(() => reject(new Error("Operation aborted")));
								return;
							}
							if (results.length === 0) {
								settle(() =>
									resolve({
										content: [{ type: "text", text: "No files found matching patterns" }],
										details: undefined,
									}),
								);
								return;
							}

							// Relativize paths against the search root
							const relativized = results.map((p) => {
								if (p.startsWith(searchPath)) return toPosixPath(p.slice(searchPath.length + 1));
								return toPosixPath(path.relative(searchPath, p));
							});

							// Deduplicate
							const unique = [...new Set(relativized)];

							const totalMatches = unique.length;
							const resultLimitReached = totalMatches >= effectiveLimit;
							const truncated = unique.slice(0, effectiveLimit);

							// Compress output
							let resultOutput: string;
							if (compress) {
								resultOutput = compressPaths(truncated);
							} else {
								resultOutput = truncated.join("\n");
							}

							// Apply byte-level truncation
							const truncationResult = truncateHead(resultOutput, {
								maxLines: Number.MAX_SAFE_INTEGER,
							});
							resultOutput = truncationResult.content;

							// Calculate unique directories
							const uniqueDirs = new Set(truncated.map((p) => path.dirname(p)));

							const details: GlobToolDetails = {
								totalMatches,
								uniqueDirectories: uniqueDirs.size,
							};
							const notices: string[] = [];
							if (resultLimitReached) {
								notices.push(
									`${effectiveLimit} results limit reached. Use limit=${effectiveLimit * 2} for more`,
								);
								details.resultLimitReached = effectiveLimit;
							}
							if (truncationResult.truncated) {
								notices.push(`${formatSize(DEFAULT_MAX_BYTES)} limit reached`);
								details.truncation = truncationResult;
							}
							if (notices.length > 0) {
								resultOutput += `\n\n[${notices.join(". ")}]`;
							}

							settle(() =>
								resolve({
									content: [{ type: "text", text: resultOutput }],
									details: Object.keys(details).length > 0 ? details : undefined,
								}),
							);
							return;
						}

						// Default implementation uses fd.
						const fdPath = await ensureTool("fd", true);
						if (signal?.aborted) {
							settle(() => reject(new Error("Operation aborted")));
							return;
						}
						if (!fdPath) {
							settle(() =>
								reject(
									new Error(
										"fd unavailable and could not be downloaded — use the bash tool to run find instead",
									),
								),
							);
							return;
						}

						// Run all patterns in parallel
						const allResults: string[] = [];

						const runPattern = async (pattern: string): Promise<string[]> => {
							const args: string[] = [
								"--glob",
								"--color=never",
								"--hidden",
								"--no-require-git",
								"--max-results",
								String(effectiveLimit),
							];

							// Add type filter
							if (typeFilter) {
								args.push("--type", typeFilter);
							}

							// Add exclusions
							for (const excl of allIgnore) {
								args.push("--exclude", excl);
							}

							// Add depth
							if (depth !== undefined) {
								args.push("--max-depth", String(depth));
							}

							const effectivePattern = applyFdGlobPattern(args, pattern);
							args.push("--", effectivePattern, searchPath);

							return new Promise<string[]>((res, rej) => {
								const child = spawn(fdPath, args, { stdio: ["ignore", "pipe", "pipe"] });
								const rl = createInterface({ input: child.stdout });
								const lines: string[] = [];

								const cleanup = () => {
									rl.close();
								};

								rl.on("line", (line) => {
									lines.push(line);
								});

								child.on("error", (error) => {
									cleanup();
									rej(new Error(`Failed to run fd: ${error.message}`));
								});

								child.on("close", () => {
									cleanup();
									res(lines);
								});
							});
						};

						// Execute all patterns in parallel
						const patternResults = await Promise.all(patterns.map(runPattern));

						// Merge and deduplicate results
						const seen = new Set<string>();
						for (const results of patternResults) {
							for (const rawLine of results) {
								const line = rawLine.replace(/\r$/, "").trim();
								if (!line) continue;

								const posixPath = relativizeFdLine(line, searchPath);
								if (!seen.has(posixPath)) {
									seen.add(posixPath);
									allResults.push(posixPath);
								}
							}
						}

						if (allResults.length === 0) {
							settle(() =>
								resolve({
									content: [{ type: "text", text: "No files found matching patterns" }],
									details: undefined,
								}),
							);
							return;
						}

						// Sort results for stable output
						allResults.sort();

						const totalMatches = allResults.length;
						const resultLimitReached = totalMatches >= effectiveLimit;
						const truncated = allResults.slice(0, effectiveLimit);

						// Compress output
						let resultOutput: string;
						if (compress) {
							resultOutput = compressPaths(truncated);
						} else {
							resultOutput = truncated.join("\n");
						}

						// Apply byte-level truncation
						const truncationResult = truncateHead(resultOutput, {
							maxLines: Number.MAX_SAFE_INTEGER,
						});
						resultOutput = truncationResult.content;

						// Calculate unique directories
						const uniqueDirs = new Set(truncated.map((p) => path.dirname(p)));

						const details: GlobToolDetails = {
							totalMatches,
							uniqueDirectories: uniqueDirs.size,
						};
						const notices: string[] = [];
						if (resultLimitReached) {
							notices.push(`${effectiveLimit} results limit reached. Use limit=${effectiveLimit * 2} for more`);
							details.resultLimitReached = effectiveLimit;
						}
						if (truncationResult.truncated) {
							notices.push(`${formatSize(DEFAULT_MAX_BYTES)} limit reached`);
							details.truncation = truncationResult;
						}
						if (notices.length > 0) {
							resultOutput += `\n\n[${notices.join(". ")}]`;
						}

						settle(() =>
							resolve({
								content: [{ type: "text", text: resultOutput }],
								details: Object.keys(details).length > 0 ? details : undefined,
							}),
						);
					} catch (e) {
						if (signal?.aborted) {
							settle(() => reject(new Error("Operation aborted")));
							return;
						}
						const error = e instanceof Error ? e : new Error(String(e));
						settle(() => reject(error));
					}
				})();
			});
		},
		renderCall(args, theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatGlobCall(args, theme));
			return text;
		},
		renderResult(result, options, theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatGlobResult(result as any, options, theme, context.showImages));
			return text;
		},
	};
}

export function createGlobTool(cwd: string, options?: GlobToolOptions): AgentTool<typeof globSchema> {
	return wrapToolDefinition(createGlobToolDefinition(cwd, options));
}
