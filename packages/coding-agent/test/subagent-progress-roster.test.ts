import { afterEach, describe, expect, it } from "vitest";
import { disposeSubagentPool, getSubagentPool, setSubagentPoolForTesting } from "../src/core/subagent-pool-instance.js";
import { taskStore } from "../src/core/task-store.js";

/**
 * Verifies the pool-creation wiring that maps `task_progress` events onto the
 * task panel's agent roster row (the consumer half of live subagent progress).
 * Roster rows are keyed per run by the pool task id — not by agent type — so
 * concurrent same-type subagents each drive their own row. Emitting synthetic
 * pool events exercises the listener without spawning a child.
 */
describe("subagent task_progress → roster activity", () => {
	afterEach(() => {
		disposeSubagentPool();
		setSubagentPoolForTesting(undefined);
		taskStore.clear();
	});

	const activityOf = (id: string): string | undefined => taskStore.agents().find((a) => a.id === id)?.activity;

	it("sets the current tool on tool_execution_start and clears it on tool_execution_end", () => {
		taskStore.upsertAgent({ id: "run-1", name: "explore#1", kind: "subagent", state: "running" });
		const pool = getSubagentPool(process.cwd());

		pool.emit("task_progress", {
			task_id: "run-1",
			agent_type: "explore",
			event: { type: "tool_execution_start", toolName: "grep" },
		});
		expect(activityOf("run-1")).toBe("grep");

		pool.emit("task_progress", {
			task_id: "run-1",
			agent_type: "explore",
			event: { type: "tool_execution_end", toolName: "grep" },
		});
		expect(activityOf("run-1")).toBe("");
	});

	it("shows thinking between turns (matching the inbox) and clears on terminal pool events", () => {
		taskStore.upsertAgent({ id: "run-1", name: "explore#1", kind: "subagent", state: "running" });
		const pool = getSubagentPool(process.cwd());

		pool.emit("task_progress", {
			task_id: "run-1",
			agent_type: "explore",
			event: { type: "tool_execution_start", toolName: "bash" },
		});
		expect(activityOf("run-1")).toBe("bash");
		pool.emit("task_progress", { task_id: "run-1", agent_type: "explore", event: { type: "turn_end" } });
		// turn_end means the model is reasoning, not idle — same word the inbox uses.
		expect(activityOf("run-1")).toBe("thinking");

		pool.emit("task_progress", {
			task_id: "run-1",
			agent_type: "explore",
			event: { type: "tool_execution_start", toolName: "read" },
		});
		expect(activityOf("run-1")).toBe("read");
		pool.emit("task_done", { agent_type: "explore", task_id: "run-1" });
		expect(activityOf("run-1")).toBe("");
	});

	it("keeps concurrent same-type runs on separate rows (no last-writer-wins collision)", () => {
		taskStore.upsertAgent({ id: "run-1", name: "explore#1", kind: "subagent", state: "running" });
		taskStore.upsertAgent({ id: "run-2", name: "explore#2", kind: "subagent", state: "running" });
		const pool = getSubagentPool(process.cwd());

		pool.emit("task_progress", {
			task_id: "run-1",
			agent_type: "explore",
			event: { type: "tool_execution_start", toolName: "grep" },
		});
		pool.emit("task_progress", {
			task_id: "run-2",
			agent_type: "explore",
			event: { type: "tool_execution_start", toolName: "bash" },
		});
		expect(activityOf("run-1")).toBe("grep");
		expect(activityOf("run-2")).toBe("bash");

		// One run finishing must not clear its sibling's activity.
		pool.emit("task_done", { agent_type: "explore", task_id: "run-2" });
		expect(activityOf("run-1")).toBe("grep");
		expect(activityOf("run-2")).toBe("");
	});

	it("is a no-op for a run with no roster row", () => {
		const pool = getSubagentPool(process.cwd());
		pool.emit("task_progress", {
			task_id: "ghost-run",
			agent_type: "ghost",
			event: { type: "tool_execution_start", toolName: "grep" },
		});
		expect(taskStore.agents().find((a) => a.id === "ghost-run")).toBeUndefined();
	});
});
