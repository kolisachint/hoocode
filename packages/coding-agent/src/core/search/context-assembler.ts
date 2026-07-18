/**
 * Token-budgeted span expansion (docs/hybrid-retrieval-design.md, step 5 of
 * the shipping order).
 *
 * Retrieval works on chunk ids; only here — after fusion — are line windows
 * read from disk. Every candidate gets a compact `path:start-end [sources]`
 * header; snippets are added top-down until the budget runs out, so the model
 * always sees the full ranked list but never an unbounded dump.
 */

import { readFileSync } from "fs";
import path from "path";
import type { FusedCandidate } from "./types.js";

/** Rough chars-per-token for budgeting (index-time uses the same heuristic). */
const CHARS_PER_TOKEN = 4;
const DEFAULT_TOKEN_BUDGET = 2000;
/** Snippet caps keep one giant chunk from eating the whole budget. Results
 *  past the top few get a shallower snippet: rank carries most of the value,
 *  and full-depth snippets for every result roughly doubles the token cost. */
const MAX_SNIPPET_LINES = 20;
const TOP_FULL_SNIPPETS = 3;
const TAIL_SNIPPET_LINES = 8;
const MAX_SNIPPET_LINE_CHARS = 200;

export interface AssembleOptions {
	cwd: string;
	/** Approximate token budget for the whole result text. */
	tokenBudget?: number;
}

export interface AssembledContext {
	text: string;
	/** How many candidates got an inline snippet (the rest are bare headers). */
	snippetCount: number;
}

function sourcesLabel(candidate: FusedCandidate): string {
	return Object.keys(candidate.ranks).sort().join("+");
}

export function assembleContext(candidates: readonly FusedCandidate[], options: AssembleOptions): AssembledContext {
	const budgetChars = (options.tokenBudget ?? DEFAULT_TOKEN_BUDGET) * CHARS_PER_TOKEN;
	const fileCache = new Map<string, string[] | undefined>();

	const readLines = (rel: string): string[] | undefined => {
		if (!fileCache.has(rel)) {
			try {
				const content = readFileSync(path.resolve(options.cwd, rel), "utf-8");
				fileCache.set(rel, content.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n"));
			} catch {
				fileCache.set(rel, undefined);
			}
		}
		return fileCache.get(rel);
	};

	const sections: string[] = [];
	let usedChars = 0;
	let snippetCount = 0;
	let snippetsExhausted = false;

	for (const candidate of candidates) {
		const lines = readLines(candidate.path);
		// Clamp the span to the file as it exists now (fallback spans may
		// overshoot; the file may have changed since indexing).
		const start = Math.max(1, candidate.startLine);
		const end = lines ? Math.min(candidate.endLine, lines.length) : candidate.endLine;
		const header = `${candidate.path}:${start}-${end} [${sourcesLabel(candidate)}]`;
		usedChars += header.length + 1;

		if (!snippetsExhausted && lines && end >= start) {
			const depth = snippetCount < TOP_FULL_SNIPPETS ? MAX_SNIPPET_LINES : TAIL_SNIPPET_LINES;
			const snippetEnd = Math.min(end, start + depth - 1);
			const rawLines = lines.slice(start - 1, snippetEnd);
			// Chunk spans often end on a blank line (trailing-newline artifact);
			// trailing blanks carry no signal, so drop them.
			while (rawLines.length > 0 && rawLines[rawLines.length - 1].trim() === "") rawLines.pop();
			const snippetLines = rawLines.map(
				(text, i) =>
					`  ${start + i}: ${text.length > MAX_SNIPPET_LINE_CHARS ? `${text.slice(0, MAX_SNIPPET_LINE_CHARS)}…` : text}`,
			);
			const snippet = snippetLines.join("\n");
			if (snippet) {
				if (usedChars + snippet.length <= budgetChars) {
					sections.push(`${header}\n${snippet}`);
					usedChars += snippet.length + 1;
					snippetCount++;
					continue;
				}
				// Budget hit: stop expanding, keep listing bare headers.
				snippetsExhausted = true;
			}
		}
		sections.push(header);
	}

	return { text: sections.join("\n\n"), snippetCount };
}
