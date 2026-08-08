/**
 * Deterministic reranker over the fused top-50
 * (docs/hybrid-retrieval-design.md, step 7 of the shipping order).
 *
 * The eval gate showed fused Recall@50 well above Recall@5/10 — the right
 * candidates survive fusion but sit too deep. This reranker re-orders them
 * using evidence that is only cheap to compute *after* fusion, when there
 * are ≤50 candidates instead of thousands of lines:
 *
 *   - term coverage: how many distinct query terms appear in the candidate's
 *     actual expanded window (read from disk);
 *   - path affinity: query terms appearing in the candidate's file path —
 *     this is what lets a query like `core/search/hybrid-search.ts` rank the
 *     file itself first, which content grep alone cannot do;
 *   - fused prior: the RRF ordering, so retriever consensus still counts.
 *
 * Not every signal suits every query. A query that names something and a query
 * that describes behaviour want different evidence, so the name-matching
 * signal is gated on {@link queryIsProse} — see its comment for the numbers.
 *
 * Purely lexical-statistical and deterministic — no model, no I/O beyond
 * reading candidate windows. A cross-encoder can later replace the scoring
 * function behind the same signature; that model work belongs to
 * `kolisachint/embeddingsearchtools`, not here.
 */

import { readFileSync } from "fs";
import path from "path";
import { buildLexicalQueryPlan } from "./lexical-retriever.js";
import type { FusedCandidate } from "./types.js";

/** Weights of the scoring blend. The eval harness (`bun run search-eval`) is
 *  the instrument for changing them — don't tune blind. */
const WEIGHT_FUSED_PRIOR = 0.4;
const WEIGHT_TERM_COVERAGE = 0.35;
const WEIGHT_PATH_AFFINITY = 0.25;
/** Additive bonus when the query *is* the candidate's path (or its suffix):
 *  the caller named the file, so no amount of content evidence elsewhere
 *  should outrank it. */
const EXACT_PATH_BONUS = 0.5;
/**
 * Additive bonus when the window *declares* a query term rather than merely
 * mentioning it.
 *
 * This targets the largest measured gap in the eval: on the 22 exact-symbol
 * queries the definition is in the top 10 about 85% of the time but ranked
 * first only about 20% of the time. Call sites outnumber definitions and
 * contain the identical identifier, so term coverage — which saturates at 1.0
 * for both — cannot separate them. Structure can.
 */
const DECLARATION_BONUS = 0.3;

/**
 * Sentence glue. Identifiers and paths never contain these words, so two or
 * more of them means the query is a sentence rather than a name.
 *
 * This gates {@link DECLARATION_BONUS} because that bonus is a *name-matching*
 * signal and a prose query has no name to match. Its plan terms are ordinary
 * words — "results", "search", "index" — so it fires on whichever candidate
 * happens to declare a variable by one of them, at a weight larger than the
 * entire path-affinity term, and buries the fused ordering that did know the
 * answer. On the conceptual class reranking was scoring *below not reranking
 * at all* (MRR 0.246 un-reranked vs 0.124 reranked on `semantic`).
 *
 * Measured on the 62-query gold set, gating it moved conceptual MRR +0.046
 * (`semantic +rr`), +0.040 (`auto +rr`), +0.056 (`bm25+dense +rr`) and left
 * exact-symbol, error-fragment and path bit-identical — the gate never fires
 * on those, by construction. Overall MRR +0.007 to +0.013, which the paired
 * sign test does not call significant (p = 0.11 to 0.29); the classes it
 * protects are what justify it, not the aggregate.
 *
 * Path affinity is deliberately *not* gated. It is a topic signal, not a name
 * signal: "how does a grep line number become an embedding chunk id" wants
 * files with `grep` and `chunk` in the path. Gating it too was measured and
 * was strictly worse — same conceptual gain, roughly double the cross-file
 * loss (−0.058 vs −0.032 on `semantic +rr`).
 *
 * Deliberately conservative: two hits, not one, so a terse query like
 * `hybrid search fusion` keeps today's scoring untouched.
 */
