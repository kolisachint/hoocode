/**
 * Headless default tool bundle: the same built-in tools the hoocode CLI
 * registers (bash/read/edit/write/grep/find/ls), implemented without any
 * CLI or TUI dependency so they can run in a separate process (for example
 * a hooteams worker). The CLI keeps its own richer implementations with
 * interactive rendering; these share the tool names and parameter contracts.
 *
 * No singletons, no top-level side effects: every call to getDefaultTools()
 * builds a fresh bundle bound to the given cwd.
 */

import { readFile } from "node:fs/promises";
import { isAbsolute, join, relative, resolve } from "node:path";
import ignore from "ignore";
import { type Static, Type } from "typebox";
import { NodeExecutionEnv } from "../harness/env/nodejs.js";
import { FileError } from "../harness/types.js";
import {
	DEFAULT_MAX_BYTES,
	DEFAULT_MAX_LINES,
	formatSize,
	truncateHead,
	truncateLine,
	truncateTail,
} from "../harness/utils/truncate.js";
import type { AgentTool, AgentToolResult } from "../types.js";

export interface DefaultToolsOptions {
	/** Working directory the tools operate in. Defaults to process.cwd(). */
	cwd?: string;
}

function textResult(text: string): AgentToolResult<undefined> {
	return { content: [{ type: "text", text }], details: undefined };
}

function resolveToCwd(cwd: string, path: string): string {
	return isAbsolute(path) ? path : resolve(cwd, path);
}

// ---------------------------------------------------------------------------
// bash
// ---------------------------------------------------------------------------

const bashSchema = Type.Object({
	command: Type.String({ description: "Bash command to execute" }),
	timeout: Type.Optional(Type.Number({ description: "Timeout in seconds (optional, no default timeout)" })),
});

function createBashTool(env: NodeExecutionEnv): AgentTool<typeof bashSchema> {
	return {
		name: "bash",
		label: "bash",
		description: `Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last ${DEFAULT_MAX_LINES} lines or ${DEFAULT_MAX_BYTES / 1024}KB (whichever is hit first). Optionally provide a timeout in seconds.`,
		parameters: bashSchema,
		execute: async (_toolCallId, params: Static<typeof bashSchema>, signal) => {
			let combined = "";
			let exitCode: number;
			try {
				const result = await env.exec(params.command, {
					timeout: params.timeout,
					signal,
					onStdout: (chunk) => {
						combined += chunk;
					},
					onStderr: (chunk) => {
						combined += chunk;
					},
				});
				exitCode = result.exitCode;
			} catch (error) {
				const message = error instanceof Error ? error.message : String(error);
				if (message.startsWith("timeout:")) {
					throw new Error(
						`Command timed out after ${params.timeout}s${combined ? `\nOutput so far:\n${combined}` : ""}`,
					);
				}
				throw error;
			}
			const truncation = truncateTail(combined);
			let text = truncation.content;
			if (truncation.truncated) {
				text = `[Output truncated: showing last ${truncation.outputLines} of ${truncation.totalLines} lines (${formatSize(truncation.totalBytes)})]\n${text}`;
			}
			if (exitCode !== 0) {
				text = text.length > 0 ? `${text}\nExit code: ${exitCode}` : `Exit code: ${exitCode}`;
			}
			return textResult(text.length > 0 ? text : "(no output)");
		},
	};
}

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

const readSchema = Type.Object({
	path: Type.String({ description: "Path to the file to read (relative or absolute)" }),
	offset: Type.Optional(Type.Number({ description: "Line number to start reading from (1-indexed)" })),
	limit: Type.Optional(Type.Number({ description: "Maximum number of lines to read" })),
});

function createReadTool(env: NodeExecutionEnv): AgentTool<typeof readSchema> {
	return {
		name: "read",
		label: "read",
		description: `Read the contents of a text file. Output is truncated to ${DEFAULT_MAX_LINES} lines or ${DEFAULT_MAX_BYTES / 1024}KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.`,
		parameters: readSchema,
		execute: async (_toolCallId, params: Static<typeof readSchema>) => {
			const content = await env.readTextFile(params.path);
			let lines = content.split("\n");
			const totalLines = lines.length;
			const offset = params.offset !== undefined ? Math.max(1, Math.floor(params.offset)) : 1;
			if (offset > totalLines) {
				throw new Error(`Offset ${offset} is past the end of the file (${totalLines} lines)`);
			}
			lines = lines.slice(offset - 1);
			if (params.limit !== undefined) {
				lines = lines.slice(0, Math.max(0, Math.floor(params.limit)));
			}
			const truncation = truncateHead(lines.join("\n"));
			let text = truncation.content;
			if (truncation.truncated) {
				const lastShown = offset - 1 + truncation.outputLines;
				text = `${text}\n[Truncated: showing lines ${offset}-${lastShown} of ${totalLines}. Continue with offset=${lastShown + 1}]`;
			}
			return textResult(text);
		},
	};
}

