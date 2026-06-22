import { afterEach, describe, expect, it } from "vitest";
import { subagentInbox } from "../src/core/subagent-inbox.js";
import type { TaskResult } from "../src/core/subagent-pool.js";

function okResult(taskId: string, summary: string): TaskResult {
	return {
		handled_inline: false,
		task_id: taskId,
		agent_type: "explore",
		result: {
			task_id: taskId,
			ok: true,
			stdout: "",
			stderr: "",
			exit_code: 0,
			status: "complete",
			result_data: { summary, files_changed: [], confidence: 0.9, status: "complete" },
		},
	} as TaskResult;
}

function failResult(taskId: string, status: "failed" | "stalled" | "timeout", error: string): TaskResult {
	return {
		handled_inline: false,
		task_id: taskId,
		agent_type: "explore",
		result: { task_id: taskId, ok: false, stdout: "", stderr: "", exit_code: 1, status, error },
	} as TaskResult;
}

afterEach(() => subagentInbox.clear());

describe("subagent inbox", () => {
	it("allocates monotonic per-agent labels", () => {
		expect(subagentInbox.nextLabel("explore")).toBe("explore#1");
		expect(subagentInbox.nextLabel("explore")).toBe("explore#2");
		expect(subagentInbox.nextLabel("plan")).toBe("plan#1");
	});

	it("tracks a task from running to done and retains the body until collected", () => {
		subagentInbox.start("t1", "explore#1", "explore");
		const running = subagentInbox.get("t1");
		expect(running?.lifecycle).toBe("running");
		expect(subagentInbox.outstanding()).toHaveLength(1);

		subagentInbox.finish("t1", okResult("t1", "Found the bug in foo.ts:42\nmore detail"));
		const done = subagentInbox.get("t1");
		expect(done?.lifecycle).toBe("done");
		expect(done?.summaryLine).toBe("Found the bug in foo.ts:42");
		expect(subagentInbox.outstanding()).toHaveLength(0);

		// Collect returns the full body once, then drops it and marks collected.
		const collected = subagentInbox.collect("t1");
		expect(collected?.body).toContain("more detail");
		expect(subagentInbox.get("t1")?.lifecycle).toBe("collected");
		// A second collect yields nothing (body already delivered), summary survives.
		expect(subagentInbox.collect("t1")).toBeUndefined();
		expect(subagentInbox.get("t1")?.summaryLine).toBe("Found the bug in foo.ts:42");
	});

	it("resolves a handle by task id or by label", () => {
		subagentInbox.start("abc123", "plan#1", "plan");
		expect(subagentInbox.get("abc123")?.label).toBe("plan#1");
		expect(subagentInbox.get("plan#1")?.taskId).toBe("abc123");
		expect(subagentInbox.get("nope")).toBeUndefined();
	});

	it("maps failure status onto stalled/timeout/failed lifecycles", () => {
		subagentInbox.start("f1", "explore#1", "explore");
		subagentInbox.finish("f1", failResult("f1", "stalled", "no heartbeat"));
		expect(subagentInbox.get("f1")?.lifecycle).toBe("stalled");
		expect(subagentInbox.get("f1")?.error).toBe("no heartbeat");
		// Failed tasks are not collectable (no body).
		expect(subagentInbox.collect("f1")).toBeUndefined();
	});

	it("settles a task with no TaskResult via fail()", () => {
		subagentInbox.start("e1", "explore#1", "explore");
		subagentInbox.fail("e1", "dispatch threw");
		expect(subagentInbox.get("e1")?.lifecycle).toBe("failed");
		expect(subagentInbox.get("e1")?.summaryLine).toBe("dispatch threw");
	});
});
