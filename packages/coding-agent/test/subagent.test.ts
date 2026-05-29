import { describe, expect, test } from "vitest";
import { getSubagentSystemPrompt, SUBAGENT_MODES, type SubagentMode } from "../src/core/subagent.js";
import { taskStore } from "../src/core/task-store.js";

describe("subagent system prompts", () => {
	test("exposes the five tool modes", () => {
		expect([...SUBAGENT_MODES]).toEqual(["explore", "edit", "test", "fix", "review"]);
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
