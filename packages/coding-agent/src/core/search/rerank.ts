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

export function rerankCandidates(query: string, candidates: readonly FusedCandidate[], cwd: string): RerankResult {
	const startedMs = Date.now();
	const plan = buildLexicalQueryPlan(query);
	if (!plan || candidates.length < 2) {
		return { candidates: [...candidates], latencyMs: Date.now() - startedMs };
	}
	const terms = plan.terms;
	const queryPath = query.trim().toLowerCase();

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
			declaresAnyTerm = present.some((t) => declaresTerm(window, t));
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
