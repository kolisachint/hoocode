/**
 * Is this already written down?
 *
 * The answer decides the most useful distinction the digest makes — `new` vs
 * `restated` vs `has-skill` — and it used to be decided by bag-of-words
 * overlap: count how many content words of the proposal appear anywhere in a
 * rule line, call it covered above 0.6. That is wrong in both directions and
 * for the same reason, namely that it does not read. It calls "always run tests
 * before pushing" covered by a line about "running the test suite in CI", and
 * it misses a real paraphrase that happens to pick different vocabulary.
 *
 * Both mistakes are expensive. A false `restated` accuses a rule that is
 * working of not working, and tells the reader to rewrite something fine. A
 * false `new` proposes a rule they already have, which is how a context file
 * grows duplicates.
 *
 * So a model reads the rules and the proposals together and matches them. One
 * call for the whole batch, because the question is small and the corpus is the
 * same for every item — the context file is a few thousand tokens and does not
 * want re-sending once per proposal.
 */

import type { Model } from "@kolisachint/hoocode-ai";
import { completeSimple } from "@kolisachint/hoocode-ai";

export interface CoverageIndex {
	/** Candidate rule lines from the repo context file and both user scopes. */
	ruleLines: string[];
	skills: Array<{ name: string; description: string }>;
}

export interface CoverageMatch {
	/** The context-file line that covers this, if any. */
	rule?: string;
	/** The skill that covers this, if any. Only set when no rule matched. */
	skill?: string;
}

/** One thing to look up, identified by the label the reduce step grouped on. */
export interface CoverageQuery {
	label: string;
	text: string;
}

/**
 * Decide coverage for a batch. Injectable so the pipeline can be tested without
 * a model, and so a run with no model configured can degrade to "everything is
 * new" rather than failing.
 */
export type CoverageJudge = (
	queries: CoverageQuery[],
	index: CoverageIndex,
	signal?: AbortSignal,
) => Promise<Map<string, CoverageMatch>>;

/** Rule lines sent per call. A context file longer than this is already the problem. */
const MAX_RULE_LINES = 400;
/** Skills sent per call. */
const MAX_SKILLS = 120;
/** Description characters per skill — the opening says what it does; the rest is trigger bait. */
const SKILL_DESCRIPTION_CHARS = 300;
const MAX_RESPONSE_TOKENS = 2_000;

const COVERAGE_SYSTEM_PROMPT = `You decide whether each proposed rule is ALREADY covered by existing project rules or skills.

You are given numbered RULES (lines from context files), numbered SKILLS (name and description), and numbered PROPOSALS.

Each rule reads \`[scope] Heading > Subheading > line\`. The scope is \`repo\` (binds work in this project) or \`user\` (binds everywhere). The heading path is the section the line lives under, and it is what tells you the line's subject when the line alone is ambiguous.

For each proposal, decide:
- "rule" — an existing rule already says this. The reader repeating it means that rule is not working, so it should be rewritten rather than duplicated.
- "skill" — an existing skill already does this, and the reader asked by hand anyway. Usually the skill's description does not describe the situation they were in.
- "new" — nothing covers it.

Judge by MEANING, not by shared words. Different vocabulary for the same instruction is covered. Shared vocabulary about different things is NOT covered:
- proposal "always use bun, never npm" vs rule "install dependencies with bun" → covered (rule)
- proposal "run tests before pushing" vs rule "CI runs the test suite on every PR" → NOT covered, these are different instructions to different actors
- proposal "prefer table output" vs rule "use tables in documentation" → NOT covered unless the scope matches

Prefer "new" when genuinely unsure. A false "covered" tells the reader to rewrite a rule that is fine; a false "new" merely proposes something they can reject.

Rules win over skills when both match: rewriting a line is more actionable than sharpening a description.

Output STRICT JSON, no markdown fence, no prose. Use the proposal's exact label:
{"verdicts":[{"label":"use-bun-not-npm","verdict":"rule","ruleIndex":3},{"label":"scaffold-route","verdict":"skill","skillIndex":1},{"label":"prefer-tables","verdict":"new"}]}`;