// ---------------------------------------------------------------------------
// edit
// ---------------------------------------------------------------------------

const replaceEditSchema = Type.Object(
	{
		oldText: Type.String({
			description:
				"Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call.",
		}),
		newText: Type.String({ description: "Replacement text for this targeted edit." }),
	},
	{ additionalProperties: false },
);

const editSchema = Type.Object(
	{
		path: Type.String({ description: "Path to the file to edit (relative or absolute)" }),
		edits: Type.Array(replaceEditSchema, {
			description:
				"One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits.",
		}),
	},
	{ additionalProperties: false },
);

function countOccurrences(haystack: string, needle: string): number {
	if (needle.length === 0) return 0;
	let count = 0;
	let index = haystack.indexOf(needle);
	while (index !== -1) {
		count++;
		index = haystack.indexOf(needle, index + needle.length);
	}
	return count;
}

function createEditTool(env: NodeExecutionEnv): AgentTool<typeof editSchema> {
	return {
		name: "edit",
		label: "edit",
		description:
			"Edit a file by replacing exact text. Each edit's oldText must appear exactly once in the file. Provide multiple edits to make several targeted replacements in one call.",
		parameters: editSchema,
		execute: async (_toolCallId, params: Static<typeof editSchema>) => {
			const original = await env.readTextFile(params.path);
			if (params.edits.length === 0) {
				throw new Error("No edits provided");
			}
			let content = original;
			for (const [index, edit] of params.edits.entries()) {
				const occurrences = countOccurrences(original, edit.oldText);
				if (occurrences === 0) {
					throw new Error(`edits[${index}].oldText not found in ${params.path}`);
				}
				if (occurrences > 1) {
					throw new Error(
						`edits[${index}].oldText matches ${occurrences} locations in ${params.path}; add surrounding context to make it unique`,
					);
				}
				if (!content.includes(edit.oldText)) {
					throw new Error(`edits[${index}].oldText overlaps with an earlier edit in the same call`);
				}
				content = content.replace(edit.oldText, edit.newText);
			}
			await env.writeFile(params.path, content);
			return textResult(
				`Applied ${params.edits.length} edit${params.edits.length === 1 ? "" : "s"} to ${params.path}`,
			);
		},
	};
}

// ---------------------------------------------------------------------------
// write
// ---------------------------------------------------------------------------

const writeSchema = Type.Object({
	path: Type.String({ description: "Path to the file to write (relative or absolute)" }),
	content: Type.String({ description: "Content to write to the file" }),
});

function createWriteTool(env: NodeExecutionEnv): AgentTool<typeof writeSchema> {
	return {
		name: "write",
		label: "write",
		description: "Write content to a file, creating parent directories as needed. Overwrites existing files.",
		parameters: writeSchema,
		execute: async (_toolCallId, params: Static<typeof writeSchema>) => {
			await env.writeFile(params.path, params.content);
			return textResult(`Wrote ${formatSize(Buffer.byteLength(params.content, "utf-8"))} to ${params.path}`);
		},
	};
}

// ---------------------------------------------------------------------------
// shared file walking for grep/find
// ---------------------------------------------------------------------------

/** Convert a glob pattern to a regular expression over `/`-separated paths. */
export function globToRegExp(pattern: string): RegExp {
	let regex = "";
	for (let i = 0; i < pattern.length; i++) {
		const char = pattern[i];
		if (char === "*") {
			if (pattern[i + 1] === "*") {
				// `**/` and `**` cross directory boundaries
				if (pattern[i + 2] === "/") {
					regex += "(?:[^/]+/)*";
					i += 2;
				} else {
					regex += ".*";
					i += 1;
				}
			} else {
				regex += "[^/]*";
			}
		} else if (char === "?") {
			regex += "[^/]";
		} else if ("\\^$.|+()[]{}".includes(char)) {
			regex += `\\${char}`;
		} else {
			regex += char;
		}
	}
	return new RegExp(`^${regex}$`);
}

/** Match a relative path against a glob; patterns without `/` match the basename at any depth. */
function matchGlob(relPath: string, pattern: string): boolean {
	const normalized = relPath.split("\\").join("/");
	if (!pattern.includes("/")) {
		const base = normalized.split("/").pop() ?? normalized;
		return globToRegExp(pattern).test(base);
	}
	return globToRegExp(pattern).test(normalized);
}

