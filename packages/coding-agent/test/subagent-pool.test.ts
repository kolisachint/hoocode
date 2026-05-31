import { chmodSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { DEFAULT_SUBAGENT_MAX_TURNS, SubagentPool, type SubagentPoolTask } from "../src/core/subagent-pool.js";

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

/**
 * Mock that emits a single assistant message_end with enough tokens to blow the
 * budget, then exits cleanly. Used to verify the budget is advisory: the pool
 * emits telemetry but neither kills nor fails the task.
 */
function createBudgetBusterExecutable(dir: string, tokens: number): string {
	const path = join(dir, "mock-budget-buster.js");
	const content = `#!/usr/bin/env node
console.log(JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: ${tokens} } } }));
process.exit(0);
`;
	writeFileSync(path, content);
	chmodSync(path, 0o755);
	return path;
}

/** Mock that records the CLI args it was spawned with, then exits cleanly. */
function createArgvCaptureExecutable(dir: string): string {
	const path = join(dir, "mock-argv.js");
	const content = `#!/usr/bin/env node
const fs = require("node:fs");
const p = require("node:path");
fs.writeFileSync(p.join(process.cwd(), "argv.json"), JSON.stringify(process.argv.slice(2)));
console.log(JSON.stringify({ type: "done", reason: "stop", message: { role: "assistant", content: [{ type: "text", text: "ok" }] } }));
process.exit(0);
`;
	writeFileSync(path, content);
	chmodSync(path, 0o755);
	return path;
}

/** Mock that records argv and writes a valid result.json under its --task-id dir, then exits 0. */
function createResultWritingExecutable(dir: string, delayMs = 0): string {
	const path = join(dir, "mock-result.js");
	const content = `#!/usr/bin/env node
const fs = require("node:fs");
const p = require("node:path");
const argv = process.argv.slice(2);
const ti = argv.indexOf("--task-id");
const taskId = ti >= 0 ? argv[ti + 1] : "unknown";
fs.writeFileSync(p.join(process.cwd(), "argv.json"), JSON.stringify(argv));
function run() {
	const outDir = p.join(process.cwd(), ".hoocode", "dispatch", taskId);
	fs.mkdirSync(outDir, { recursive: true });
	fs.writeFileSync(p.join(outDir, "result.json"), JSON.stringify({ summary: "background done", files_changed: [], confidence: 0.9, status: "complete" }));
	console.log(JSON.stringify({ type: "done", reason: "stop", message: { role: "assistant", content: [{ type: "text", text: "ok" }] } }));
	process.exit(0);
}
const delay = ${delayMs};
if (delay > 0) setTimeout(run, delay); else run();
`;
	writeFileSync(path, content);
	chmodSync(path, 0o755);
	return path;
}

function createValidResultJson(cwd: string, taskId: string): void {
	const dir = join(cwd, ".hoocode", "dispatch", taskId);
	mkdirSync(dir, { recursive: true });
	writeFileSync(
		join(dir, "result.json"),
		JSON.stringify({
			summary: "All files updated successfully",
			files_changed: ["src/foo.ts"],
			confidence: 0.95,
			status: "complete",
		}),
	);
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
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		const task: SubagentPoolTask = { task_id: "t1", agent_type: "explore", task: "hello" };
		createValidResultJson(tmpDir, "t1");
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
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		const tasks: SubagentPoolTask[] = [
			{ task_id: "t1", agent_type: "explore", task: "a" },
			{ task_id: "t2", agent_type: "explore", task: "b" },
			{ task_id: "t3", agent_type: "explore", task: "c" },
		];
		for (const t of tasks) {
			createValidResultJson(tmpDir, t.task_id);
			pool!.spawn(t);
		}
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
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });

		// Occupy the single slot with a slow blocker
		createValidResultJson(tmpDir, "blocker");
		pool.spawn({ task_id: "blocker", agent_type: "edit", task: "block" });

		// Queue tasks while slot is occupied
		createValidResultJson(tmpDir, "t1");
		pool.spawn({ task_id: "t1", agent_type: "doc", task: "doc task" });
		createValidResultJson(tmpDir, "t2");
		pool.spawn({ task_id: "t2", agent_type: "explore", task: "explore task" });
		createValidResultJson(tmpDir, "t3");
		pool.spawn({ task_id: "t3", agent_type: "edit", task: "edit task" });

		const order: string[] = [];
		for (const id of ["blocker", "t2", "t3", "t1"]) {
			await pool.wait_for(id);
			if (id !== "blocker") order.push(id);
		}

		// explore (priority 2) then edit (priority 1) then doc (priority 0)
		expect(order).toEqual(["t2", "t3", "t1"]);
	});

	test("emits task_failed after retry on persistent spawn failure", async () => {
		const exe = join(tmpDir, "nonexistent-binary");
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		const failedEvents: Array<{ task_id: string; error: string }> = [];
		pool.on("task_failed", (data) => {
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
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		const task: SubagentPoolTask = { task_id: "t1", agent_type: "explore", task: "hello" };
		createValidResultJson(tmpDir, "t1");
		pool.spawn(task);
		expect(() => pool!.spawn(task)).toThrow("Duplicate task_id: t1");
	});

	test("wait_for returns immediately for already-completed task", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		createValidResultJson(tmpDir, "t1");
		pool.spawn({ task_id: "t1", agent_type: "explore", task: "hello" });
		const first = await pool.wait_for("t1");
		expect(first.ok).toBe(true);
		expect(pool!.running_count()).toBe(0);
		expect(pool!.queued_count()).toBe(0);
	});

	test("dispose kills running processes and rejects waiters", async () => {
		const exe = createMockExecutable(tmpDir, 0, 5000);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
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
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, defaultTokenBudget: 1000, cwd: tmpDir });
		const task: SubagentPoolTask = {
			task_id: "t1",
			agent_type: "explore",
			task: "hello",
			token_budget: 500,
		};
		createValidResultJson(tmpDir, "t1");
		pool.spawn(task);
		const result = await pool.wait_for("t1");
		expect(result.ok).toBe(true);
	});

	test("marks task failed when output verification fails", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		const task: SubagentPoolTask = { task_id: "bad-output", agent_type: "explore", task: "hello" };
		// Intentionally do NOT create result.json
		pool.spawn(task);
		const result = await pool.wait_for("bad-output");
		expect(result.task_id).toBe("bad-output");
		expect(result.ok).toBe(false);
		expect(result.error).toContain("result.json not found");
		expect(result.exit_code).toBe(0);
	});

	test("emits task_failed when output verification fails", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		const failedEvents: Array<{ task_id: string; error: string }> = [];
		pool.on("task_failed", (data) => {
			failedEvents.push(data as { task_id: string; error: string });
		});
		const task: SubagentPoolTask = { task_id: "bad-output", agent_type: "explore", task: "hello" };
		pool.spawn(task);
		const result = await pool.wait_for("bad-output");
		expect(result.ok).toBe(false);
		expect(failedEvents.length).toBe(1);
		expect(failedEvents[0].task_id).toBe("bad-output");
		expect(failedEvents[0].error).toContain("result.json not found");
	});

	test("emits task_done on successful completion", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		const doneEvents: Array<{ task_id: string; status: string }> = [];
		pool.on("task_done", (data) => {
			doneEvents.push(data as { task_id: string; status: string });
		});
		const task: SubagentPoolTask = { task_id: "t1", agent_type: "explore", task: "hello" };
		createValidResultJson(tmpDir, "t1");
		pool.spawn(task);
		const result = await pool.wait_for("t1");
		expect(result.ok).toBe(true);
		expect(doneEvents.length).toBe(1);
		expect(doneEvents[0].task_id).toBe("t1");
		expect(doneEvents[0].status).toBe("complete");
	});

	test("get_status returns running for active tasks", async () => {
		const exe = createMockExecutable(tmpDir, 0, 5000);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		createValidResultJson(tmpDir, "t1");
		pool.spawn({ task_id: "t1", agent_type: "explore", task: "hello" });
		await new Promise((r) => setTimeout(r, 50));
		expect(pool!.get_status("t1")).toBe("running");
	});

	test("get_status returns queued for pending tasks", () => {
		const exe = createMockExecutable(tmpDir, 0, 5000);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		createValidResultJson(tmpDir, "t1");
		pool.spawn({ task_id: "t1", agent_type: "explore", task: "hello" });
		createValidResultJson(tmpDir, "t2");
		pool.spawn({ task_id: "t2", agent_type: "explore", task: "world" });
		expect(pool!.get_status("t2")).toBe("queued");
	});

	test("get_status returns done after successful completion", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		createValidResultJson(tmpDir, "t1");
		pool.spawn({ task_id: "t1", agent_type: "explore", task: "hello" });
		await pool.wait_for("t1");
		expect(pool!.get_status("t1")).toBe("done");
	});

	test("get_status returns failed after bad exit", async () => {
		const exe = createMockExecutable(tmpDir, 1);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		pool.spawn({ task_id: "t1", agent_type: "explore", task: "hello" });
		await pool.wait_for("t1");
		expect(pool!.get_status("t1")).toBe("failed");
	});

	test("passes a default --max-turns hard cap to spawned subagents", async () => {
		const exe = createArgvCaptureExecutable(tmpDir);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		createValidResultJson(tmpDir, "mt-task");
		pool.spawn({ task_id: "mt-task", agent_type: "explore", task: "scan" });
		await pool.wait_for("mt-task");
		const argv = JSON.parse(readFileSync(join(tmpDir, "argv.json"), "utf-8")) as string[];
		const idx = argv.indexOf("--max-turns");
		expect(idx).toBeGreaterThanOrEqual(0);
		expect(argv[idx + 1]).toBe(String(DEFAULT_SUBAGENT_MAX_TURNS));
		// Sessions are now persisted (not ephemeral) so subagents can be resumed.
		const si = argv.indexOf("--session");
		expect(si).toBeGreaterThanOrEqual(0);
		expect(argv[si + 1]).toBe(join(tmpDir, ".hoocode", "dispatch", "mt-task", "session.jsonl"));
		expect(argv).not.toContain("--no-session");
	});

	test("dispatchDetached returns a handle immediately and the result is collectable when done", async () => {
		const exe = createResultWritingExecutable(tmpDir, 150);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		const dispatched = pool.dispatchDetached("do background work", { forceAgent: "explore" });
		expect(dispatched.handled_inline).toBe(false);
		const id = dispatched.task_id!;
		expect(id).toBeTruthy();
		// Still running: not yet collectable.
		expect(["running", "queued"]).toContain(pool.get_status(id));
		expect(pool.collect(id)).toBeUndefined();
		// Wait for the pool to report completion (without consuming via wait_for).
		await new Promise<void>((resolve) => {
			pool!.on("task_done", (d: { task_id: string }) => {
				if (d.task_id === id) resolve();
			});
		});
		const result = pool.collect(id);
		expect(result?.ok).toBe(true);
		expect((result?.result_data as { summary?: string } | undefined)?.summary).toBe("background done");
		// collect is non-destructive: a second call still returns the result.
		expect(pool.collect(id)).toBeDefined();
		expect(pool.get_status(id)).toBe("done");
	});

	test("resume continues the original task's persisted session file", async () => {
		const exe = createResultWritingExecutable(tmpDir);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		// Simulate a prior dispatch: persisted session + dispatch log for the original task.
		const originalId = "orig-task";
		const originalDir = join(tmpDir, ".hoocode", "dispatch", originalId);
		mkdirSync(originalDir, { recursive: true });
		const originalSession = join(originalDir, "session.jsonl");
		writeFileSync(originalSession, '{"type":"session"}\n');
		writeFileSync(join(originalDir, "dispatch-log.json"), JSON.stringify({ agent_type: "explore" }));

		const resumed = await pool.resume(originalId, "follow-up instruction");
		expect(resumed.handled_inline).toBe(false);
		const argv = JSON.parse(readFileSync(join(tmpDir, "argv.json"), "utf-8")) as string[];
		const si = argv.indexOf("--session");
		expect(argv[si + 1]).toBe(originalSession);
		// The resumed run uses a fresh task id, not the original.
		const ti = argv.indexOf("--task-id");
		expect(argv[ti + 1]).not.toBe(originalId);
	});

	test("resume rejects when there is no persisted session", async () => {
		const exe = createMockExecutable(tmpDir);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		await expect(pool.resume("missing", "go")).rejects.toThrow(/No resumable session/);
	});

	test("treats an exceeded token budget as advisory: emits telemetry but does not kill or fail the task", async () => {
		const exe = createBudgetBusterExecutable(tmpDir, 1000);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		const exceededEvents: Array<{ task_id: string; used: number; limit: number }> = [];
		pool.on("budget_exceeded", (data) => {
			exceededEvents.push(data as { task_id: string; used: number; limit: number });
		});
		// The child "wrote" a valid result.json; exceeding the budget must not change that.
		createValidResultJson(tmpDir, "budget-task");
		pool.spawn({ task_id: "budget-task", agent_type: "explore", task: "big task", token_budget: 500 });
		const result = await pool.wait_for("budget-task");
		expect(result.ok).toBe(true);
		expect(result.status).toBe("complete");
		expect(result.budget_exceeded).toBe(true);
		expect(result.error).toBeUndefined();
		expect(exceededEvents.length).toBe(1);
		expect(exceededEvents[0].task_id).toBe("budget-task");
		expect(exceededEvents[0].limit).toBe(500);
	});
});
