/**
 * The agent-selection eval, and the trigger judge it shares with G4.
 *
 * The live run needs a model, so what is held here is everything around it: the
 * roster is described to the judge exactly as the system prompt describes it,
 * the gold set matches the roster, the scoring produces the confusion matrix the
 * plan/explore decision rests on, and a judge that returns garbage degrades to
 * "nothing fired" rather than to shifted verdicts scoring the wrong prompts.
 */

import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { summarizeAgentDescription } from "../src/core/agent-registry.js";
import {
	agentCandidates,
	loadAgentCases,
	runAgentSelectionEval,
	validateAgentCases,
} from "../src/core/agent-selection-eval.js";
import type { TriggerCandidate, TriggerCase, TriggerJudge } from "../src/core/extensions/plugins/trigger-eval.js";
import { parseTriggerVerdicts } from "../src/core/extensions/plugins/trigger-judge.js";

const packageRoot = join(__dirname, "..");
const goldPath = join(packageRoot, "test", "fixtures", "agent-selection.json");

/** A judge that replays a fixed verdict list, so scoring is testable without a model. */
function scriptedJudge(verdicts: (string | null)[]): TriggerJudge {
	return async () => verdicts;
}

describe("the roster as the judge sees it", () => {
	it("offers the built-in agents", () => {
		const names = agentCandidates(packageRoot).map((c) => c.name);
		expect(names).toContain("explore");
		expect(names).toContain("plan");
		expect(names).toContain("general-purpose");
	});

	it("describes each agent with the text the system prompt emits, not the raw frontmatter", () => {
		// <available_agents> carries the summarized description. Judging the full
		// frontmatter would score text the model never sees — and the summary is
		// exactly the part that drops the "DO NOT use for" half, which is where the
		// explore/plan distinction is spelled out most explicitly.
		for (const candidate of agentCandidates(packageRoot)) {
			expect(candidate.description).toBe(summarizeAgentDescription(candidate.description));
			expect(candidate.description).not.toContain("DO NOT use");
			expect(candidate.own).toBe(true);
		}
	});
});

describe("the gold set", () => {
	const cases = loadAgentCases(goldPath);

	it("loads", () => {
		expect(cases).toBeDefined();
		expect((cases as TriggerCase[]).length).toBeGreaterThan(20);
	});

	it("only expects agents that exist", () => {
		// The guard that matters: renaming or removing an agent without updating
		// the gold set would score as a permanent miss and read like the
		// description got worse.
		expect(validateAgentCases(agentCandidates(packageRoot), cases as TriggerCase[])).toEqual([]);
	});

	it("covers every agent, and covers not delegating at all", () => {
		const expected = new Set((cases as TriggerCase[]).map((c) => c.expect));
		for (const candidate of agentCandidates(packageRoot)) {
			expect(expected, `no case expects ${candidate.name}`).toContain(candidate.name);
		}
		// Without these an over-eager roster scores perfectly while making the
		// agent worse, by shipping trivial work to a subagent that cannot see the
		// conversation.
		expect(expected).toContain(null);
	});
});

