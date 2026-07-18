/**
 * Internal lexical retriever for hybrid search.
 *
 * This is not the grep *tool* — it is the lexical recall backend: it turns a
 * natural query into a ripgrep pattern, streams matches, and returns bare
 * `rel:line` hits for the grep→chunk adapter. rg drives the fast path; the
 * pure-JS nativeGrep fallback keeps restricted environments working, same as
 * the grep tool.
 */

import { createInterface } from "node:readline";
import { spawn } from "child_process";
import { readFileSync } from "fs";
import path from "path";
import { ensureTool } from "../../utils/tools-manager.js";
import { isNativeSearchForced, nativeGrep } from "../tools/native-search.js";
import type { GrepLineHit } from "./adapter.js";

/** Terms considered per query (longest first) when building the pattern. */
const MAX_TERMS = 4;
/** Minimum token length worth matching on. */
const MIN_TERM_LENGTH = 3;

function escapeRegExp(value: string): string {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export interface LexicalQueryPlan {
	/** rg-ready regex pattern. */
	pattern: string;
	/** Raw (unescaped, lowercased) terms, for per-line term attribution. */
	terms: string[];
}

/**
 * Build the retrieval plan for a query: a quoted segment is searched
 * verbatim; otherwise the longest few identifier-ish tokens are OR-ed
 * together. Returns undefined when the query yields nothing searchable.
 */
export function buildLexicalQueryPlan(query: string): LexicalQueryPlan | undefined {
	const quoted = [...query.matchAll(/["'`]([^"'`]+)["'`]/g)]
		.map((m) => m[1].trim())
		.filter((s) => s.length > 0)
		.sort((a, b) => b.length - a.length)[0];
	if (quoted) return { pattern: escapeRegExp(quoted), terms: [quoted.toLowerCase()] };

	const tokens = [...new Set(query.match(/[A-Za-z0-9_$][\w$.-]*/g) ?? [])]
		.filter((t) => t.length >= MIN_TERM_LENGTH)
		.sort((a, b) => b.length - a.length || a.localeCompare(b))
		.slice(0, MAX_TERMS);
	if (tokens.length === 0) {
		const trimmed = query.trim();
		return trimmed ? { pattern: escapeRegExp(trimmed), terms: [trimmed.toLowerCase()] } : undefined;
	}
	return { pattern: tokens.map(escapeRegExp).join("|"), terms: tokens.map((t) => t.toLowerCase()) };
}

/** Pattern-only view of {@link buildLexicalQueryPlan}. */
export function buildLexicalPattern(query: string): string | undefined {
	return buildLexicalQueryPlan(query)?.pattern;
}

/** Which plan terms appear on a matched line (retrieval is case-insensitive,
 *  so attribution is too). */
function termsOnLine(plan: LexicalQueryPlan, lineText: string | undefined): string[] {
	if (!lineText) return [];
	const lower = lineText.toLowerCase();
	return plan.terms.filter((t) => lower.includes(t));
}

/**
 * Run lexical retrieval over `cwd`, returning up to `limit` line-hits in
 * output order with POSIX repo-relative paths.
 */
export interface RunLexicalOptions {
	cwd: string;
	query: string;
	limit: number;
	glob?: string;
	signal?: AbortSignal;
}

/**
 * Run the lexical retriever for the search tool. Optional glob filter scopes
 * file paths (slashless matches basename anywhere, slash patterns match the
 * full repo-relative path).
 */
function normalizeSearchGlob(glob: string | undefined): string | undefined {
	if (!glob) return undefined;
	// Match fd/rg semantics: a slash-containing glob is anchored anywhere in
	// the tree, so prepend "**/" unless it already starts with a slash or "**/".
	if (glob.includes("/") && !glob.startsWith("/") && !glob.startsWith("**/")) {
		return `**/${glob}`;
	}
	return glob;
}

export async function runLexicalRetriever(options: RunLexicalOptions): Promise<GrepLineHit[]> {
	const { cwd, query, limit, glob: rawGlob, signal } = options;
	const glob = normalizeSearchGlob(rawGlob);
	const plan = buildLexicalQueryPlan(query);
	if (!plan) return [];
	const { pattern } = plan;

	const toRel = (filePath: string): string => {
		const rel = path.relative(cwd, filePath);
		return (rel && !rel.startsWith("..") ? rel : filePath).replace(/\\/g, "/");
	};

	const rgPath = isNativeSearchForced() ? undefined : await ensureTool("rg", true);
	if (!rgPath) {
		const result = await nativeGrep(cwd, {
			pattern,
			isDirectory: true,
			ignoreCase: true,
			limit,
			glob,
			signal,
			readFile: (p) => readFileSync(p, "utf-8"),
		});
		return result.matches.map((m) => ({
			rel: toRel(m.filePath),
			line: m.lineNumber,
			terms: termsOnLine(plan, m.lineText),
		}));
	}

	return new Promise<GrepLineHit[]>((resolve, reject) => {
		// --sort path forces a deterministic (single-threaded) walk: with the
		// match cap truncating the stream, a parallel walk would return a
		// different hit subset per run — "same query, different context".
		const args = ["--json", "--line-number", "--color=never", "--hidden", "--ignore-case", "--sort", "path"];
		if (glob) args.push("--glob", glob);
		args.push("--", pattern, cwd);
		const child = spawn(rgPath, args, { stdio: ["ignore", "pipe", "pipe"] });
		const rl = createInterface({ input: child.stdout });
		const hits: GrepLineHit[] = [];
		let stderr = "";
		let killedDueToLimit = false;
		let aborted = false;

		const onAbort = () => {
			aborted = true;
			if (!child.killed) child.kill();
		};
		signal?.addEventListener("abort", onAbort, { once: true });
		child.stderr?.on("data", (chunk) => {
			stderr += chunk.toString();
		});

		rl.on("line", (line) => {
			if (!line.trim() || hits.length >= limit) return;
			let event: any;
			try {
				event = JSON.parse(line);
			} catch {
				return;
			}
			if (event.type !== "match") return;
			const filePath = event.data?.path?.text;
			const lineNumber = event.data?.line_number;
			if (filePath && typeof lineNumber === "number") {
				hits.push({ rel: toRel(filePath), line: lineNumber, terms: termsOnLine(plan, event.data?.lines?.text) });
			}
			if (hits.length >= limit && !child.killed) {
				killedDueToLimit = true;
				child.kill();
			}
		});

		child.on("error", (error) => {
			signal?.removeEventListener("abort", onAbort);
			reject(new Error(`Failed to run ripgrep: ${error.message}`));
		});
		child.on("close", (code) => {
			rl.close();
			signal?.removeEventListener("abort", onAbort);
			if (aborted) {
				reject(new Error("Operation aborted"));
				return;
			}
			// rg exits 1 on "no matches" — that is a valid empty result.
			if (!killedDueToLimit && code !== 0 && code !== 1) {
				reject(new Error(stderr.trim() || `ripgrep exited with code ${code}`));
				return;
			}
			resolve(hits);
		});
	});
}
