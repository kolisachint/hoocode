import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { parseAgentDefinition } from "../src/core/agent-frontmatter.js";
import { taskStore } from "../src/core/task-store.js";
import { buildTaskMainPrompt, resolveForkSessionFile, summarizeAgentDescription } from "../src/core/tools/subagent.js";
import { EMBEDDED_AGENT_PROMPTS } from "../src/init-templates.generated.js";

describe("built-in subagent tool allowlists (frontmatter)", () => {
	function agentFor(name: string) {
		return parseAgentDefinition(EMBEDDED_AGENT_PROMPTS[name]!, { source: "builtin", fallbackName: name }).agent;
	}
	function toolsFor(name: string): string[] {
		return agentFor(name)?.tools ?? [];
	}

	test("defines the built-in agents matching Claude Code's roster", () => {
		expect(Object.keys(EMBEDDED_AGENT_PROMPTS).sort()).toEqual(["explore", "general-purpose", "plan"]);
	});

	test("plan is a read-only research agent", () => {
		const tools = toolsFor("plan");
		expect(tools).not.toContain("edit");
		expect(tools).not.toContain("write");
		expect(tools).not.toContain("bash");
	});

	test("every built-in agent declares a non-empty tool allowlist", () => {
		for (const name of Object.keys(EMBEDDED_AGENT_PROMPTS)) {
			expect(toolsFor(name).length).toBeGreaterThan(0);
		}
	});

	test("explore is strictly read-only (no edit, write, or bash)", () => {
		const tools = toolsFor("explore");
		expect(tools).not.toContain("edit");
		expect(tools).not.toContain("write");
		expect(tools).not.toContain("bash");
	});

	test("general-purpose can write and is the delegating agent", () => {
		expect(toolsFor("general-purpose")).toContain("write");
		expect(agentFor("general-purpose")?.delegate).toBe(true);
	});
});

describe("resolveForkSessionFile", () => {
	let tmpDir: string;
	beforeEach(() => {
		tmpDir = join(tmpdir(), `fork-test-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(tmpDir, { recursive: true });
	});
	afterEach(() => {
		delete process.env.HOOCODE_SESSION_DIR;
	});

	function writeParentSession(): string {
		const path = join(tmpDir, "parent.jsonl");
		writeFileSync(
			path,
			`${JSON.stringify({ type: "session", version: 3, id: "parent", timestamp: new Date().toISOString(), cwd: tmpDir })}\n`,
		);
		return path;
	}

	test("returns undefined for a non-fork agent", () => {
		expect(resolveForkSessionFile({ fork: undefined }, writeParentSession(), tmpDir)).toBeUndefined();
	});

	test("returns undefined when there is no parent session", () => {
		expect(resolveForkSessionFile({ fork: true }, undefined, tmpDir)).toBeUndefined();
	});

	test("forks the parent session for a fork agent", () => {
		const forked = resolveForkSessionFile({ fork: true }, writeParentSession(), tmpDir);
		expect(forked).toBeTruthy();
		expect(existsSync(forked!)).toBe(true);
		expect(forked).not.toBe(join(tmpDir, "parent.jsonl"));
	});

	test("falls back to undefined when the parent session is empty/invalid", () => {
		const empty = join(tmpDir, "empty.jsonl");
		writeFileSync(empty, "");
		expect(resolveForkSessionFile({ fork: true }, empty, tmpDir)).toBeUndefined();
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

describe("buildTaskMainPrompt", () => {
	test("references the <available_agents> list instead of re-rendering the roster", () => {
		const prompt = buildTaskMainPrompt();
		// The roster is emitted once, authoritatively, in the system prompt's
		// <available_agents> block; the appendix must not duplicate it.
		expect(prompt).toContain("<available_agents>");
		expect(prompt).not.toContain("- explore: ");
	});

	test("guides the agent to delegate proactively", () => {
		const prompt = buildTaskMainPrompt();
		expect(prompt).toContain("Delegate proactively");
		expect(prompt).not.toContain("Default to handling small, quick, or single-file work inline");
	});

	test("tells the agent to mark the plan item in_progress before dispatching", () => {
		// Dispatches are attributed to the single in_progress TodoWrite item
		// (linkedTaskId); the prompt must teach the ordering that makes the link
		// land instead of leaving it to chance.
		const prompt = buildTaskMainPrompt();
		expect(prompt).toContain("mark the plan item in_progress BEFORE dispatching");
	});
});
