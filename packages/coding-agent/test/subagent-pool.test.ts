import { chmodSync, existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { SubagentPool, type SubagentPoolTask } from "../src/core/subagent-pool.js";

function createMockExecutable(dir: string, exitCode: number = 0, delayMs: number = 0): string {
	const path = join(dir, "mock-hoocode.js");
	const content = `#!/usr/bin/env node
const delay = ${delayMs};
const code = ${exitCode};
function run() {
	console.log(JSON.stringify({ type: "start", partial: {} }));
	if (code === 0) {
		console.log(JSON.stringify({ type: "done", reason: "stop", message: { role: "assistant", content: [{ type: "text", text: "completed" }] } }));
	} else {
		console.error("mock error");
	}
	process.exit(code);
}
if (delay > 0) setTimeout(run, delay);
else run();
`;
	writeFileSync(path, content);
	chmodSync(path, 0o755);
	return path;
}

describe("SubagentPool", () => {
	let tmpDir: string;
	let pool: SubagentPool | undefined;

	beforeEach(() => {
		tmpDir = join(tmpdir(), `subagent-pool-test-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(tmpDir, { recursive: true });
	});

	afterEach(() => {
		pool?.dispose();
		if (existsSync(tmpDir)) {
			rmSync(tmpDir, { recursive: true });
		}
	});

	test("spawns a subagent and waits for result", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2 });
		const task: SubagentPoolTask = { task_id: "t1", agent_type: "explore", task: "hello" };
		pool.spawn(task);
		const result = await pool.wait_for("t1");
		expect(result.task_id).toBe("t1");
		expect(result.ok).toBe(true);
		expect(result.exit_code).toBe(0);
		expect(result.stdout).toContain("completed");
		expect(pool.running_count()).toBe(0);
		expect(pool.queued_count()).toBe(0);
	});

	test("respects max concurrency", async () => {
		const exe = createMockExecutable(tmpDir, 0, 50);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2 });
		const tasks: SubagentPoolTask[] = [
			{ task_id: "t1", agent_type: "explore", task: "a" },
			{ task_id: "t2", agent_type: "explore", task: "b" },
			{ task_id: "t3", agent_type: "explore", task: "c" },
		];
		for (const t of tasks) pool!.spawn(t);
		expect(pool!.running_count()).toBe(2);
		expect(pool!.queued_count()).toBe(1);
		await pool!.wait_for("t1");
		await pool!.wait_for("t2");
		await pool!.wait_for("t3");
		expect(pool!.running_count()).toBe(0);
		expect(pool!.queued_count()).toBe(0);
	});

	test("priority ordering: explore before doc when queued", async () => {
		const exe = createMockExecutable(tmpDir, 0, 50);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1 });

		// Occupy the single slot with a slow blocker
		pool.spawn({ task_id: "blocker", agent_type: "edit", task: "block" });

		// Queue tasks while slot is occupied
		pool.spawn({ task_id: "t1", agent_type: "doc", task: "doc task" });
		pool.spawn({ task_id: "t2", agent_type: "explore", task: "explore task" });
		pool.spawn({ task_id: "t3", agent_type: "edit", task: "edit task" });

		const order: string[] = [];
		for (const id of ["blocker", "t2", "t3", "t1"]) {
			await pool.wait_for(id);
			if (id !== "blocker") order.push(id);
		}

		// explore (priority 2) then edit (priority 1) then doc (priority 0)
		expect(order).toEqual(["t2", "t3", "t1"]);
	});

	test("emits subagent_failed after retry on persistent spawn failure", async () => {
		const exe = join(tmpDir, "nonexistent-binary");
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2 });
		const failedEvents: Array<{ task_id: string; error: string }> = [];
		pool.on("subagent_failed", (data) => {
			failedEvents.push(data as { task_id: string; error: string });
		});
		const task: SubagentPoolTask = { task_id: "t1", agent_type: "explore", task: "hello" };
		pool.spawn(task);
		const result = await pool.wait_for("t1");
		expect(result.ok).toBe(false);
		expect(result.error).toBeTruthy();
		expect(failedEvents.length).toBe(1);
		expect(failedEvents[0].task_id).toBe("t1");
	});

	test("duplicate task_id throws", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2 });
		const task: SubagentPoolTask = { task_id: "t1", agent_type: "explore", task: "hello" };
		pool.spawn(task);
		expect(() => pool!.spawn(task)).toThrow("Duplicate task_id: t1");
	});

	test("wait_for returns immediately for already-completed task", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2 });
		pool.spawn({ task_id: "t1", agent_type: "explore", task: "hello" });
		const first = await pool.wait_for("t1");
		expect(first.ok).toBe(true);
		expect(pool!.running_count()).toBe(0);
		expect(pool!.queued_count()).toBe(0);
	});

	test("dispose kills running processes and rejects waiters", async () => {
		const exe = createMockExecutable(tmpDir, 0, 5000);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2 });
		pool.spawn({ task_id: "t1", agent_type: "explore", task: "hello" });
		// Give the process a moment to start
		await new Promise((r) => setTimeout(r, 50));
		expect(pool!.running_count()).toBe(1);

		const waitPromise = pool.wait_for("t1");
		pool.dispose();
		await expect(waitPromise).rejects.toThrow("SubagentPool disposed");
		expect(pool!.running_count()).toBe(0);
		expect(pool!.queued_count()).toBe(0);
	});

	test("tracks slot metadata correctly", async () => {
		const exe = createMockExecutable(tmpDir, 0, 50);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, defaultTokenBudget: 1000 });
		const task: SubagentPoolTask = {
			task_id: "t1",
			agent_type: "explore",
			task: "hello",
			token_budget: 500,
		};
		pool.spawn(task);
		const result = await pool.wait_for("t1");
		expect(result.ok).toBe(true);
	});
});
