/**
 * Headless default tool bundle: the same built-in tools the hoocode CLI
 * registers (bash/read/edit/write), implemented without any CLI or TUI
 * dependency so they can run in a separate process (for example a hooteams
 * worker). The CLI keeps its own richer implementations with
 * interactive rendering; these share the tool names and parameter contracts.
 *
 * No singletons, no top-level side effects: every call to getDefaultTools()
 * builds a fresh bundle bound to the given cwd.
 */

import { resolve } from "node:path";
import { type Static, Type } from "typebox";
import { NodeExecutionEnv } from "../harness/env/nodejs.js";
import {
	DEFAULT_MAX_BYTES,
	DEFAULT_MAX_LINES,
	formatSize,
	truncateHead,
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
// bundle
// ---------------------------------------------------------------------------

/**
 * Build the default headless tool bundle (bash/read/edit/write) bound to the
 * given working directory.
 *
 * The CLI's Task tool is intentionally not part of this bundle: it requires
 * the CLI's subagent runtime (agent registry, subagent pool, session
 * services), which does not exist in a standalone process.
 */
export function getDefaultTools(opts?: DefaultToolsOptions): AgentTool<any>[] {
	const cwd = resolve(opts?.cwd ?? process.cwd());
	const env = new NodeExecutionEnv({ cwd });
	return [createBashTool(env), createReadTool(env), createEditTool(env), createWriteTool(env)];
}
