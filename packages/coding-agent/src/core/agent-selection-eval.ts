/**
 * Does the agent roster steer a dispatch to the right agent?
 *
 * The `Task` tool asks the model to pick a `subagent_type` from
 * `<available_agents>`, and that block contains nothing but each agent's
 * summarized description. So agent selection is the same question G4 asks about
 * skills — "given these descriptions and this situation, which one fires?" —
 * and it reuses the same harness rather than growing a second one.
 *
 * What this exists to decide: `plan` and `explore` ship with the same tools, the
 * same isolation and the same `background` flag, differing only in model tier
 * and output contract — and `complexity` on the Task tool already expresses the
 * tier. Whether they are two agents or one is a question about whether the model
 * can actually tell them apart from their descriptions, which is measurable and
 * was previously being argued from intuition.
 *
 * The candidates use `summarizeAgentDescription`, not the raw frontmatter: the
 * summary is what the system prompt actually emits, and evaluating the full
 * description would score text the model never sees.
 */

import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { loadAgentRegistry, summarizeAgentDescription } from "./agent-registry.js";
import {
	runTriggerEval,
	type TriggerCandidate,
	type TriggerCase,
	type TriggerEvalOutcome,
	type TriggerJudge,
} from "./extensions/plugins/trigger-eval.js";

/** How often one agent was chosen where another was expected. */
export interface ConfusionEntry {
	expected: string;
	actual: string;
	count: number;
}

export interface AgentSelectionReport {
	corpusHash: string;
	agents: string[];
	caseCount: number;
	/** Cases where the expected agent was chosen, over cases expecting any agent. */
	accuracy?: number;
	/** Cases correctly left to the parent, over cases expecting no delegation. */
	inlineAccuracy?: number;
	/** Every wrong pick, most frequent first. The pairs here are the finding. */
	confusion: ConfusionEntry[];
	/** Per-agent recall: chosen / expected. An agent nobody picks is dead weight. */
	perAgent: Array<{ agent: string; expected: number; chosen: number; recall: number }>;
}

/**
 * The built-in agents as judge candidates, described exactly as the system
 * prompt describes them.
 *
 * `own: true` for all of them: unlike a plugin eval there is no foreign roster
 * to discriminate against, so every case is scored against the same closed set.
 * The `expect: null` cases carry the discriminative half instead — they ask
 * whether the model declines to delegate work it should keep.
 */
export function agentCandidates(cwd: string = process.cwd()): TriggerCandidate[] {
	const registry = loadAgentRegistry({ cwd, includeBuiltins: true, includeClaude: false });
	return registry
		.list()
		.map((agent) => ({
			name: agent.name,
			description: summarizeAgentDescription(agent.description ?? ""),
			own: true,
		}))
		.sort((a, b) => a.name.localeCompare(b.name));
}

/** Load a gold set: `{ "cases": [{ "prompt": "...", "expect": "explore" | null }] }`. */
export function loadAgentCases(file: string): TriggerCase[] | undefined {
	if (!existsSync(file)) return undefined;
	try {
		const raw = JSON.parse(readFileSync(file, "utf-8")) as { cases?: unknown };
		if (!Array.isArray(raw.cases)) return undefined;
		const cases = raw.cases.filter(
			(c): c is TriggerCase =>
				!!c &&
				typeof c === "object" &&
				typeof (c as TriggerCase).prompt === "string" &&
				(c as TriggerCase).prompt.trim().length > 0 &&
				((c as TriggerCase).expect === null || typeof (c as TriggerCase).expect === "string"),
		);
		return cases.length > 0 ? cases : undefined;
	} catch {
		return undefined;
	}
}

/**
 * Every expected agent in the gold set must exist in the roster.
 *
 * A typo'd or removed agent name would otherwise score as a permanent miss and
 * read as a description problem, which is the most expensive way to be wrong
 * about an eval.
 */
export function validateAgentCases(candidates: readonly TriggerCandidate[], cases: readonly TriggerCase[]): string[] {
	const known = new Set(candidates.map((c) => c.name));
	const problems: string[] = [];
	for (const [i, testCase] of cases.entries()) {
		if (testCase.expect !== null && !known.has(testCase.expect)) {
			problems.push(`case ${i} expects "${testCase.expect}", which is not in the roster`);
		}
	}
	return problems;
}

function hashCorpus(candidates: readonly TriggerCandidate[], cases: readonly TriggerCase[]): string {
	const h = createHash("sha256");
	for (const c of candidates) h.update(`${c.name} ${c.description} `);
	for (const c of cases) h.update(`${c.prompt} ${c.expect ?? ""} `);
	return h.digest("hex").slice(0, 16);
}

