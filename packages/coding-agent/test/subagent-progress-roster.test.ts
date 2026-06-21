import { afterEach, describe, expect, it } from "vitest";
import { disposeSubagentPool, getSubagentPool, setSubagentPoolForTesting } from "../src/core/subagent-pool-instance.js";
import { taskStore } from "../src/core/task-store.js";

/**
 * Verifies the pool-creation wiring that maps `task_progress` events onto the
 * task panel's agent roster row (the consumer half of live subagent progress).
 * Emitting synthetic pool events exercises the listener without spawning a child.
 */
describe("subagent task_progress → roster activity", () => {
	afterEach(() => {
		disposeSubagentPool();
		setSubagentPoolForTesting(undefined);
		taskStore.reset();
	});

	const activityOf = (id: string): string | undefined => taskStore.agents().find((a) => a.id === id)?.activity;

	it("sets the current tool on tool_execution_start and clears it on tool_execution_end", () => {
		taskStore.upsertAgent({ id: "explore", name: "explore", kind: "subagent", state: "running" });
		const pool = getSubagentPool(process.cwd());

		pool.emit("task_progress", { agent_type: "explore", event: { type: "tool_execution_start", toolName: "grep" } });
		expect(activityOf("explore")).toBe("grep");

		pool.emit("task_progress", { agent_type: "explore", event: { type: "tool_execution_end", toolName: "grep" } });
		expect(activityOf("explore")).toBe("");
	});

	it("clears activity between turns and on terminal pool events", () => {
		taskStore.upsertAgent({ id: "explore", name: "explore", kind: "subagent", state: "running" });
		const pool = getSubagentPool(process.cwd());

		pool.emit("task_progress", { agent_type: "explore", event: { type: "tool_execution_start", toolName: "bash" } });
		expect(activityOf("explore")).toBe("bash");
		pool.emit("task_progress", { agent_type: "explore", event: { type: "turn_end" } });
		expect(activityOf("explore")).toBe("");

		pool.emit("task_progress", { agent_type: "explore", event: { type: "tool_execution_start", toolName: "read" } });
		expect(activityOf("explore")).toBe("read");
		pool.emit("task_done", { agent_type: "explore", task_id: "t1" });
		expect(activityOf("explore")).toBe("");
	});

	it("is a no-op for an agent with no roster row", () => {
		const pool = getSubagentPool(process.cwd());
		pool.emit("task_progress", { agent_type: "ghost", event: { type: "tool_execution_start", toolName: "grep" } });
		expect(taskStore.agents().find((a) => a.id === "ghost")).toBeUndefined();
	});
});
