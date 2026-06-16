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

/**
 * Mock that writes a failed result.json (carrying a provider usage-limit error
 * in its summary) under its --task-id dir, then exits 1. Used to verify the
 * pool surfaces the concrete failure reason instead of a generic message.
 */
function createFailingResultExecutable(dir: string, summary: string): string {
	const path = join(dir, "mock-failing-result.js");
	const content = `#!/usr/bin/env node
const fs = require("node:fs");
const p = require("node:path");
const argv = process.argv.slice(2);
const ti = argv.indexOf("--task-id");
const taskId = ti >= 0 ? argv[ti + 1] : "unknown";
const outDir = p.join(process.cwd(), ".hoocode", "dispatch", taskId);
fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(p.join(outDir, "result.json"), JSON.stringify({ summary: ${JSON.stringify(summary)}, files_changed: [], confidence: 0.5, status: "failed" }));
process.exit(1);
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

/** Mock that records the HOOCODE_SUBAGENT_DEPTH env it was spawned with, then exits cleanly. */
function createEnvCaptureExecutable(dir: string): string {
	const path = join(dir, "mock-env.js");
	const content = `#!/usr/bin/env node
const fs = require("node:fs");
const p = require("node:path");
fs.writeFileSync(p.join(process.cwd(), "env.json"), JSON.stringify({
	depth: process.env.HOOCODE_SUBAGENT_DEPTH ?? null,
	maxDepth: process.env.HOOCODE_SUBAGENT_MAX_DEPTH ?? null,
}));
console.log(JSON.stringify({ type: "done", reason: "stop", message: { role: "assistant", content: [{ type: "text", text: "ok" }] } }));
process.exit(0);
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

	test("removes the dispatch dir on a clean, verified success", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		const dispatchDir = join(tmpDir, ".hoocode", "dispatch", "t1");
		createValidResultJson(tmpDir, "t1");
		expect(existsSync(dispatchDir)).toBe(true);
		pool.spawn({ task_id: "t1", agent_type: "explore", task: "hello" });
		const result = await pool.wait_for("t1");
		expect(result.ok).toBe(true);
		// result_data is preserved in-memory even though the files are gone.
		expect((result.result_data as { status?: string } | undefined)?.status).toBe("complete");
		expect(existsSync(dispatchDir)).toBe(false);
	});

	test("keeps the dispatch dir on failure for debugging", async () => {
		const exe = createMockExecutable(tmpDir, 0);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		const dispatchDir = join(tmpDir, ".hoocode", "dispatch", "bad-output");
		// No result.json -> verification fails -> dir is retained with output.json.
		pool.spawn({ task_id: "bad-output", agent_type: "explore", task: "hello" });
		const result = await pool.wait_for("bad-output");
		expect(result.ok).toBe(false);
		expect(existsSync(join(dispatchDir, "output.json"))).toBe(true);
	});

	test("get_status returns failed after bad exit", async () => {
		const exe = createMockExecutable(tmpDir, 1);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 2, cwd: tmpDir });
		pool.spawn({ task_id: "t1", agent_type: "explore", task: "hello" });
		await pool.wait_for("t1");
		expect(pool!.get_status("t1")).toBe("failed");
	});

	test("surfaces the child's failure reason from result.json on a non-zero exit", async () => {
		const reason = "Task failed: Anthropic usage limit reached. Please try again later.";
		const exe = createFailingResultExecutable(tmpDir, reason);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		pool.spawn({ task_id: "quota", agent_type: "general-purpose", task: "do work" });
		const result = await pool.wait_for("quota");
		expect(result.ok).toBe(false);
		expect(result.error).toContain("usage limit reached");
		expect((result.result_data as { status?: string } | undefined)?.status).toBe("failed");
	});

	test("falls back to the stderr tail when no result.json is present", async () => {
		// createMockExecutable(exit=1) writes "mock error" to stderr and no result.json.
		const exe = createMockExecutable(tmpDir, 1);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		pool.spawn({ task_id: "stderr-only", agent_type: "explore", task: "hello" });
		const result = await pool.wait_for("stderr-only");
		expect(result.ok).toBe(false);
		expect(result.error).toContain("mock error");
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

	test("stamps a spawned child with depth 1 and propagates the tree-wide cap from the root", async () => {
		const exe = createEnvCaptureExecutable(tmpDir);
		const env = { ...process.env, HOOCODE_SUBAGENT_MAX_DEPTH: "2" };
		delete env.HOOCODE_SUBAGENT_DEPTH; // simulate a root process
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir, env });
		createValidResultJson(tmpDir, "d-root");
		pool.spawn({ task_id: "d-root", agent_type: "explore", task: "scan" });
		await pool.wait_for("d-root");
		const captured = JSON.parse(readFileSync(join(tmpDir, "env.json"), "utf-8")) as {
			depth: string | null;
			maxDepth: string | null;
		};
		expect(captured.depth).toBe("1");
		expect(captured.maxDepth).toBe("2");
	});

	test("increments depth across nesting levels (a depth-1 pool spawns a depth-2 child)", async () => {
		const exe = createEnvCaptureExecutable(tmpDir);
		const env = { ...process.env, HOOCODE_SUBAGENT_DEPTH: "1", HOOCODE_SUBAGENT_MAX_DEPTH: "2" };
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir, env });
		createValidResultJson(tmpDir, "d-nested");
		pool.spawn({ task_id: "d-nested", agent_type: "explore", task: "scan" });
		await pool.wait_for("d-nested");
		const captured = JSON.parse(readFileSync(join(tmpDir, "env.json"), "utf-8")) as {
			depth: string | null;
			maxDepth: string | null;
		};
		expect(captured.depth).toBe("2");
		expect(captured.maxDepth).toBe("2");
	});

	test("a delegate:true agent gets Task in its allowlist and subagents enabled when nesting is permitted", async () => {
		const exe = createArgvCaptureExecutable(tmpDir);
		// Project-local orchestrator agent that opts into delegation with a restricted allowlist.
		const agentsDir = join(tmpDir, ".hoocode", "agents");
		mkdirSync(agentsDir, { recursive: true });
		writeFileSync(
			join(agentsDir, "orchestrator.md"),
			`---\nname: orchestrator\ndescription: Breaks work into subtasks and delegates each.\ntools: read, grep, find, ls\ndelegate: true\n---\nDelegate subtasks via the Task tool.\n`,
		);
		// Cap raised to 2 (as --max-subagent-depth 2 would seed): the depth-1 child may still nest.
		const env = { ...process.env, HOOCODE_SUBAGENT_MAX_DEPTH: "2" };
		delete env.HOOCODE_SUBAGENT_DEPTH;
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir, env });
		createValidResultJson(tmpDir, "orch-task");
		pool.spawn({ task_id: "orch-task", agent_type: "orchestrator", task: "do a multi-part job" });
		await pool.wait_for("orch-task");
		const argv = JSON.parse(readFileSync(join(tmpDir, "argv.json"), "utf-8")) as string[];
		expect(argv).toContain("--enable-subagents");
		const ti = argv.indexOf("--tools");
		expect(ti).toBeGreaterThanOrEqual(0);
		const toolList = argv[ti + 1].split(",");
		expect(toolList).toContain("Task");
		expect(toolList).toContain("TaskOutput");
	});

	test("a delegate:true agent does NOT get delegation tools at the default cap (no nesting)", async () => {
		const exe = createArgvCaptureExecutable(tmpDir);
		const agentsDir = join(tmpDir, ".hoocode", "agents");
		mkdirSync(agentsDir, { recursive: true });
		writeFileSync(
			join(agentsDir, "orchestrator.md"),
			`---\nname: orchestrator\ndescription: Breaks work into subtasks and delegates each.\ntools: read, grep, find, ls\ndelegate: true\n---\nDelegate subtasks via the Task tool.\n`,
		);
		// Default cap (1): the spawned child is at depth 1 == cap, so it cannot nest further.
		const env = { ...process.env, HOOCODE_SUBAGENT_MAX_DEPTH: "1" };
		delete env.HOOCODE_SUBAGENT_DEPTH;
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir, env });
		createValidResultJson(tmpDir, "orch-task2");
		pool.spawn({ task_id: "orch-task2", agent_type: "orchestrator", task: "do a multi-part job" });
		await pool.wait_for("orch-task2");
		const argv = JSON.parse(readFileSync(join(tmpDir, "argv.json"), "utf-8")) as string[];
		expect(argv).not.toContain("--enable-subagents");
		const ti = argv.indexOf("--tools");
		expect(argv[ti + 1].split(",")).not.toContain("Task");
	});

	test("forwards an agent's disallowedTools denylist to the child", async () => {
		const exe = createArgvCaptureExecutable(tmpDir);
		const agentsDir = join(tmpDir, ".hoocode", "agents");
		mkdirSync(agentsDir, { recursive: true });
		writeFileSync(
			join(agentsDir, "limited.md"),
			`---\nname: limited\ndescription: restricted agent.\ntools: read, grep, find, ls, bash\ndisallowedTools: bash\n---\nbody`,
		);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		createValidResultJson(tmpDir, "lim-task");
		pool.spawn({ task_id: "lim-task", agent_type: "limited", task: "scan" });
		await pool.wait_for("lim-task");
		const argv = JSON.parse(readFileSync(join(tmpDir, "argv.json"), "utf-8")) as string[];
		const di = argv.indexOf("--disallowed-tools");
		expect(di).toBeGreaterThanOrEqual(0);
		expect(argv[di + 1]).toBe("bash");
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

	// Reach into the private lifeguard to inject a "stalled" verdict while the child
	// is still running, reproducing the race where checkHeartbeats fires SIGKILL just
	// as a healthy child is finishing.
	function injectStall(p: SubagentPool, task_id: string): void {
		const lifeguard = (
			p as unknown as { lifeguard: { emit(event: string, payload: { task_id: string; pid: number }): boolean } }
		).lifeguard;
		lifeguard.emit("stalled", { task_id, pid: 0 });
	}

	test("a late lifeguard stall does not clobber a child that already completed cleanly", async () => {
		// Child writes a valid result.json and exits 0 after a short delay.
		const exe = createResultWritingExecutable(tmpDir, 150);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		const dispatchDir = join(tmpDir, ".hoocode", "dispatch", "race");
		pool.spawn({ task_id: "race", agent_type: "explore", task: "work" });
		// Let the child start and get monitored, then fire a stall verdict mid-flight.
		await new Promise((r) => setTimeout(r, 40));
		injectStall(pool, "race");
		const result = await pool.wait_for("race");
		// The genuine completion wins over the stale stall verdict.
		expect(result.ok).toBe(true);
		expect(result.status).toBe("complete");
		expect((result.result_data as { summary?: string } | undefined)?.summary).toBe("background done");
		expect(pool.get_status("race")).toBe("done");
		// Clean-success path ran, so the dispatch dir was removed.
		expect(existsSync(dispatchDir)).toBe(false);
	});

	test("a genuine stall (no verified result) still reports stalled", async () => {
		// Child exits 0 but never writes result.json, so verification fails.
		const exe = createMockExecutable(tmpDir, 0, 150);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		const stalledEvents: Array<{ task_id: string }> = [];
		pool.on("task_stalled", (data) => stalledEvents.push(data as { task_id: string }));
		pool.spawn({ task_id: "hung", agent_type: "explore", task: "work" });
		await new Promise((r) => setTimeout(r, 40));
		injectStall(pool, "hung");
		const result = await pool.wait_for("hung");
		expect(result.ok).toBe(false);
		expect(result.status).toBe("stalled");
		expect(pool.get_status("hung")).toBe("stalled");
		expect(stalledEvents.map((e) => e.task_id)).toContain("hung");
	});
});
