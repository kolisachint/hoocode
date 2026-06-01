import { describe, expect, test } from "vitest";
import { getSubagentSystemPrompt, SUBAGENT_MODES, type SubagentMode } from "../src/core/subagent.js";
import { taskStore } from "../src/core/task-store.js";
import { buildTaskMainPrompt, summarizeAgentDescription } from "../src/core/tools/subagent.js";

describe("subagent system prompts", () => {
	test("exposes the six tool modes", () => {
		expect([...SUBAGENT_MODES]).toEqual(["explore", "edit", "test", "fix", "review", "doc"]);
	});

	test("every mode has a non-empty prompt", () => {
		for (const mode of SUBAGENT_MODES) {
			const prompt = getSubagentSystemPrompt(mode);
			expect(prompt.trim().length).toBeGreaterThan(0);
		}
	});

	test("every prompt stays well under 500 tokens", () => {
		// Rough token estimate: ~4 characters per token is a standard heuristic.
		for (const mode of SUBAGENT_MODES) {
			const prompt = getSubagentSystemPrompt(mode);
			const approxTokens = Math.ceil(prompt.length / 4);
			expect(approxTokens).toBeLessThan(500);
		}
	});

	test("throws for an unknown mode", () => {
		expect(() => getSubagentSystemPrompt("bogus" as SubagentMode)).toThrow();
	});
});

describe("task store", () => {
	test("create assigns incrementing ids and pending status", () => {
		const a = taskStore.create("first task", { subagentMode: "explore" });
		const b = taskStore.create("second task");
		expect(b.id).toBe(a.id + 1);
		expect(a.status).toBe("pending");
		expect(a.subagentMode).toBe("explore");
		expect(b.subagentMode).toBeUndefined();
	});

	test("update mutates status and notifies subscribers", () => {
		let notified = 0;
		const unsubscribe = taskStore.subscribe(() => {
			notified++;
		});
		const task = taskStore.create("work");
		taskStore.update(task.id, { status: "in_progress" });
		taskStore.update(task.id, { status: "done" });
		unsubscribe();
		const stored = taskStore.list().find((t) => t.id === task.id);
		expect(stored?.status).toBe("done");
		// One notification for create plus two for the updates.
		expect(notified).toBe(3);
	});

	test("update ignores unknown ids", () => {
		expect(() => taskStore.update(999999, { status: "done" })).not.toThrow();
	});
});

describe("summarizeAgentDescription", () => {
	const builtinStyle = [
		"Use this subagent ONLY when:",
		"- Reading or understanding code without changes",
		"- Scouting a codebase for plans or maps",
		"- Analyzing dependencies, imports, project structure",
		"",
		"DO NOT use for:",
		"- Writing or modifying code",
	].join("\n");

	test("drops the boilerplate header and surfaces the first bullets", () => {
		const summary = summarizeAgentDescription(builtinStyle);
		expect(summary).toBe(
			"Reading or understanding code without changes; Scouting a codebase for plans or maps; Analyzing dependencies, imports, project structure",
		);
		expect(summary).not.toContain("Use this subagent ONLY when");
		expect(summary).not.toContain("DO NOT");
	});

	test("keeps a plain single-line description as-is", () => {
		expect(summarizeAgentDescription("Expert TypeScript reviewer")).toBe("Expert TypeScript reviewer");
	});

	test("falls back to the first prose line when there are no bullets", () => {
		expect(summarizeAgentDescription("Reviews PRs for correctness and risk.\nMore detail here.")).toBe(
			"Reviews PRs for correctness and risk.",
		);
	});

	test("returns empty string for empty input", () => {
		expect(summarizeAgentDescription("")).toBe("");
	});
});

describe("buildTaskMainPrompt agent list", () => {
	test("renders distinct, meaningful summaries instead of a repeated header", () => {
		const prompt = buildTaskMainPrompt(process.cwd());
		// The old bug rendered every agent row as the same boilerplate header.
		expect(prompt).not.toContain(": Use this subagent ONLY when:");
		expect(prompt).toContain("- explore: ");
		const exploreLine = prompt.split("\n").find((line) => line.startsWith("- explore: "));
		expect(exploreLine).toBeDefined();
		expect(exploreLine).not.toBe("- explore: Use this subagent ONLY when:");
	});
});