/**
 * Turn a scored run into the report that answers the design question.
 *
 * `runTriggerEval`'s recall/specificity are the right numbers for a plugin
 * defending itself against a foreign roster. Here the roster is closed, so the
 * useful shape is a confusion matrix: which agent loses to which, and how often.
 * "explore and plan are interchangeable" is a claim about one cell.
 */
export function summarizeAgentSelection(
	outcome: Extract<TriggerEvalOutcome, { status: "ran" }>,
	candidates: readonly TriggerCandidate[],
): AgentSelectionReport {
	const confusion = new Map<string, ConfusionEntry>();
	const expectedCounts = new Map<string, number>();
	const chosenCounts = new Map<string, number>();
	let delegated = 0;
	let delegatedCorrect = 0;
	let inline = 0;
	let inlineCorrect = 0;

	for (const result of outcome.record.results) {
		if (result.expected === null) {
			inline++;
			if (result.actual === null) inlineCorrect++;
			continue;
		}
		delegated++;
		expectedCounts.set(result.expected, (expectedCounts.get(result.expected) ?? 0) + 1);
		if (result.correct) {
			delegatedCorrect++;
			chosenCounts.set(result.expected, (chosenCounts.get(result.expected) ?? 0) + 1);
			continue;
		}
		// "(none)" is a real outcome, not a missing value: declining to delegate
		// work that should have been delegated is a different failure from picking
		// the wrong agent, and collapsing them would hide which one is happening.
		const actual = result.actual ?? "(none)";
		const key = `${result.expected} ${actual}`;
		const entry = confusion.get(key) ?? { expected: result.expected, actual, count: 0 };
		entry.count++;
		confusion.set(key, entry);
	}

	const perAgent = candidates
		.map((candidate) => {
			const expected = expectedCounts.get(candidate.name) ?? 0;
			const chosen = chosenCounts.get(candidate.name) ?? 0;
			return { agent: candidate.name, expected, chosen, recall: expected > 0 ? chosen / expected : 0 };
		})
		.sort((a, b) => a.recall - b.recall);

	return {
		corpusHash: outcome.record.corpusHash,
		agents: candidates.map((c) => c.name),
		caseCount: outcome.record.caseCount,
		accuracy: delegated > 0 ? delegatedCorrect / delegated : undefined,
		inlineAccuracy: inline > 0 ? inlineCorrect / inline : undefined,
		confusion: [...confusion.values()].sort((a, b) => b.count - a.count),
		perAgent,
	};
}

export type AgentSelectionOutcome =
	| { status: "not-run"; reason: string }
	| { status: "ran"; report: AgentSelectionReport; outcome: Extract<TriggerEvalOutcome, { status: "ran" }> };

/** Score the roster against a gold set. Never throws; a missing model is `not-run`. */
export async function runAgentSelectionEval(
	candidates: readonly TriggerCandidate[],
	cases: readonly TriggerCase[] | undefined,
	judge: TriggerJudge | undefined,
): Promise<AgentSelectionOutcome> {
	if (cases && cases.length > 0) {
		const problems = validateAgentCases(candidates, cases);
		if (problems.length > 0) return { status: "not-run", reason: `invalid gold set: ${problems.join("; ")}` };
	}

	const outcome = await runTriggerEval("agent-selection", candidates, cases, judge);
	if (outcome.status === "not-run") return outcome;

	return {
		status: "ran",
		report: {
			...summarizeAgentSelection(outcome, candidates),
			// The shared harness hashes its own way; restate it over exactly what
			// this eval judged so two reports are comparable on their own terms.
			corpusHash: hashCorpus(candidates, cases ?? []),
		},
		outcome,
	};
}

/** Human-readable report for the CLI. */
export function formatAgentSelectionReport(report: AgentSelectionReport): string {
	const pct = (v: number | undefined) => (v === undefined ? "n/a" : `${Math.round(v * 100)}%`);
	const lines = [
		`corpus ${report.corpusHash} - ${report.caseCount} case(s) over ${report.agents.length} agent(s): ${report.agents.join(", ")}`,
		`delegation accuracy ${pct(report.accuracy)}   inline accuracy ${pct(report.inlineAccuracy)}`,
		"",
		"per agent (lowest recall first):",
	];
	for (const row of report.perAgent) {
		lines.push(`  ${row.agent.padEnd(18)} ${row.chosen}/${row.expected} chosen  (${pct(row.recall)})`);
	}
	if (report.confusion.length > 0) {
		lines.push("", "confusions (expected then actual):");
		for (const entry of report.confusion) {
			lines.push(`  ${entry.expected} => ${entry.actual}   x${entry.count}`);
		}
	}
	return lines.join("\n");
}
