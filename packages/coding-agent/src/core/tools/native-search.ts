/**
 * Native (pure-JS) fallbacks for the `find` and `grep` tools.
 *
 * The tools normally shell out to `fd` / `rg`, which are downloaded on demand
 * (see tools-manager.ts). In restricted environments those binaries may be
 * neither on PATH nor downloadable, and `HOOCODE_NATIVE_SEARCH=1` can also force
 * this path. Rather than failing the tool and asking the model to fall back to
 * `bash`, these functions reproduce the essential behaviour in JS so search
 * degrades automatically:
 *
 *   - hierarchical `.gitignore` handling (each `.gitignore` is scoped to its own
 *     subtree, matching fd's `--no-require-git` behaviour and issue #3303),
 *   - hidden files included (like `fd --hidden` / `rg --hidden`),
 *   - `.git` always skipped; `node_modules` skipped for `find` (mirrors the
 *     tool's built-in excludes) but left to `.gitignore` for `grep` (like rg).
 *
 * These are best-effort approximations, not byte-for-byte fd/rg parity: globs
 * are matched with `minimatch` and patterns with JS `RegExp`, and only
 * `.gitignore` files are honoured (not `.ignore` or global excludes).
 */

import { readdirSync, readFileSync, statSync } from "fs";
import ignore, { type Ignore } from "ignore";
import { minimatch } from "minimatch";
import path from "path";
import { toPosixPath } from "./fd-utils.js";

export type EntryType = "f" | "d" | "l";

/** Hard cap on entries enumerated during a single walk, so a pathological tree
 *  can never hang the fallback. Well above any tool's own result limit. */
const MAX_ENTRIES = 200_000;

/** Files larger than this are skipped by the grep fallback (rg streams; we read
 *  whole files, so we guard against loading huge blobs into memory). */
const MAX_GREP_FILE_BYTES = 20 * 1024 * 1024;

export interface CollectedEntry {
	/** Absolute path. */
	abs: string;
	/** POSIX path relative to the walk root. */
	rel: string;
	type: EntryType;
}

/** A `.gitignore` matcher scoped to the subtree rooted at `baseDir`. */
interface GitignoreMatcher {
	baseDir: string;
	ig: Ignore;
}

function loadGitignore(dir: string): Ignore | undefined {
	let content: string;
	try {
		content = readFileSync(path.join(dir, ".gitignore"), "utf-8");
	} catch {
		return undefined;
	}
	return ignore().add(content);
}

/**
 * Whether `absPath` is ignored by any applicable `.gitignore`. Each matcher only
 * applies to paths inside its `baseDir`, and the path is tested relative to that
 * base — so `a/.gitignore` scopes to `a/` and its descendants but never `b/`.
 */
function isGitIgnored(absPath: string, isDir: boolean, matchers: GitignoreMatcher[]): boolean {
	for (const m of matchers) {
		const rel = path.relative(m.baseDir, absPath);
		if (rel === "" || rel.startsWith("..") || path.isAbsolute(rel)) continue;
		const relPosix = toPosixPath(rel) + (isDir ? "/" : "");
		if (m.ig.ignores(relPosix)) return true;
	}
	return false;
}

export interface WalkOptions {
	/** Max entry depth relative to root; direct children are depth 1. */
	maxDepth?: number;
	signal?: AbortSignal;
	/** Directory names to never descend into. Defaults to `.git`. */
	alwaysSkipDirs?: Set<string>;
}

/**
 * Walk `root` depth-first, returning every entry not excluded by `.gitignore`
 * or an always-skip directory. Symlinks are reported but never followed (avoids
 * cycles). Enumeration stops at {@link MAX_ENTRIES}.
 */
