/**
 * Context-file and prompt-input loading for the resource loader.
 *
 * Reads AGENTS.md / CLAUDE.md context files from the agent dir and the cwd
 * ancestor chain (warning/truncating oversized ones, since they are injected
 * into the system prompt every turn), and resolves a system-prompt input that
 * may be either an inline string or a file path. Extracted from resource-loader.ts.
 */

import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import chalk from "chalk";

/**
 * A loaded context file plus the recurring-cost metadata the UI surfaces.
 * `tokens`/`size` are optional so callers that synthesize context files (SDK
 * overrides, tests) can keep passing plain `{ path, content }`.
 */
export interface ContextFile {
	path: string;
	content: string;
	/** Rough token estimate (bytes / 4) of the content as loaded. */
	tokens?: number;
	/** Set only past the soft limit; "truncated" means the hard limit clipped it. */
	size?: "large" | "truncated";
}

/**
 * Resolve a prompt input that is either an inline string or a path to a file.
 * If the input names an existing file, its contents are returned; otherwise the
 * input is treated as the prompt text itself.
 */
export function resolvePromptInput(input: string | undefined, description: string): string | undefined {
	if (!input) {
		return undefined;
	}

	if (existsSync(input)) {
		try {
			return readFileSync(input, "utf-8");
		} catch (error) {
			console.error(chalk.yellow(`Warning: Could not read ${description} file ${input}: ${error}`));
			return input;
		}
	}

	return input;
}

// Context files (AGENTS.md / CLAUDE.md) are injected into the system prompt on
// every turn, so their size has a recurring cost on every provider. Warn the
// user past a soft limit (~2k tokens) and truncate at a hard limit (~10k tokens)
// so a pasted spec can't silently bloat every request forever.
const CONTEXT_FILE_WARN_BYTES = 8 * 1024;
const CONTEXT_FILE_MAX_BYTES = 40 * 1024;

function loadContextFileFromDir(dir: string): { file: ContextFile | null; warnings: string[] } {
	const warnings: string[] = [];
	const candidates = ["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"];
	for (const filename of candidates) {
		const filePath = join(dir, filename);
		if (existsSync(filePath)) {
			try {
				let content = readFileSync(filePath, "utf-8");
				const bytes = Buffer.byteLength(content, "utf-8");
				// Size is reported structurally (not as a warning string) so the UI can
				// annotate the file where it is already listed instead of repeating it.
				let size: ContextFile["size"];
				if (bytes > CONTEXT_FILE_MAX_BYTES) {
					content =
						content.slice(0, CONTEXT_FILE_MAX_BYTES) +
						`\n\n[truncated: file exceeded ${CONTEXT_FILE_MAX_BYTES} bytes (~10k tokens); keep context files brief — large specs belong in linked files, not in the system prompt]`;
					size = "truncated";
				} else if (bytes > CONTEXT_FILE_WARN_BYTES) {
					size = "large";
				}
				return { file: { path: filePath, content, tokens: Math.round(bytes / 4), size }, warnings };
			} catch (error) {
				warnings.push(`Could not read ${filePath}: ${error}`);
			}
		}
	}
	return { file: null, warnings };
}

export function loadProjectContextFiles(options: { cwd: string; agentDir: string }): {
	agentsFiles: ContextFile[];
	warnings: string[];
} {
	const resolvedCwd = options.cwd;
	const resolvedAgentDir = options.agentDir;

	const contextFiles: ContextFile[] = [];
	const warnings: string[] = [];
	const seenPaths = new Set<string>();

	const globalResult = loadContextFileFromDir(resolvedAgentDir);
	if (globalResult.file) {
		contextFiles.push(globalResult.file);
		seenPaths.add(globalResult.file.path);
	}
	warnings.push(...globalResult.warnings);

	const ancestorContextFiles: ContextFile[] = [];

	let currentDir = resolvedCwd;
	const root = resolve("/");

	while (true) {
		const result = loadContextFileFromDir(currentDir);
		if (result.file && !seenPaths.has(result.file.path)) {
			ancestorContextFiles.unshift(result.file);
			seenPaths.add(result.file.path);
		}
		warnings.push(...result.warnings);

		if (currentDir === root) break;

		const parentDir = resolve(currentDir, "..");
		if (parentDir === currentDir) break;
		currentDir = parentDir;
	}

	contextFiles.push(...ancestorContextFiles);

	return { agentsFiles: contextFiles, warnings };
}