const ALWAYS_IGNORED = new Set([".git", "node_modules"]);

/**
 * Walk files under root depth-first, honoring the root .gitignore (nested
 * .gitignore files are not consulted) and always skipping .git/node_modules.
 * Yields paths relative to root with `/` separators.
 */
async function collectFiles(env: NodeExecutionEnv, root: string, limit: number): Promise<string[]> {
	const ig = ignore();
	try {
		ig.add(await readFile(join(root, ".gitignore"), "utf-8"));
	} catch {
		// no .gitignore at the search root
	}
	const results: string[] = [];
	const stack: string[] = [""];
	while (stack.length > 0 && results.length < limit) {
		const dir = stack.pop()!;
		let entries: Awaited<ReturnType<NodeExecutionEnv["listDir"]>>;
		try {
			entries = await env.listDir(dir === "" ? root : join(root, dir));
		} catch {
			continue;
		}
		entries.sort((a, b) => a.name.localeCompare(b.name));
		for (const entry of entries) {
			if (ALWAYS_IGNORED.has(entry.name)) continue;
			const relPath = dir === "" ? entry.name : `${dir}/${entry.name}`;
			if (entry.kind === "directory") {
				if (ig.ignores(`${relPath}/`)) continue;
				stack.push(relPath);
			} else if (entry.kind === "file") {
				if (ig.ignores(relPath)) continue;
				results.push(relPath);
				if (results.length >= limit) break;
			}
		}
	}
	return results;
}

// ---------------------------------------------------------------------------
// grep
// ---------------------------------------------------------------------------

const grepSchema = Type.Object({
	pattern: Type.String({ description: "Search pattern (regex or literal string)" }),
	path: Type.Optional(Type.String({ description: "Directory or file to search (default: current directory)" })),
	glob: Type.Optional(Type.String({ description: "Filter files by glob pattern, e.g. '*.ts' or '**/*.spec.ts'" })),
	ignoreCase: Type.Optional(Type.Boolean({ description: "Case-insensitive search (default: false)" })),
	literal: Type.Optional(
		Type.Boolean({ description: "Treat pattern as literal string instead of regex (default: false)" }),
	),
	limit: Type.Optional(Type.Number({ description: "Maximum number of matches to return (default: 100)" })),
});

const GREP_DEFAULT_LIMIT = 100;
const GREP_FILE_SCAN_LIMIT = 50_000;

function escapeRegExp(text: string): string {
	return text.replace(/[\\^$.|?*+()[\]{}]/g, "\\$&");
}

function looksBinary(content: string): boolean {
	return content.includes("\0");
}

function createGrepTool(env: NodeExecutionEnv, cwd: string): AgentTool<typeof grepSchema> {
	return {
		name: "grep",
		label: "grep",
		description: `Search file contents for a pattern. Returns matching lines with file paths and line numbers. Respects the root .gitignore. Output is truncated to ${GREP_DEFAULT_LIMIT} matches by default.`,
		parameters: grepSchema,
		execute: async (_toolCallId, params: Static<typeof grepSchema>, signal) => {
			const source = params.literal ? escapeRegExp(params.pattern) : params.pattern;
			const regex = new RegExp(source, params.ignoreCase ? "i" : "");
			const limit = params.limit ?? GREP_DEFAULT_LIMIT;
			const searchRoot = resolveToCwd(cwd, params.path ?? ".");

			const rootInfo = await env.fileInfo(searchRoot);
			const files =
				rootInfo.kind === "file"
					? [relative(cwd, searchRoot).split("\\").join("/") || rootInfo.name]
					: await collectFiles(env, searchRoot, GREP_FILE_SCAN_LIMIT);

			const matches: string[] = [];
			let limitReached = false;
			outer: for (const file of files) {
				if (signal?.aborted) throw new Error("aborted");
				if (params.glob && !matchGlob(file, params.glob)) continue;
				const absolute = rootInfo.kind === "file" ? searchRoot : join(searchRoot, file);
				let content: string;
				try {
					content = await env.readTextFile(absolute);
				} catch {
					continue;
				}
				if (looksBinary(content)) continue;
				const displayPath = rootInfo.kind === "file" ? file : relative(cwd, absolute).split("\\").join("/");
				const lines = content.split("\n");
				for (let i = 0; i < lines.length; i++) {
					if (!regex.test(lines[i])) continue;
					matches.push(`${displayPath}:${i + 1}: ${truncateLine(lines[i]).text}`);
					if (matches.length >= limit) {
						limitReached = true;
						break outer;
					}
				}
			}

			if (matches.length === 0) {
				return textResult("No matches found");
			}
			const truncation = truncateHead(matches.join("\n"));
			let text = truncation.content;
			if (limitReached) {
				text = `${text}\n[Match limit of ${limit} reached; refine the pattern or raise limit]`;
			} else if (truncation.truncated) {
				text = `${text}\n[Output truncated at ${formatSize(truncation.maxBytes)}]`;
			}
			return textResult(text);
		},
	};
}

