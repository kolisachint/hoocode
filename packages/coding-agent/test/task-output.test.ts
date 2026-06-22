import { mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { ExtensionContext } from "../src/core/extensions/types.js";
import { subagentInbox } from "../src/core/subagent-inbox.js";
import type { TaskResult } from "../src/core/subagent-pool.js";
import { createTaskOutputToolDefinition } from "../src/core/tools/subagent.js";

const cleanups: Array<() => void> = [];

function tempCwd(): string {
	const dir = join(tmpdir(), `task-output-${Date.now()}-${Math.random().toString(36).slice(2)}`);
	mkdirSync(dir, { recursive: true });
	cleanups.push(() => rmSync(dir, { recursive: true, force: true }));
	return dir;
}

function ctx(): ExtensionContext {
	return { cwd: tempCwd() } as unknown as ExtensionContext;
}

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

async function run(args: Record<string, unknown>) {
	const tool = createTaskOutputToolDefinition();
	const result = await tool.execute("tc", args as never, undefined, undefined, ctx());
	return { text: (result.content[0] as { text: string }).text, details: result.details };
}

afterEach(() => {
	subagentInbox.clear();
	while (cleanups.length > 0) cleanups.pop()?.();
});

describe("TaskOutput tool", () => {
	it("lists all background subagents with status, without bodies", async () => {
		subagentInbox.start("t1", "explore#1", "explore");
		subagentInbox.start("t2", "explore#2", "explore");
		subagentInbox.finish("t2", okResult("t2", "Two: headline\nlong body that must not appear in the roster listing"));

		const { text, details } = await run({ list: true });
		expect(text).toContain("explore#1");
		expect(text).toContain("running");
		expect(text).toContain("explore#2");
		expect(text).toContain("done");
		// The roster shows only the one-line summary, never the full body.
		expect(text).toContain("Two: headline");
		expect(text).not.toContain("must not appear in the roster");
		expect(details).toMatchObject({ status: "list", ok: true, outstanding: 1 });
	});

	it("returns the body once for a finished task, then reports it as already delivered", async () => {
		subagentInbox.start("t1", "explore#1", "explore");
		subagentInbox.finish("t1", okResult("t1", "the full result body"));

		const first = await run({ task_id: "explore#1" });
		expect(first.text).toBe("the full result body");
		expect(first.details).toMatchObject({ status: "done", ok: true });

		const second = await run({ task_id: "explore#1" });
		expect(second.text).toContain("already delivered");
		expect(second.details).toMatchObject({ status: "collected", ok: true });
	});

	it("reports status (not an error) for a still-running task", async () => {
		subagentInbox.start("t1", "explore#1", "explore");
		const { text, details } = await run({ task_id: "explore#1" });
		expect(text).toContain("still running");
		expect(details).toMatchObject({ status: "running", ok: true });
	});

	it("never throws on an unknown handle", async () => {
		const { text, details } = await run({ task_id: "nope#9" });
		expect(text).toContain("No background task");
		expect(details).toMatchObject({ status: "unknown", ok: false });
	});

	it("reports a failed task without throwing", async () => {
		subagentInbox.start("t1", "explore#1", "explore");
		subagentInbox.fail("t1", "no heartbeat", "stalled");
		const { text, details } = await run({ task_id: "explore#1" });
		expect(text).toContain("stalled");
		expect(text).toContain("no heartbeat");
		expect(details).toMatchObject({ status: "stalled", ok: false });
	});

	it("wait:true blocks until the task finishes, then returns its body", async () => {
		subagentInbox.start("t1", "explore#1", "explore");
		setTimeout(() => subagentInbox.finish("t1", okResult("t1", "finished after waiting")), 10);
		const { text, details } = await run({ task_id: "explore#1", wait: true, timeout_ms: 2000 });
		expect(text).toBe("finished after waiting");
		expect(details).toMatchObject({ status: "done", ok: true });
	});

	it("wait:true with no task_id is a barrier for all outstanding tasks", async () => {
		subagentInbox.start("t1", "explore#1", "explore");
		subagentInbox.start("t2", "explore#2", "explore");
		setTimeout(() => subagentInbox.finish("t1", okResult("t1", "one")), 5);
		setTimeout(() => subagentInbox.finish("t2", okResult("t2", "two")), 15);
		const { details } = await run({ wait: true, timeout_ms: 2000 });
		expect(details).toMatchObject({ status: "list", outstanding: 0 });
	});
});
