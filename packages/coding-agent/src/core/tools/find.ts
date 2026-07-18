import { createInterface } from "node:readline";
import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type ChildProcess, spawn } from "child_process";
import { existsSync } from "fs";
import path from "path";
import { type Static, Type } from "typebox";
import { keyHint } from "../../modes/interactive/components/keybinding-hints.js";
import { ensureTool } from "../../utils/tools-manager.js";
import type { ToolDefinition, ToolRenderResultOptions } from "../extensions/types.js";
import { applyFdGlobPattern, relativizeFdLine, toPosixPath } from "./fd-utils.js";
import { isNativeSearchForced, nativeFind } from "./native-search.js";
import { resolveToCwd } from "./path-utils.js";
import { getTextOutput, invalidArgText, shortenPath, str } from "./render-utils.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";
import { DEFAULT_MAX_BYTES, formatSize, type TruncationResult, truncateHead } from "./truncate.js";

const findSchema = Type.Object({
	pattern: Type.Union([Type.String(), Type.Array(Type.String())], {
		description:
			"Glob pattern(s) to match files. Pass one pattern or an array for OR logic, e.g. '*.ts', 'src/**/*.spec.ts', or ['src/**/*.ts', 'test/**/*.ts'].",
	}),
	path: Type.Optional(Type.String({ description: "Directory to search in (default: current directory)" })),
	exclude: Type.Optional(
		Type.Union([Type.String(), Type.Array(Type.String())], {
			description: "Additional exclusion glob(s), e.g. '**/*.test.ts' or ['**/dist/**', '**/build/**'].",
		}),
	),
	type: Type.Optional(
		Type.Union([Type.Literal("f"), Type.Literal("d"), Type.Literal("l")], {
			description: "Filter by entry type: 'f' files, 'd' directories, 'l' symlinks (default: 'f').",
		}),
	),
	depth: Type.Optional(Type.Number({ description: "Maximum directory depth to search." })),
	limit: Type.Optional(Type.Number({ description: "Maximum number of results (default: 1000)." })),
	compress: Type.Optional(
		Type.Boolean({ description: "Group files in the same directory to shorten output (default: false)." }),
	),
});

export type FindToolInput = Static<typeof findSchema>;

const DEFAULT_LIMIT = 1000;
const NO_RESULTS_MESSAGE = "No files found matching pattern";

export interface FindToolDetails {
	truncation?: TruncationResult;
	resultLimitReached?: number;
}

/**
 * Pluggable operations for the find tool.
 * Override these to delegate file search to remote systems (for example SSH).
 */
export interface FindOperations {
	/** Check if path exists */
	exists: (absolutePath: string) => Promise<boolean> | boolean;
	/** Find files matching one or more glob patterns. Returns relative or absolute paths. */
	glob: (
		patterns: string[],
		cwd: string,
		options: { ignore: string[]; limit: number; type?: string },
	) => Promise<string[]> | string[];
}

const defaultFindOperations: FindOperations = {
	exists: existsSync,
	// This is a placeholder. Actual fd execution happens in execute() when no custom glob is provided.
	glob: () => [],
};

export interface FindToolOptions {
	/** Custom operations for find. Default: local filesystem plus fd */
	operations?: FindOperations;
}

/**
 * Compress a list of file paths by grouping files in the same directory.
 *
 * Compression rules:
 * - If a directory has >=3 entries all sharing the same extension: fold into `dir/{stem1,stem2,stem3}.ext`
 * - If >=3 entries with mixed extensions: sub-group by extension; fold sub-groups >=3
 * - If directory has >6 total files: summarize verbatim after sub-grouping
 * - 1-2 entries: emit verbatim
 */
function compressPaths(paths: string[]): string {
	if (paths.length === 0) return "";

	const sorted = [...paths].sort();

	const dirMap = new Map<string, string[]>();
	for (const p of sorted) {
		const dir = path.dirname(p);
		const base = path.basename(p);
		if (!dirMap.has(dir)) dirMap.set(dir, []);
		dirMap.get(dir)!.push(base);
	}

	const result: string[] = [];

	for (const [dir, files] of dirMap) {
		if (files.length <= 2) {
			for (const f of files) result.push(dir === "." ? f : `${dir}/${f}`);
			continue;
		}

		const extMap = new Map<string, string[]>();
		for (const f of files) {
			const ext = path.extname(f);
			const stem = path.basename(f, ext);
			if (!extMap.has(ext)) extMap.set(ext, []);
			extMap.get(ext)!.push(stem);
		}

		if (extMap.size === 1 && files.length >= 3) {
			const [ext, stems] = extMap.entries().next().value!;
			const displayDir = dir === "." ? "" : `${dir}/`;
			result.push(`${displayDir}{${stems.join(",")}}${ext}`);
			continue;
		}

		let anyFolded = false;
		const subResults: string[] = [];
		for (const [ext, stems] of extMap) {
			if (stems.length >= 3) {
				const displayDir = dir === "." ? "" : `${dir}/`;
				subResults.push(`${displayDir}{${stems.join(",")}}${ext}`);
				anyFolded = true;
			} else {
				for (const stem of stems) subResults.push(dir === "." ? `${stem}${ext}` : `${dir}/${stem}${ext}`);
			}
		}

		if (anyFolded || files.length > 6) {
			result.push(...subResults);
		} else {
			for (const f of files) result.push(dir === "." ? f : `${dir}/${f}`);
		}
	}

	return result.join("\n");
}

