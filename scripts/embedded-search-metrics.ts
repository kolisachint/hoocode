#!/usr/bin/env npx tsx

/**
 * Embedded-search metrics.
 *
 * For every tracked file in the repo (or an explicit set of paths), report:
 *   - lines       number of newline-delimited lines
 *   - chars       number of characters (UTF-16 code units, i.e. String.length)
 *   - tokens      estimated tokens *if loaded* into context
 *
 * Token estimate uses the same chars/4 heuristic the agent runtime uses to
 * budget context (see estimateTokens in packages/agent/src/harness/compaction
 * and packages/ai/src/providers/faux.ts). It intentionally overestimates.
 *
 * The point is to decide whether a file — or the whole corpus — is a good fit
 * for embedded search: small, text-heavy files index and retrieve cheaply;
 * files whose token cost dwarfs a retrieval budget are better chunked or
 * excluded.
 */

import { execFileSync } from "node:child_process";
import { readFileSync, statSync } from "node:fs";
import { relative, resolve } from "node:path";

const CHARS_PER_TOKEN = 4;

// A single retrieved chunk that comfortably fits a typical embed/RAG budget.
// Files above this are flagged as needing chunking before embedded search.
const GOOD_TOKEN_LIMIT = 2_000;
// Above this a file dominates a retrieval window on its own.
const LARGE_TOKEN_LIMIT = 8_000;

interface FileMetrics {
	path: string;
	lines: number;
	chars: number;
	tokens: number;
	binary: boolean;
}

interface Args {
	root: string;
	paths: string[];
	top: number;
	json: boolean;
	all: boolean;
}

function parseArgs(): Args {
	const argv = process.argv.slice(2);
	const args: Args = { root: process.cwd(), paths: [], top: 25, json: false, all: false };

	for (let i = 0; i < argv.length; i++) {
		const arg = argv[i];
		if (arg === "--top" && argv[i + 1]) {
			const value = Number.parseInt(argv[++i], 10);
			if (!Number.isInteger(value) || value < 0) throw new Error("--top must be a non-negative integer");
			args.top = value;
		} else if ((arg === "--root" || arg === "-C") && argv[i + 1]) {
			args.root = resolve(argv[++i]);
		} else if (arg === "--json") {
			args.json = true;
		} else if (arg === "--all") {
			args.all = true;
		} else if (arg === "--help" || arg === "-h") {
			console.log(`Usage: scripts/embedded-search-metrics.ts [options] [paths...]

Reports lines, characters, and estimated tokens-if-loaded for each file, so you
can judge whether the corpus is a good fit for embedded search.

Options:
  -C, --root <path>   Repository root to scan (default: current cwd)
  --top <n>           Show the n largest files by tokens (default: 25, 0 = all)
  --all               List every file, not just the top n
  --json              Emit JSON instead of a text report
  -h, --help          Show this help

With no [paths...], scans all git-tracked files under --root.
With [paths...], scans exactly those files instead.`);
			process.exit(0);
		} else if (arg.startsWith("-")) {
			throw new Error(`Unknown argument: ${arg}`);
		} else {
			args.paths.push(arg);
		}
	}

	return args;
}