// ---------------------------------------------------------------------------
// find
// ---------------------------------------------------------------------------

const findSchema = Type.Object({
	pattern: Type.String({
		description: "Glob pattern to match files, e.g. '*.ts', '**/*.json', or 'src/**/*.spec.ts'",
	}),
	path: Type.Optional(Type.String({ description: "Directory to search in (default: current directory)" })),
	limit: Type.Optional(Type.Number({ description: "Maximum number of results (default: 1000)" })),
});

const FIND_DEFAULT_LIMIT = 1000;

function createFindTool(env: NodeExecutionEnv, cwd: string): AgentTool<typeof findSchema> {
	return {
		name: "find",
		label: "find",
		description: `Search for files by glob pattern. Returns matching file paths relative to the search directory. Respects the root .gitignore. Output is truncated to ${FIND_DEFAULT_LIMIT} results by default.`,
		parameters: findSchema,
		execute: async (_toolCallId, params: Static<typeof findSchema>) => {
			const limit = params.limit ?? FIND_DEFAULT_LIMIT;
			const searchRoot = resolveToCwd(cwd, params.path ?? ".");
			const files = await collectFiles(env, searchRoot, GREP_FILE_SCAN_LIMIT);
			const matched: string[] = [];
			let limitReached = false;
			for (const file of files) {
				if (!matchGlob(file, params.pattern)) continue;
				matched.push(file);
				if (matched.length >= limit) {
					limitReached = true;
					break;
				}
			}
			if (matched.length === 0) {
				return textResult("No files found");
			}
			let text = matched.join("\n");
			if (limitReached) {
				text = `${text}\n[Result limit of ${limit} reached]`;
			}
			return textResult(text);
		},
	};
}

// ---------------------------------------------------------------------------
// ls
// ---------------------------------------------------------------------------

const lsSchema = Type.Object({
	path: Type.Optional(Type.String({ description: "Directory to list (default: current directory)" })),
	limit: Type.Optional(Type.Number({ description: "Maximum number of entries to return (default: 500)" })),
});

const LS_DEFAULT_LIMIT = 500;

function createLsTool(env: NodeExecutionEnv, cwd: string): AgentTool<typeof lsSchema> {
	return {
		name: "ls",
		label: "ls",
		description: `List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. Output is truncated to ${LS_DEFAULT_LIMIT} entries by default.`,
		parameters: lsSchema,
		execute: async (_toolCallId, params: Static<typeof lsSchema>) => {
			const target = resolveToCwd(cwd, params.path ?? ".");
			const limit = params.limit ?? LS_DEFAULT_LIMIT;
			const info = await env.fileInfo(target);
			if (info.kind !== "directory") {
				throw new FileError("not_directory", `Not a directory: ${target}`, target);
			}
			const entries = await env.listDir(target);
			entries.sort((a, b) => a.name.localeCompare(b.name));
			const shown = entries.slice(0, limit);
			let text = shown.map((entry) => (entry.kind === "directory" ? `${entry.name}/` : entry.name)).join("\n");
			if (entries.length > limit) {
				text = `${text}\n[Entry limit of ${limit} reached; ${entries.length - limit} more entries]`;
			}
			return textResult(text.length > 0 ? text : "(empty directory)");
		},
	};
}

// ---------------------------------------------------------------------------
// bundle
// ---------------------------------------------------------------------------

/**
 * Build the default headless tool bundle (bash/read/edit/write/grep/find/ls)
 * bound to the given working directory.
 *
 * The CLI's Task tool is intentionally not part of this bundle: it requires
 * the CLI's subagent runtime (agent registry, subagent pool, session
 * services), which does not exist in a standalone process.
 */
export function getDefaultTools(opts?: DefaultToolsOptions): AgentTool<any>[] {
	const cwd = resolve(opts?.cwd ?? process.cwd());
	const env = new NodeExecutionEnv({ cwd });
	return [
		createBashTool(env),
		createReadTool(env),
		createEditTool(env),
		createWriteTool(env),
		createGrepTool(env, cwd),
		createFindTool(env, cwd),
		createLsTool(env, cwd),
	];
}
