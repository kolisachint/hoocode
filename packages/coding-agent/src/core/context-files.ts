/**
 * Context-file and prompt-input loading for the resource loader.
 *
 * Reads AGENTS.md / CLAUDE.md context files from the user scopes and the cwd
 * ancestor chain (warning/truncating oversized ones, since they are injected
 * into the system prompt every turn), and resolves a system-prompt input that
 * may be either an inline string or a file path. Extracted from resource-loader.ts.
 *
 * Two user scopes are read, least specific first: `~/.agents/AGENTS.md` (the
 * cross-vendor convention, so rules written for one tool are seen by hoocode)
 * and `~/.hoocode/AGENTS.md` (the native home, which therefore wins on
 * conflict). Both are additive — neither shadows the other, and no migration
 * is needed for users who already have the native file.
 */

import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import chalk from "chalk";
import { getUserAgentsDir } from "../config.js";

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

// The per-file limits above cap one file; they say nothing about the total,
// and the set can now stack two user scopes on top of a whole ancestor chain.
// So the aggregate gets its own budget, deliberately warn-first: past the soft
// cap (~6k tokens) the user is told what it costs, and only past the hard cap
// (~16k tokens) is anything trimmed. Trimming should be the branch that never
// runs in practice.
const CONTEXT_TOTAL_WARN_BYTES = 24 * 1024;
const CONTEXT_TOTAL_MAX_BYTES = 64 * 1024;
// Below this, a trimmed file has no room left to say anything useful, so it is
// replaced by the notice alone rather than a few words of severed prose.
const CONTEXT_TRIM_MIN_BYTES = 512;

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

export interface LoadProjectContextFilesOptions {
	cwd: string;
	/** hoocode's native home, e.g. `~/.hoocode`. */
	agentDir: string;
	/**
	 * The cross-vendor user scope, e.g. `~/.agents`. Defaults to `~/.agents`;
	 * injectable so tests do not read the real home directory.
	 */
	userAgentsDir?: string;
}

export function loadProjectContextFiles(options: LoadProjectContextFilesOptions): {
	agentsFiles: ContextFile[];
	warnings: string[];
} {
	const resolvedCwd = options.cwd;
	const resolvedAgentDir = options.agentDir;
	const resolvedUserAgentsDir = options.userAgentsDir ?? getUserAgentsDir();

	const contextFiles: ContextFile[] = [];
	const warnings: string[] = [];
	const seenPaths = new Set<string>();

	// Least specific first: the cross-vendor scope, then the native home. A file
	// already seen is never added twice, so pointing both at one directory is a
	// no-op rather than a doubled prompt.
	for (const dir of [resolvedUserAgentsDir, resolvedAgentDir]) {
		const result = loadContextFileFromDir(dir);
		if (result.file && !seenPaths.has(result.file.path)) {
			contextFiles.push(result.file);
			seenPaths.add(result.file.path);
		}
		warnings.push(...result.warnings);
	}

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

	warnings.push(...enforceTotalBudget(contextFiles));

	return { agentsFiles: contextFiles, warnings };
}

/**
 * Apply the aggregate budget to an already-ordered context-file list.
 *
 * Mutates entries in place and returns any warnings. The list is ordered least
 * specific first, and specificity is what decides who pays: the walk runs from
 * the end so the file nearest the work keeps its budget, and a user-scope file
 * is trimmed before a repo one. Files are trimmed rather than dropped, so a
 * file that stops fitting still says so in the prompt instead of vanishing.
 */
function enforceTotalBudget(files: ContextFile[]): string[] {
	const warnings: string[] = [];
	const total = files.reduce((sum, file) => sum + Buffer.byteLength(file.content, "utf-8"), 0);
	if (total <= CONTEXT_TOTAL_WARN_BYTES) {
		return warnings;
	}

	if (total <= CONTEXT_TOTAL_MAX_BYTES) {
		// A single oversized file is already priced by the per-file rule, which
		// flags it `large` and annotates it where it is listed. Repeating that as
		// an aggregate warning says nothing new. The aggregate exists to catch
		// what per-file checks structurally cannot see: the sum across scopes.
		if (files.length < 2) return warnings;
		warnings.push(
			`Context files total ~${Math.round(total / 4 / 100) / 10}k tokens across ${files.length} file(s), ` +
				`re-sent on every request. Keep rules to one line each, and move long or conditional guidance into a skill ` +
				`(loaded on demand) rather than a context file (loaded always).`,
		);
		return warnings;
	}

	let remaining = CONTEXT_TOTAL_MAX_BYTES;
	for (let i = files.length - 1; i >= 0; i--) {
		const file = files[i]!;
		const bytes = Buffer.byteLength(file.content, "utf-8");
		if (bytes <= remaining) {
			remaining -= bytes;
			continue;
		}

		const notice =
			`\n\n[trimmed: context files exceeded ${CONTEXT_TOTAL_MAX_BYTES} bytes (~16k tokens) in total; ` +
			`least-specific scopes are trimmed first — move long guidance into a skill]`;
		file.content =
			remaining >= CONTEXT_TRIM_MIN_BYTES ? file.content.slice(0, remaining) + notice : notice.trimStart();
		file.size = "truncated";
		file.tokens = Math.round(Buffer.byteLength(file.content, "utf-8") / 4);
		remaining = 0;
		warnings.push(`Trimmed ${file.path} — context files exceeded the total budget.`);
	}

	return warnings;
}