function buildPrompt(queries: CoverageQuery[], index: CoverageIndex): string {
	const lines: string[] = [];

	lines.push("RULES:");
	const rules = index.ruleLines.slice(0, MAX_RULE_LINES);
	if (rules.length === 0) lines.push("(none)");
	for (const [i, rule] of rules.entries()) {
		lines.push(`${i}. ${rule}`);
	}

	lines.push("", "SKILLS:");
	const skills = index.skills.slice(0, MAX_SKILLS);
	if (skills.length === 0) lines.push("(none)");
	for (const [i, skill] of skills.entries()) {
		lines.push(`${i}. ${skill.name} — ${skill.description.slice(0, SKILL_DESCRIPTION_CHARS)}`);
	}

	lines.push("", "PROPOSALS:");
	for (const query of queries) {
		lines.push(`- label: ${query.label}\n  text: ${query.text}`);
	}

	return lines.join("\n");
}

/** Read the verdict list back, ignoring anything malformed rather than failing the run. */
export function parseVerdicts(
	response: string,
	queries: CoverageQuery[],
	index: CoverageIndex,
): Map<string, CoverageMatch> {
	const out = new Map<string, CoverageMatch>();
	const start = response.indexOf("{");
	const end = response.lastIndexOf("}");
	if (start < 0 || end <= start) return out;

	let parsed: unknown;
	try {
		parsed = JSON.parse(response.slice(start, end + 1));
	} catch {
		return out;
	}

	const raw = (parsed as { verdicts?: unknown })?.verdicts;
	if (!Array.isArray(raw)) return out;

	const known = new Set(queries.map((q) => q.label));
	for (const item of raw) {
		if (!item || typeof item !== "object") continue;
		const verdict = item as Record<string, unknown>;
		const label = typeof verdict.label === "string" ? verdict.label.trim().toLowerCase() : "";
		// A label the batch did not ask about is a hallucinated row; dropping it is
		// safer than letting it mark some other proposal covered.
		if (!label || !known.has(label)) continue;

		if (verdict.verdict === "rule") {
			const at = typeof verdict.ruleIndex === "number" ? index.ruleLines[verdict.ruleIndex] : undefined;
			// An out-of-range index means the model decided "covered" but cannot show
			// which line. Treat that as `new`: the reader cannot act on an unnamed rule.
			if (at) out.set(label, { rule: at });
			continue;
		}
		if (verdict.verdict === "skill") {
			const at = typeof verdict.skillIndex === "number" ? index.skills[verdict.skillIndex] : undefined;
			if (at) out.set(label, { skill: at.name });
			continue;
		}
		out.set(label, {});
	}
	return out;
}

export interface CoverageDeps {
	model: Model<any>;
	apiKey?: string;
	headers?: Record<string, string>;
}

export function createLlmCoverageJudge(deps: CoverageDeps): CoverageJudge {
	return async (queries, index, signal) => {
		// Nothing to match against means nothing can be covered, and the call would
		// be pure cost.
		if (queries.length === 0 || (index.ruleLines.length === 0 && index.skills.length === 0)) {
			return new Map();
		}

		const response = await completeSimple(
			deps.model,
			{
				systemPrompt: COVERAGE_SYSTEM_PROMPT,
				messages: [
					{ role: "user", content: [{ type: "text", text: buildPrompt(queries, index) }], timestamp: Date.now() },
				],
			},
			{ maxTokens: MAX_RESPONSE_TOKENS, signal, apiKey: deps.apiKey, headers: deps.headers },
		);

		if (response.stopReason === "error") {
			throw new Error(response.errorMessage || "coverage call failed");
		}

		const text = response.content
			.filter((c): c is { type: "text"; text: string } => c.type === "text")
			.map((c) => c.text)
			.join("\n");
		return parseVerdicts(text, queries, index);
	};
}

/** Everything is new. Used when no model is available, so the run still produces a digest. */
export const noCoverageJudge: CoverageJudge = async () => new Map();