function listTrackedFiles(root: string): string[] {
	const out = execFileSync("git", ["-C", root, "ls-files", "-z"], { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
	return out.split("\0").filter((line) => line.length > 0);
}

function isBinary(buffer: Buffer): boolean {
	// Heuristic: a NUL byte in the first 8 KiB marks the file as binary.
	const limit = Math.min(buffer.length, 8 * 1024);
	for (let i = 0; i < limit; i++) {
		if (buffer[i] === 0) return true;
	}
	return false;
}

function measure(absPath: string, displayPath: string): FileMetrics {
	const buffer = readFileSync(absPath);
	if (isBinary(buffer)) {
		return { path: displayPath, lines: 0, chars: 0, tokens: 0, binary: true };
	}
	const text = buffer.toString("utf8");
	const chars = text.length;
	// Count lines the way an editor does: a trailing newline does not add an
	// empty final line, and an empty file is 0 lines.
	let lines = 0;
	if (chars > 0) {
		lines = 1;
		for (let i = 0; i < chars; i++) {
			if (text.charCodeAt(i) === 10) lines++;
		}
		if (text.charCodeAt(chars - 1) === 10) lines--;
	}
	return { path: displayPath, lines, chars, tokens: Math.ceil(chars / CHARS_PER_TOKEN), binary: false };
}

function formatInt(value: number): string {
	return value.toLocaleString("en-US");
}

function bucketOf(tokens: number): "good" | "large" | "huge" {
	if (tokens <= GOOD_TOKEN_LIMIT) return "good";
	if (tokens <= LARGE_TOKEN_LIMIT) return "large";
	return "huge";
}

const args = parseArgs();

const targets: Array<{ abs: string; display: string }> = [];
if (args.paths.length > 0) {
	for (const p of args.paths) {
		const abs = resolve(args.root, p);
		targets.push({ abs, display: relative(args.root, abs) || p });
	}
} else {
	for (const rel of listTrackedFiles(args.root)) {
		targets.push({ abs: resolve(args.root, rel), display: rel });
	}
}

const metrics: FileMetrics[] = [];
const skipped: string[] = [];
for (const { abs, display } of targets) {
	try {
		if (!statSync(abs).isFile()) continue;
		metrics.push(measure(abs, display));
	} catch (error) {
		skipped.push(`${display}: ${error instanceof Error ? error.message : String(error)}`);
	}
}

const textFiles = metrics.filter((m) => !m.binary);
const binaryFiles = metrics.filter((m) => m.binary);
textFiles.sort((a, b) => b.tokens - a.tokens);

const totalLines = textFiles.reduce((sum, m) => sum + m.lines, 0);
const totalChars = textFiles.reduce((sum, m) => sum + m.chars, 0);
const totalTokens = textFiles.reduce((sum, m) => sum + m.tokens, 0);
const buckets = { good: 0, large: 0, huge: 0 };
for (const m of textFiles) buckets[bucketOf(m.tokens)]++;

if (args.json) {
	console.log(
		JSON.stringify(
			{
				root: args.root,
				charsPerToken: CHARS_PER_TOKEN,
				thresholds: { goodTokenLimit: GOOD_TOKEN_LIMIT, largeTokenLimit: LARGE_TOKEN_LIMIT },
				totals: { files: textFiles.length, lines: totalLines, chars: totalChars, tokens: totalTokens },
				buckets,
				binaryFilesSkipped: binaryFiles.map((m) => m.path),
				files: (args.all || args.top === 0 ? textFiles : textFiles.slice(0, args.top)).map((m) => ({
					path: m.path,
					lines: m.lines,
					chars: m.chars,
					tokens: m.tokens,
					bucket: bucketOf(m.tokens),
				})),
			},
			null,
			2,
		),
	);
	process.exit(0);
}

const shown = args.all || args.top === 0 ? textFiles : textFiles.slice(0, args.top);
const pathWidth = Math.min(72, Math.max(4, ...shown.map((m) => m.path.length)));

console.log(`Embedded-search metrics for ${args.root}`);
console.log(`Token estimate: chars / ${CHARS_PER_TOKEN} (overestimate, matches agent context budgeting)`);
console.log("".padEnd(pathWidth + 40, "="));
console.log(`${"file".padEnd(pathWidth)}  ${"lines".padStart(8)}  ${"chars".padStart(10)}  ${"tokens".padStart(10)}  bucket`);
console.log("".padEnd(pathWidth + 40, "-"));
for (const m of shown) {
	console.log(
		`${m.path.length > pathWidth ? `…${m.path.slice(-(pathWidth - 1))}` : m.path.padEnd(pathWidth)}  ` +
			`${formatInt(m.lines).padStart(8)}  ${formatInt(m.chars).padStart(10)}  ${formatInt(m.tokens).padStart(10)}  ${bucketOf(m.tokens)}`,
	);
}
if (!args.all && args.top !== 0 && textFiles.length > shown.length) {
	console.log(`… ${formatInt(textFiles.length - shown.length)} more files (use --all to list every file)`);
}

console.log("".padEnd(pathWidth + 40, "-"));
console.log(
	`${"TOTAL".padEnd(pathWidth)}  ${formatInt(totalLines).padStart(8)}  ${formatInt(totalChars).padStart(10)}  ${formatInt(totalTokens).padStart(10)}  ${formatInt(textFiles.length)} files`,
);

console.log("\nSuitability for embedded search");
console.log(
	`  ≤ ${formatInt(GOOD_TOKEN_LIMIT)} tokens (index/retrieve as-is): ${formatInt(buckets.good)} files ` +
		`(${textFiles.length ? ((buckets.good / textFiles.length) * 100).toFixed(1) : "0.0"}%)`,
);
console.log(
	`  ${formatInt(GOOD_TOKEN_LIMIT)}–${formatInt(LARGE_TOKEN_LIMIT)} tokens (chunk before embedding): ${formatInt(buckets.large)} files`,
);
console.log(`  > ${formatInt(LARGE_TOKEN_LIMIT)} tokens (large; chunk or exclude):  ${formatInt(buckets.huge)} files`);
console.log(`  whole corpus if loaded at once: ~${formatInt(totalTokens)} tokens`);

if (binaryFiles.length > 0) {
	console.log(`\nSkipped ${formatInt(binaryFiles.length)} binary file(s) (not embeddable as text).`);
}
if (skipped.length > 0) {
	console.log(`\nUnreadable (${skipped.length}):`);
	for (const note of skipped) console.log(`  ${note}`);
}
