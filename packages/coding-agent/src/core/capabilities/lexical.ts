/**
 * In-memory BM25 over the capability index — the leg that always works.
 *
 * Deliberately not the repo's lexical retriever: that one shells out to ripgrep
 * over files on disk, and capabilities are a few hundred short strings held in
 * memory. It is also not optional. The dense leg needs a binary that may not be
 * installed and a store that may still be building, so if retrieval depended on
 * it, "find me a tool that sends email" would work on some machines and not
 * others. This leg makes the floor deterministic and dependency-free; dense is
 * strictly additive on top.
 *
 * The tokenizer does the load-bearing work here. Tool names are the query terms
 * that matter most and they arrive as `mcp_github_create_pull_request` or
 * `createPullRequest`, so a naive whitespace split would make the single most
 * common query shape — a name the model half-remembers — the one thing BM25
 * cannot match.
 */

import type { CapabilityDoc } from "./registry.js";

/** Standard Okapi BM25 parameters. Nothing here justifies tuning them. */
const K1 = 1.2;
const B = 0.75;

/**
 * Fold a regular English plural to its singular, or return undefined.
 *
 * Deliberately crude: only the endings that are unambiguous without a
 * dictionary. Documentation headings are written in whichever number reads best
 * ("Themes", "Custom Providers") while questions are asked in the other ("how
 * do I add a theme"), and with no folding at all those two never meet — the
 * single most useful term in the query is the one term that cannot match.
 *
 * Irregulars are left alone. Getting "indices" wrong costs a missed hit;
 * inventing a stemmer that mangles "status" into "statu" would cost matches
 * that work today.
 */
function singularize(token: string): string | undefined {
	if (token.length > 4 && token.endsWith("ies")) return `${token.slice(0, -3)}y`;
	if (token.length > 4 && /(?:ss|sh|ch|x|z)es$/.test(token)) return token.slice(0, -2);
	if (token.length > 3 && token.endsWith("s") && !token.endsWith("ss") && !token.endsWith("us")) {
		return token.slice(0, -1);
	}
	return undefined;
}

/**
 * Split identifiers the way a person reads them: `mcp_github_create_pr` and
 * `createPullRequest` both yield their parts, and the original token is kept so
 * an exact name still scores as an exact match.
 *
 * Singular forms are emitted *alongside* the originals rather than replacing
 * them, which is the same bargain the identifier splitting makes: an exact
 * token still matches exactly, and a near miss now matches too.
 */
export function tokenize(text: string): string[] {
	const out: string[] = [];
	const push = (token: string): void => {
		out.push(token);
		const singular = singularize(token);
		if (singular && singular !== token) out.push(singular);
	};
	for (const raw of text.toLowerCase().match(/[a-z0-9]+(?:[_-][a-z0-9]+)*/gi) ?? []) {
		const token = raw.toLowerCase();
		push(token);
		// Split on separators, then on camelCase boundaries in the source text.
		const parts = token.split(/[_-]+/).filter(Boolean);
		if (parts.length > 1) for (const part of parts) push(part);
	}
	for (const camel of text.match(/[a-z][a-z0-9]*|[A-Z][a-z0-9]*|[A-Z]+(?![a-z])/g) ?? []) {
		const lower = camel.toLowerCase();
		if (lower.length > 1) push(lower);
	}
	return out;
}

/**
 * Function words dropped from *queries* only.
 *
 * Questions arrive as "how do I add a custom theme", and in a corpus of a few
 * hundred short documents the filler carries real weight: a section whose prose
 * happens to say "Add an AGENTS.md file ... to tell it how to work" outscores
 * the section actually titled "Creating a Custom Theme", because it matched
 * three throwaway words to the target's one meaningful one.
 *
 * Query-side only, deliberately. Stripping these from documents too would
 * change every document length and every average, re-tuning a ranking that
 * works; dropping a term from the query just stops it contributing, which is
 * the whole intent.
 */
const QUERY_STOPWORDS = new Set([
	"a",
	"an",
	"and",
	"are",
	"as",
	"at",
	"be",
	"by",
	"can",
	"do",
	"does",
	"for",
	"from",
	"get",
	"how",
	"i",
	"in",
	"is",
	"it",
	"its",
	"me",
	"my",
	"of",
	"on",
	"or",
	"so",
	"that",
	"the",
	"then",
	"there",
	"this",
	"to",
	"use",
	"using",
	"want",
	"was",
	"what",
	"when",
	"where",
	"which",
	"who",
	"why",
	"will",
	"with",
	"you",
	"your",
]);

/** The text a document is matched on: name first, since that is what queries name. */
function documentText(doc: CapabilityDoc): string {
	return `${doc.name} ${doc.name} ${doc.source ?? ""} ${doc.description}`;
}

export interface LexicalHit {
	id: string;
	score: number;
}

interface Posting {
	id: string;
	length: number;
	counts: Map<string, number>;
}

/**
 * A built BM25 index. Cheap enough to rebuild whenever the capability set
 * changes — a few hundred short documents — so there is no invalidation story
 * to get wrong.
 */
export class LexicalIndex {
	private readonly postings: Posting[] = [];
	private readonly docFreq = new Map<string, number>();
	private readonly avgLength: number;

	constructor(docs: readonly CapabilityDoc[]) {
		let total = 0;
		for (const doc of docs) {
			const tokens = tokenize(documentText(doc));
			const counts = new Map<string, number>();
			for (const t of tokens) counts.set(t, (counts.get(t) ?? 0) + 1);
			for (const t of counts.keys()) this.docFreq.set(t, (this.docFreq.get(t) ?? 0) + 1);
			this.postings.push({ id: doc.id, length: tokens.length, counts });
			total += tokens.length;
		}
		this.avgLength = this.postings.length > 0 ? total / this.postings.length : 0;
	}

	get size(): number {
		return this.postings.length;
	}

	/** Top `k` documents for `query`, best first. Documents scoring zero are omitted. */
	search(query: string, k = 10): LexicalHit[] {
		const raw = tokenize(query);
		// Fall back to the unfiltered terms when a query is nothing but function
		// words, so "what is it" still searches rather than silently matching all.
		const filtered = raw.filter((t) => !QUERY_STOPWORDS.has(t));
		const terms = filtered.length > 0 ? filtered : raw;
		if (terms.length === 0 || this.postings.length === 0) return [];
		const n = this.postings.length;

		const hits: LexicalHit[] = [];
		for (const posting of this.postings) {
			let score = 0;
			for (const term of new Set(terms)) {
				const tf = posting.counts.get(term);
				if (!tf) continue;
				const df = this.docFreq.get(term) ?? 0;
				// Okapi IDF, floored at zero: a term in every document carries no
				// signal, and the raw formula would make it actively negative.
				const idf = Math.max(0, Math.log(1 + (n - df + 0.5) / (df + 0.5)));
				const norm = tf * (K1 + 1);
				const denom = tf + K1 * (1 - B + (B * posting.length) / (this.avgLength || 1));
				score += idf * (norm / denom);
			}
			if (score > 0) hits.push({ id: posting.id, score });
		}

		// Ties break by id so the same query always returns the same order — a
		// retrieval tool that reshuffles equal-scoring results is a reproducibility
		// problem disguised as a ranking one.
		hits.sort((a, b) => b.score - a.score || a.id.localeCompare(b.id));
		return hits.slice(0, k);
	}
}
