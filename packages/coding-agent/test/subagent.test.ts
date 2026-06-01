import { describe, expect, test } from "vitest";
import { parseAgentDefinition } from "../src/core/agent-frontmatter.js";
import { taskStore } from "../src/core/task-store.js";
import { buildTaskMainPrompt, summarizeAgentDescription } from "../src/core/tools/subagent.js";
import { EMBEDDED_AGENT_PROMPTS } from "../src/init-templates.generated.js";

describe("built-in subagent tool allowlists (frontmatter)", () => {
	function toolsFor(name: string): string[] {
		const { agent } = parseAgentDefinition(EMBEDDED_AGENT_PROMPTS[name]!, { source: "builtin", fallbackName: name });
		return agent?.tools ?? [];
	}

	test("defines the five built-in agents", () => {
		expect(Object.keys(EMBEDDED_AGENT_PROMPTS).sort()).toEqual(["doc", "edit", "explore", "review", "test"]);
	});

	test("every built-in agent declares a non-empty tool allowlist", () => {
		for (const name of Object.keys(EMBEDDED_AGENT_PROMPTS)) {
			expect(toolsFor(name).length).toBeGreaterThan(0);
		}
	});

	test("read-only agents omit edit and write", () => {
		for (const name of ["explore", "test", "review"]) {
			expect(toolsFor(name)).not.toContain("edit");
			expect(toolsFor(name)).not.toContain("write");
		}
	});

	test("edit and doc agents can write", () => {
		for (const name of ["edit", "doc"]) {
			expect(toolsFor(name)).toContain("write");
		}
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