const PROSE_FUNCTION_WORDS = new Set([
	"a",
	"after",
	"all",
	"an",
	"and",
	"any",
	"are",
	"as",
	"at",
	"be",
	"been",
	"before",
	"between",
	"but",
	"by",
	"can",
	"does",
	"do",
	"each",
	"for",
	"from",
	"had",
	"has",
	"have",
	"how",
	"if",
	"in",
	"into",
	"is",
	"it",
	"its",
	"of",
	"on",
	"one",
	"or",
	"should",
	"so",
	"than",
	"that",
	"the",
	"then",
	"this",
	"to",
	"under",
	"was",
	"were",
	"what",
	"when",
	"where",
	"which",
	"why",
	"with",
	"would",
]);
/** Function words needed before a query counts as prose. */
const PROSE_WORD_THRESHOLD = 2;

/**
 * Is `query` a sentence rather than a name?
 *
 * A quoted segment is never prose regardless of its words: the plan collapses
 * it to one literal term, so the declaration bonus cannot fire on it anyway,
 * and its path-token split is what lets `"Theme not initialized…"` find
 * `core/theme.ts`.
 */
export function queryIsProse(query: string): boolean {
	if (/["'`][^"'`]+["'`]/.test(query)) return false;
	const words = query.toLowerCase().match(/[a-z]+/g) ?? [];
	let hits = 0;
	for (const word of words) {
		if (PROSE_FUNCTION_WORDS.has(word) && ++hits >= PROSE_WORD_THRESHOLD) return true;
	}
	return false;
}

/** Keywords that introduce a definition across the languages this indexes.
 *  Matched against lowercased text, so the term is lowercased too. */
const DECLARATION_KEYWORDS =
	"function|class|interface|type|enum|struct|impl|trait|fn|def|const|let|var|namespace|module";

/** Does `window` declare `term`, as opposed to referencing it? */
function declaresTerm(window: string, term: string): boolean {
	const escaped = term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
	// `function foo(`, `class Foo {`, `const foo =` ...
	if (new RegExp(`\\b(?:${DECLARATION_KEYWORDS})\\s+${escaped}\\b`).test(window)) return true;
	// `foo(...) {` at the start of a line — methods, Go/Rust receivers, Python defs
	// already covered above, but this catches object-literal and class members.
	if (
		new RegExp(`^\\s*(?:(?:async|public|private|protected|static|export)\\s+)*${escaped}\\s*[(<]`, "m").test(window)
	) {
		return true;
	}
	return false;
}

/** Inverse document frequency over the candidate pool.
 *
 * True corpus IDF lives in the BM25 index and is not exposed over the daemon
 * protocol, so this approximates it with the candidate set: a term present in
 * every candidate discriminates nothing, one present in three carries the
 * signal. That is the comparison the reranker actually needs to make, since it
 * only ever orders candidates against each other. */
function inverseDocumentFrequency(documentFrequency: number, total: number): number {
	return Math.log(1 + total / Math.max(1, documentFrequency));
}

export interface RerankResult {
	candidates: FusedCandidate[];
	latencyMs: number;
}

/**
 * The exact source text each candidate stands for, as the model should see it.
 *
 * Shared with the cross-encoder path so both rerankers score identical text —
 * otherwise a comparison between them would partly measure which one got a
 * better view of the candidate.
 */
export function readCandidateWindows(candidates: readonly FusedCandidate[], cwd: string): Array<string | undefined> {
	const fileCache = new Map<string, string[] | undefined>();
	const read = (rel: string): string[] | undefined => {
		if (!fileCache.has(rel)) {
			try {
				fileCache.set(rel, readFileSync(path.resolve(cwd, rel), "utf-8").split("\n"));
			} catch {
				fileCache.set(rel, undefined);
			}
		}
		return fileCache.get(rel);
	};
	return candidates.map((candidate) => {
		const lines = read(candidate.path);
		if (!lines) return undefined;
		return lines.slice(Math.max(0, candidate.startLine - 1), Math.min(lines.length, candidate.endLine)).join("\n");
	});
}

export function rerankCandidates(query: string, candidates: readonly FusedCandidate[], cwd: string): RerankResult {
	const startedMs = Date.now();
	const plan = buildLexicalQueryPlan(query);
	if (!plan || candidates.length < 2) {
		return { candidates: [...candidates], latencyMs: Date.now() - startedMs };
	}
	const terms = plan.terms;
	const queryPath = query.trim().toLowerCase();
	// Prose asks about behaviour, not about a name, so the one signal that reads
	// candidates *as names* is switched off.
	const prose = queryIsProse(query);

	const fileCache = new Map<string, string[] | undefined>();
	const readLines = (rel: string): string[] | undefined => {
		if (!fileCache.has(rel)) {
			try {
				const content = readFileSync(path.resolve(cwd, rel), "utf-8");
				fileCache.set(rel, content.toLowerCase().split("\n"));
			} catch {
				fileCache.set(rel, undefined);
			}
		}
		return fileCache.get(rel);
	};

	// Read every candidate window once: the term/declaration signals and the
	// candidate-pool IDF all need them, and files repeat across candidates.
	const windows = candidates.map((candidate) => {
		const lines = readLines(candidate.path);
		if (!lines) return undefined;
		return lines.slice(Math.max(0, candidate.startLine - 1), Math.min(lines.length, candidate.endLine)).join("\n");
	});

	// Candidate-pool document frequency per term, for the IDF weighting below.
	const documentFrequency = new Map<string, number>();
	for (const term of terms) {
		documentFrequency.set(term, windows.filter((w) => w?.includes(term)).length);
	}
	const termWeight = new Map(
		terms.map((t) => [t, inverseDocumentFrequency(documentFrequency.get(t) ?? 0, candidates.length)]),
	);
	const totalTermWeight = terms.reduce((sum, t) => sum + (termWeight.get(t) ?? 0), 0);

	// Fused prior normalized by score, not by position: a candidate both
	// retrievers agreed on should outrank one that squeaked in, and a uniform
	// 1 - index/length ramp throws that magnitude away.
	const maxRrfScore = Math.max(...candidates.map((c) => c.rrfScore), Number.MIN_VALUE);

	const scored = candidates.map((candidate, index) => {
		const fusedPrior = candidate.rrfScore / maxRrfScore;

		const window = windows[index];
		let termCoverage = 0;
		let declaresAnyTerm = false;
		if (window && terms.length > 0) {
			const present = terms.filter((t) => window.includes(t));
			termCoverage =
				totalTermWeight > 0
					? present.reduce((sum, t) => sum + (termWeight.get(t) ?? 0), 0) / totalTermWeight
					: present.length / terms.length;
			declaresAnyTerm = !prose && present.some((t) => declaresTerm(window, t));
		}

		const lowerPath = candidate.path.toLowerCase();
		// A quoted phrase rarely names a file; split it into path-ish tokens so
		// `"token budget exceeded"` still gets partial path credit.
		const pathTerms = terms.length === 1 ? terms[0].split(/[^a-z0-9_$]+/).filter((t) => t.length >= 3) : terms;
		const pathAffinity =
			pathTerms.length > 0 ? pathTerms.filter((t) => lowerPath.includes(t)).length / pathTerms.length : 0;

		const exactPath =
			queryPath.length >= 3 && (lowerPath === queryPath || lowerPath.endsWith(`/${queryPath}`)) ? 1 : 0;

		const score =
			WEIGHT_FUSED_PRIOR * fusedPrior +
			WEIGHT_TERM_COVERAGE * termCoverage +
			WEIGHT_PATH_AFFINITY * pathAffinity +
			EXACT_PATH_BONUS * exactPath +
			(declaresAnyTerm ? DECLARATION_BONUS : 0);
		return { candidate, index, score };
	});

	// Stable, deterministic: score desc, fused order as tie-break.
	scored.sort((a, b) => b.score - a.score || a.index - b.index);
	return { candidates: scored.map((s) => s.candidate), latencyMs: Date.now() - startedMs };
}