describe("scoring", () => {
	const candidates: TriggerCandidate[] = [
		{ name: "explore", description: "read-only investigation", own: true },
		{ name: "plan", description: "research then plan", own: true },
	];
	const cases: TriggerCase[] = [
		{ prompt: "where is X", expect: "explore" },
		{ prompt: "trace Y", expect: "explore" },
		{ prompt: "plan the migration", expect: "plan" },
		{ prompt: "rename a local", expect: null },
	];

	it("reports accuracy separately for delegation and for staying inline", async () => {
		// explore correct, explore mistaken for plan, plan correct, inline correct.
		const outcome = await runAgentSelectionEval(candidates, cases, scriptedJudge(["explore", "plan", "plan", null]));
		expect(outcome.status).toBe("ran");
		if (outcome.status !== "ran") return;
		expect(outcome.report.accuracy).toBeCloseTo(2 / 3);
		expect(outcome.report.inlineAccuracy).toBe(1);
	});

	it("names which agent lost to which, which is the whole point", async () => {
		const outcome = await runAgentSelectionEval(candidates, cases, scriptedJudge(["plan", "plan", "plan", null]));
		if (outcome.status !== "ran") throw new Error("expected a run");
		expect(outcome.report.confusion).toEqual([{ expected: "explore", actual: "plan", count: 2 }]);
		const explore = outcome.report.perAgent.find((row) => row.agent === "explore");
		expect(explore).toEqual({ agent: "explore", expected: 2, chosen: 0, recall: 0 });
	});

	it("separates 'picked the wrong agent' from 'declined to delegate'", async () => {
		// Both are wrong, and collapsing them would hide which failure is
		// happening — an over-narrow description versus an over-broad sibling.
		const outcome = await runAgentSelectionEval(candidates, cases, scriptedJudge([null, "plan", "plan", null]));
		if (outcome.status !== "ran") throw new Error("expected a run");
		expect(outcome.report.confusion).toContainEqual({ expected: "explore", actual: "(none)", count: 1 });
		expect(outcome.report.confusion).toContainEqual({ expected: "explore", actual: "plan", count: 1 });
	});

	it("counts a false delegation against inline accuracy", async () => {
		const outcome = await runAgentSelectionEval(
			candidates,
			cases,
			scriptedJudge(["explore", "explore", "plan", "explore"]),
		);
		if (outcome.status !== "ran") throw new Error("expected a run");
		expect(outcome.report.accuracy).toBe(1);
		expect(outcome.report.inlineAccuracy).toBe(0);
	});

	it("pins what was judged, so two reports are comparable", async () => {
		const a = await runAgentSelectionEval(candidates, cases, scriptedJudge(["explore", "explore", "plan", null]));
		const edited = [{ ...candidates[0], description: "reworded" }, candidates[1]];
		const b = await runAgentSelectionEval(edited, cases, scriptedJudge(["explore", "explore", "plan", null]));
		if (a.status !== "ran" || b.status !== "ran") throw new Error("expected runs");
		// A rerun after a description edit is measuring something else. The hash is
		// what stops the two numbers being compared as if they were the same eval.
		expect(a.report.corpusHash).not.toBe(b.report.corpusHash);
	});
});

describe("degrading honestly", () => {
	const candidates: TriggerCandidate[] = [{ name: "explore", description: "read-only", own: true }];
	const cases: TriggerCase[] = [{ prompt: "where is X", expect: "explore" }];

	it("is not-run without a judge, never a silent pass", async () => {
		const outcome = await runAgentSelectionEval(candidates, cases, undefined);
		expect(outcome.status).toBe("not-run");
	});

	it("is not-run when the gold set names an agent the roster does not have", async () => {
		const outcome = await runAgentSelectionEval(
			candidates,
			[{ prompt: "p", expect: "ghost" }],
			scriptedJudge(["explore"]),
		);
		expect(outcome.status).toBe("not-run");
		if (outcome.status !== "not-run") return;
		expect(outcome.reason).toContain("ghost");
	});

	it("is not-run when the judge answers the wrong number of prompts", async () => {
		// A partial alignment between prompts and verdicts would score the wrong
		// pairs and look like a real result.
		const outcome = await runAgentSelectionEval(candidates, cases, scriptedJudge([]));
		expect(outcome.status).toBe("not-run");
	});
});

describe("reading the judge's answer", () => {
	const candidates: TriggerCandidate[] = [
		{ name: "explore", description: "", own: true },
		{ name: "plan", description: "", own: true },
	];

	it("maps verdicts by their stated index, not by arrival order", async () => {
		const response = '{"verdicts":[{"prompt":2,"capability":"plan"},{"prompt":0,"capability":"explore"}]}';
		expect(parseTriggerVerdicts(response, candidates, 3)).toEqual(["explore", null, "plan"]);
	});

	it("records a hallucinated capability as nothing fired", () => {
		// Keeping it wrong without letting it masquerade as a real pick.
		const response = '{"verdicts":[{"prompt":0,"capability":"invented-agent"}]}';
		expect(parseTriggerVerdicts(response, candidates, 1)).toEqual([null]);
	});

	it("returns one entry per prompt however malformed the answer", () => {
		for (const response of ["", "not json at all", '{"verdicts":"nope"}', "{}"]) {
			expect(parseTriggerVerdicts(response, candidates, 3), response).toEqual([null, null, null]);
		}
	});

	it("ignores an out-of-range index rather than shifting later verdicts", () => {
		const response = '{"verdicts":[{"prompt":9,"capability":"plan"},{"prompt":1,"capability":"explore"}]}';
		expect(parseTriggerVerdicts(response, candidates, 2)).toEqual([null, "explore"]);
	});

	it("tolerates a fenced response, since models add fences", () => {
		const response = '```json\n{"verdicts":[{"prompt":0,"capability":"plan"}]}\n```';
		expect(parseTriggerVerdicts(response, candidates, 1)).toEqual(["plan"]);
	});
});