function formatFindCall(
	args: { pattern?: string | string[]; path?: string; type?: string; depth?: number; limit?: number } | undefined,
	theme: typeof import("../../modes/interactive/theme/theme.js").theme,
): string {
	const patterns = args?.pattern;
	const patternStr = Array.isArray(patterns) ? patterns.join(", ") : str(patterns);
	const rawPath = str(args?.path);
	const displayPath = rawPath !== null ? shortenPath(rawPath || ".") : null;
	const type = args?.type;
	const depth = args?.depth;
	const limit = args?.limit;
	const invalidArg = invalidArgText(theme);
	let text =
		theme.fg("toolTitle", theme.bold("find")) +
		" " +
		(patternStr === null || patternStr === "" ? invalidArg : theme.fg("accent", patternStr)) +
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

function formatFindResult(
	result: {
		content: Array<{ type: string; text?: string; data?: string; mimeType?: string }>;
		details?: FindToolDetails;
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

	const resultLimit = result.details?.resultLimitReached;
	const truncation = result.details?.truncation;
	if (resultLimit || truncation?.truncated) {
		const warnings: string[] = [];
		if (resultLimit) warnings.push(`${resultLimit} results limit`);
		if (truncation?.truncated) warnings.push(`${formatSize(truncation.maxBytes ?? DEFAULT_MAX_BYTES)} limit`);
		text += `\n${theme.fg("warning", `[Truncated: ${warnings.join(", ")}]`)}`;
	}
	return text;
}

export function createFindToolDefinition(
	cwd: string,
	options?: FindToolOptions,
): ToolDefinition<typeof findSchema, FindToolDetails | undefined> {
	const customOps = options?.operations;
	return {
		name: "find",
		label: "find",
		description: `Search for files by one or more glob patterns (OR logic across an array). Optionally filter by entry type (files/dirs/symlinks), directory depth, and extra exclusions. Returns matching paths relative to the search directory. Respects .gitignore. Output is truncated to ${DEFAULT_LIMIT} results or ${DEFAULT_MAX_BYTES / 1024}KB (whichever is hit first).`,
		promptSnippet: "Find files by glob pattern (supports multiple patterns, respects .gitignore)",
		parameters: findSchema,
		async execute(
			_toolCallId,
			{
				pattern: rawPattern,
				path: searchDir,
				exclude: rawExclude,
				type,
				depth,
				limit,
				compress = false,
			}: {
				pattern: string | string[];
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
				let stopChildren: (() => void) | undefined;
				const settle = (fn: () => void) => {
					if (settled) return;
					settled = true;
					signal?.removeEventListener("abort", onAbort);
					stopChildren = undefined;
					fn();
				};
				const onAbort = () => {
					stopChildren?.();
					settle(() => reject(new Error("Operation aborted")));
				};
				signal?.addEventListener("abort", onAbort, { once: true });

				(async () => {
					try {
						const searchPath = resolveToCwd(searchDir || ".", cwd);
						const effectiveLimit = Math.max(1, limit ?? DEFAULT_LIMIT);
						const ops = customOps ?? defaultFindOperations;

						const patterns = Array.isArray(rawPattern) ? rawPattern : [rawPattern];
						const excludePatterns = rawExclude ? (Array.isArray(rawExclude) ? rawExclude : [rawExclude]) : [];
						// Always exclude node_modules and .git, plus any caller exclusions.
						const allIgnore = ["**/node_modules/**", "**/.git/**", ...excludePatterns];
						const typeFilter = type === "d" ? "d" : type === "l" ? "l" : "f";

						const emit = (relativized: string[]) => {
							if (relativized.length === 0) {
								settle(() =>
									resolve({ content: [{ type: "text", text: NO_RESULTS_MESSAGE }], details: undefined }),
								);
								return;
							}
							// Sources over-fetch by one so exactly-limit result sets are not
							// misreported as truncated.
							const unique = [...new Set(relativized)].sort();
							const resultLimitReached = unique.length > effectiveLimit;
							const truncated = unique.slice(0, effectiveLimit);
							const rawOutput = compress ? compressPaths(truncated) : truncated.join("\n");
							const truncation = truncateHead(rawOutput, { maxLines: Number.MAX_SAFE_INTEGER });
							let resultOutput = truncation.content;
							const details: FindToolDetails = {};
							const notices: string[] = [];
							if (resultLimitReached) {
								notices.push(
									`${effectiveLimit} result${effectiveLimit === 1 ? "" : "s"} limit reached. Use limit=${effectiveLimit * 2} for more, or refine pattern`,
								);
								details.resultLimitReached = effectiveLimit;
							}
							if (truncation.truncated) {
								notices.push(`${formatSize(DEFAULT_MAX_BYTES)} limit reached`);
								details.truncation = truncation;
							}
							if (notices.length > 0) resultOutput += `\n\n[${notices.join(". ")}]`;
							settle(() =>
								resolve({
									content: [{ type: "text", text: resultOutput }],
									details: Object.keys(details).length > 0 ? details : undefined,
								}),
							);
						};

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
							const results = await ops.glob(patterns, searchPath, {
								ignore: allIgnore,
								limit: effectiveLimit + 1,
								type: typeFilter,
							});
							if (signal?.aborted) {
								settle(() => reject(new Error("Operation aborted")));
								return;
							}
							emit(
								results.map((p) => {
									if (p.startsWith(searchPath)) return toPosixPath(p.slice(searchPath.length + 1));
									return toPosixPath(path.relative(searchPath, p));
								}),
							);
							return;
						}

						// Default implementation uses fd, with a pure-JS fallback when fd is
						// unavailable (restricted environments) or explicitly forced.
						const fdPath = isNativeSearchForced() ? undefined : await ensureTool("fd", true);
						if (signal?.aborted) {
							settle(() => reject(new Error("Operation aborted")));
							return;
						}
						if (!fdPath) {
							if (!(await ops.exists(searchPath))) {
								settle(() => reject(new Error(`Path not found: ${searchPath}`)));
								return;
							}
							const nativeResults = nativeFind(searchPath, {
								patterns,
								type: typeFilter,
								excludeGlobs: allIgnore,
								maxDepth: depth,
								// find always excludes node_modules/.git; the exclude globs above
								// cover them too, but skipping the dirs avoids descending them.
								alwaysSkipDirs: new Set([".git", "node_modules"]),
								signal,
							});
							if (signal?.aborted) {
								settle(() => reject(new Error("Operation aborted")));
								return;
							}
							emit(nativeResults);
							return;
						}

						const children: ChildProcess[] = [];
						stopChildren = () => {
							for (const child of children) if (!child.killed) child.kill();
						};

						// Run each pattern with fd. fd errors (e.g. an invalid glob) surface as a
						// rejection when the pattern produced no output, matching the shell's
						// behavior; a non-zero exit that still produced matches is tolerated.
						const runPattern = (pattern: string): Promise<string[]> =>
							new Promise<string[]>((res, rej) => {
								const args: string[] = [
									"--glob",
									"--color=never",
									"--hidden",
									"--no-require-git",
									"--type",
									typeFilter,
									"--max-results",
									String(effectiveLimit + 1),
								];
								for (const excl of allIgnore) args.push("--exclude", excl);
								if (depth !== undefined) args.push("--max-depth", String(depth));
								const effectivePattern = applyFdGlobPattern(args, pattern);
								args.push("--", effectivePattern, searchPath);

								const child = spawn(fdPath, args, { stdio: ["ignore", "pipe", "pipe"] });
								children.push(child);
								const rl = createInterface({ input: child.stdout! });
								let stderr = "";
								const lines: string[] = [];
								child.stderr?.on("data", (chunk) => {
									stderr += chunk.toString();
								});
								rl.on("line", (line) => lines.push(line));
								child.on("error", (error) => {
									rl.close();
									rej(new Error(`Failed to run fd: ${error.message}`));
								});
								child.on("close", (code) => {
									rl.close();
									if (code !== 0 && lines.length === 0) {
										rej(new Error(stderr.trim() || `fd exited with code ${code}`));
										return;
									}
									res(lines);
								});
							});

						const patternResults = await Promise.all(patterns.map(runPattern));
						if (signal?.aborted) {
							settle(() => reject(new Error("Operation aborted")));
							return;
						}

						const relativized: string[] = [];
						for (const results of patternResults) {
							for (const rawLine of results) {
								const line = rawLine.replace(/\r$/, "").trim();
								if (!line) continue;
								relativized.push(relativizeFdLine(line, searchPath));
							}
						}
						emit(relativized);
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
			text.setText(formatFindCall(args, theme));
			return text;
		},
		renderResult(result, options, theme, context) {
			const text = (context.lastComponent as Text | undefined) ?? new Text("", 0, 0);
			text.setText(formatFindResult(result as any, options, theme, context.showImages));
			return text;
		},
	};
}

export function createFindTool(cwd: string, options?: FindToolOptions): AgentTool<typeof findSchema> {
	return wrapToolDefinition(createFindToolDefinition(cwd, options));
}