export function collectEntries(root: string, opts: WalkOptions = {}): CollectedEntry[] {
	const out: CollectedEntry[] = [];
	const alwaysSkip = opts.alwaysSkipDirs ?? new Set([".git"]);

	const rootMatchers: GitignoreMatcher[] = [];
	const rootIg = loadGitignore(root);
	if (rootIg) rootMatchers.push({ baseDir: root, ig: rootIg });

	// `depth` is the depth of `dir`; its direct children are at depth + 1.
	const stack: Array<{ dir: string; depth: number; matchers: GitignoreMatcher[] }> = [
		{ dir: root, depth: 0, matchers: rootMatchers },
	];

	while (stack.length > 0) {
		if (out.length >= MAX_ENTRIES || opts.signal?.aborted) break;
		const { dir, depth, matchers } = stack.pop()!;

		let dirents: import("fs").Dirent[];
		try {
			dirents = readdirSync(dir, { withFileTypes: true });
		} catch {
			continue; // unreadable directory — skip rather than abort the whole walk
		}

		const entryDepth = depth + 1;
		for (const dirent of dirents) {
			if (out.length >= MAX_ENTRIES) break;

			const name = dirent.name;
			const abs = path.join(dir, name);

			let type: EntryType;
			let isDir = false;
			if (dirent.isSymbolicLink()) {
				type = "l";
			} else if (dirent.isDirectory()) {
				type = "d";
				isDir = true;
			} else if (dirent.isFile()) {
				type = "f";
			} else {
				continue; // sockets, fifos, block devices, …
			}

			if (isDir && alwaysSkip.has(name)) continue;
			if (isGitIgnored(abs, isDir, matchers)) continue;

			if (opts.maxDepth === undefined || entryDepth <= opts.maxDepth) {
				out.push({ abs, rel: toPosixPath(path.relative(root, abs)), type });
			}

			// Descend only into real directories, and only if their children can
			// still be within the depth budget.
			if (isDir && (opts.maxDepth === undefined || entryDepth < opts.maxDepth)) {
				const childIg = loadGitignore(abs);
				const nextMatchers = childIg ? [...matchers, { baseDir: abs, ig: childIg }] : matchers;
				stack.push({ dir: abs, depth: entryDepth, matchers: nextMatchers });
			}
		}
	}

	return out;
}

/**
 * Reject globs fd's parser would reject. minimatch silently treats an unclosed
 * `[` or `{` as literal text, which made the fallback resolve "no results"
 * where the fd path errors — same call, different outcome. Error messages
 * mirror fd/globset so callers (and tests) can match either path.
 */
function validateGlob(pattern: string): void {
	let inClass = false;
	let braceDepth = 0;
	for (let i = 0; i < pattern.length; i++) {
		const c = pattern[i];
		if (c === "\\") {
			i++; // skip escaped char
		} else if (inClass) {
			if (c === "]") inClass = false;
		} else if (c === "[") {
			inClass = true;
		} else if (c === "{") {
			braceDepth++;
		} else if (c === "}") {
			if (braceDepth > 0) braceDepth--;
		}
	}
	if (inClass) {
		throw new Error(`error parsing glob '${pattern}': unclosed character class; missing ']'`);
	}
	if (braceDepth > 0) {
		throw new Error(`error parsing glob '${pattern}': unclosed alternate group; missing '}'`);
	}
}

/** fd smart-case: a pattern without uppercase letters matches case-insensitively. */
function isSmartCaseInsensitive(pattern: string): boolean {
	return !/[A-Z]/.test(pattern);
}

/**
 * Match a `find` glob against a POSIX relative path, mirroring fd's semantics:
 * a slashless pattern matches the basename at any depth; a pattern containing a
 * slash matches the full path and is anchored anywhere in the tree. Case
 * sensitivity is smart-case, like fd.
 */
function matchesFindPattern(relPosix: string, pattern: string): boolean {
	const nocase = isSmartCaseInsensitive(pattern);
	if (pattern.includes("/")) {
		let p = pattern;
		if (p.startsWith("/")) {
			p = p.slice(1); // leading slash anchors to root; rel paths have no leading slash
		} else if (!p.startsWith("**/") && p !== "**") {
			p = `**/${p}`;
		}
		return minimatch(relPosix, p, { dot: true, nocase });
	}
	return minimatch(relPosix, pattern, { dot: true, matchBase: true, nocase });
}

export interface NativeFindOptions {
	patterns: string[];
	type: EntryType;
	/** Extra exclusion globs (already includes node_modules/.git for find). */
	excludeGlobs: string[];
	maxDepth?: number;
	alwaysSkipDirs?: Set<string>;
	signal?: AbortSignal;
}

/**
 * Native replacement for the fd-backed search. Returns POSIX paths relative to
 * `root`, with a trailing slash on directories (like fd), unsorted/undeduped —
 * the caller applies its own dedupe/sort/limit.
 */
export function nativeFind(root: string, opts: NativeFindOptions): string[] {
	for (const pattern of opts.patterns) validateGlob(pattern);
	for (const glob of opts.excludeGlobs) validateGlob(glob);

	const entries = collectEntries(root, {
		maxDepth: opts.maxDepth,
		signal: opts.signal,
		alwaysSkipDirs: opts.alwaysSkipDirs,
	});

	const results: string[] = [];
	for (const entry of entries) {
		if (entry.type !== opts.type) continue;
		if (opts.excludeGlobs.some((g) => minimatch(entry.rel, g, { dot: true }))) continue;
		if (!opts.patterns.some((pat) => matchesFindPattern(entry.rel, pat))) continue;
		results.push(entry.type === "d" ? `${entry.rel}/` : entry.rel);
	}
	return results;
}

export interface NativeGrepMatch {
	filePath: string;
	lineNumber: number;
	lineText: string;
}

export interface NativeGrepOptions {
	pattern: string;
	/** True when `root` is a directory; false when it is a single file. */
	isDirectory: boolean;
	ignoreCase?: boolean;
	literal?: boolean;
	/** Optional glob filter applied to file paths (like rg --glob). */
	glob?: string;
	/** Stop after this many matches. */
	limit: number;
	signal?: AbortSignal;
	readFile: (absolutePath: string) => Promise<string> | string;
}

export interface NativeGrepResult {
	matches: NativeGrepMatch[];
	matchLimitReached: boolean;
}

function escapeRegExp(value: string): string {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Cheap binary-file heuristic: a NUL byte in the first chunk. rg skips these. */
function looksBinary(content: string): boolean {
	const sampleLength = Math.min(content.length, 8192);
	for (let i = 0; i < sampleLength; i++) {
		if (content.charCodeAt(i) === 0) return true;
	}
	return false;
}

/**
 * Native replacement for the rg-backed content search. Collects up to `limit`
 * matches as `{ filePath, lineNumber, lineText }`, the same shape the tool's
 * formatter already consumes from rg's JSON output.
 *
 * Throws an Error tagged `invalidRegex` when a non-literal pattern is not a
 * valid JS regex, so the caller can surface the same "pass literal: true" hint.
 */
export async function nativeGrep(root: string, opts: NativeGrepOptions): Promise<NativeGrepResult> {
	let regex: RegExp;
	const flags = opts.ignoreCase ? "i" : "";
	if (opts.literal) {
		regex = new RegExp(escapeRegExp(opts.pattern), flags);
	} else {
		try {
			regex = new RegExp(opts.pattern, flags);
		} catch (e) {
			const error = new Error(e instanceof Error ? e.message : String(e)) as Error & { invalidRegex?: boolean };
			error.invalidRegex = true;
			throw error;
		}
	}

	let files: string[];
	if (!opts.isDirectory) {
		files = [root];
	} else {
		const entries = collectEntries(root, { signal: opts.signal });
		files = entries
			.filter((e) => e.type === "f")
			.filter((e) => {
				if (!opts.glob) return true;
				return minimatch(e.rel, opts.glob, { dot: true, matchBase: !opts.glob.includes("/") });
			})
			.map((e) => e.abs);
	}

	const matches: NativeGrepMatch[] = [];
	let matchLimitReached = false;

	for (const filePath of files) {
		if (opts.signal?.aborted || matches.length >= opts.limit) break;

		try {
			if (statSync(filePath).size > MAX_GREP_FILE_BYTES) continue;
		} catch {
			continue;
		}

		let content: string;
		try {
			content = await opts.readFile(filePath);
		} catch {
			continue;
		}
		if (looksBinary(content)) continue;

		const lines = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
		for (let i = 0; i < lines.length; i++) {
			// New RegExp per file with no /g flag → test() is stateless across lines.
			if (regex.test(lines[i])) {
				matches.push({ filePath, lineNumber: i + 1, lineText: lines[i] });
				if (matches.length >= opts.limit) {
					matchLimitReached = true;
					break;
				}
			}
		}
	}

	return { matches, matchLimitReached };
}

/** Whether the native search path is forced regardless of fd/rg availability. */
export function isNativeSearchForced(): boolean {
	return process.env.HOOCODE_NATIVE_SEARCH === "1";
}
