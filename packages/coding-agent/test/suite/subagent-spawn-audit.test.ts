/**
 * End-to-end coverage for the subagent spawn-path audit fixes:
 *
 * - per-run roster identity (concurrent same-type runs get their own rows)
 * - wall-clock elapsed for in-progress tasks
 * - subagents-lens orphan rendering + cycle guard (count always matches rows)
 * - clearable task notes (⚠ cue does not stick)
 * - lifeguard stall de-duplication (one event per reap, not one per tick)
 * - single UTF-8-safe stdout parser with bounded buffers (jsonl reader caps,
 *   error/close handling; TokenBudget fed per-line)
 * - retry lifecycle keeping the cumulative TokenBudget across attempts
 * - process-group kill reaping a subagent's grandchildren
 * - atomic result.json/output.json writes (temp + rename)
 *
 * Everything runs against local fakes/mock executables — no real APIs.
 */

import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { chmodSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough } from "node:stream";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { SubagentLifeguard } from "../../src/core/lifeguard.js";
import { subagentInbox } from "../../src/core/subagent-inbox.js";
import { SubagentPool as RealSubagentPool, type SubagentPool, type TaskResult } from "../../src/core/subagent-pool.js";
import { setSubagentPoolForTesting } from "../../src/core/subagent-pool-instance.js";
import type { SubagentResultFile } from "../../src/core/subagent-result.js";
import { writeSubagentResult } from "../../src/core/subagent-result.js";
import { taskStore } from "../../src/core/task-store.js";
import { TokenBudget } from "../../src/core/token-budget.js";
import { createTaskToolDefinition } from "../../src/core/tools/subagent.js";
import { TaskPanelComponent } from "../../src/modes/interactive/components/task-panel.js";
import { initTheme } from "../../src/modes/interactive/theme/theme.js";
import { attachJsonlLineReader } from "../../src/modes/rpc/jsonl.js";
import { writeFileAtomicSync } from "../../src/utils/atomic-file.js";

const cleanups: Array<() => void> = [];

function makeTempDir(): string {
	const dir = join(tmpdir(), `pi-audit-${Date.now()}-${Math.random().toString(36).slice(2)}`);
	mkdirSync(dir, { recursive: true });
	cleanups.push(() => rmSync(dir, { recursive: true, force: true }));
	return dir;
}

const stripAnsi = (s: string): string => s.replace(/\x1b\[[0-9;]*m/g, "");

afterEach(() => {
	vi.useRealTimers();
	setSubagentPoolForTesting(undefined);
	subagentInbox.clear();
	taskStore.clear();
	while (cleanups.length > 0) {
		cleanups.pop()?.();
	}
});

// ---------------------------------------------------------------------------
// Per-run roster identity (Task tool + task store)
// ---------------------------------------------------------------------------

describe("per-run roster identity", () => {
	function fakeSuccess(taskId: string): TaskResult {
		const data: SubagentResultFile = { summary: "done", files_changed: [], confidence: 0.9, status: "complete" };
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
				result_data: data as unknown as Record<string, unknown>,
			},
		};
	}

	/** A fake pool whose dispatches stay pending until the test resolves them. */
	function makeDeferredPool() {
		const pending = new Map<string, (result: TaskResult) => void>();
		const pool = new EventEmitter() as EventEmitter & {
			dispatch: (prompt: string, options: { taskId?: string }) => Promise<TaskResult>;
		};
		pool.dispatch = (_prompt, options) =>
			new Promise<TaskResult>((resolve) => {
				pending.set(options.taskId ?? "unknown", resolve);
			});
		return { pool: pool as unknown as SubagentPool, pending };
	}

	it("gives concurrent same-type dispatches their own roster rows and settles them independently", async () => {
		const { pool, pending } = makeDeferredPool();
		setSubagentPoolForTesting(pool);
		const cwd = makeTempDir();
		const ctx = { cwd, hasUI: true } as never;
		const tool = createTaskToolDefinition();

		// Two concurrent background explores (explore is a background agent).
		const first = tool.execute(
			"c1",
			{ description: "scan a", prompt: "scan module a", subagent_type: "explore" },
			undefined,
			undefined,
			ctx,
		);
		const second = tool.execute(
			"c2",
			{ description: "scan b", prompt: "scan module b", subagent_type: "explore" },
			undefined,
			undefined,
			ctx,
		);
		// Let both executes run up to their awaited dispatch.
		await new Promise((r) => setImmediate(r));

		const runs = taskStore.agents().filter((a) => a.kind === "subagent");
		expect(runs).toHaveLength(2);
		expect(runs[0].id).not.toBe(runs[1].id);
		// Rows are labeled like the inbox ("explore#N"), not keyed by bare type.
		expect(runs.map((a) => a.name).sort()).toEqual(["explore#1", "explore#2"]);
		expect(runs.every((a) => a.state === "running")).toBe(true);
		// Each task row points at its own run, so activity/state cannot collide.
		const tasks = taskStore.list();
		expect(tasks.map((t) => t.agent).sort()).toEqual(runs.map((a) => a.id).sort());

		// Finish only the first run: its row settles, the sibling stays running.
		const [firstId] = [...pending.keys()];
		pending.get(firstId)?.(fakeSuccess(firstId));
		await first;
		const firstRow = taskStore.agents().find((a) => a.id === firstId);
		const otherRow = taskStore.agents().find((a) => a.kind === "subagent" && a.id !== firstId);
		expect(firstRow?.state).toBe("done");
		expect(otherRow?.state).toBe("running");

		const secondId = [...pending.keys()].find((id) => id !== firstId) as string;
		pending.get(secondId)?.(fakeSuccess(secondId));
		await second;
		expect(taskStore.agents().find((a) => a.id === secondId)?.state).toBe("done");
	});

	it("attributes per-run usage to the run's own row, not a shared type row", async () => {
		const { pool, pending } = makeDeferredPool();
		setSubagentPoolForTesting(pool);
		const ctx = { cwd: makeTempDir(), hasUI: true } as never;
		const tool = createTaskToolDefinition();

		const call = tool.execute(
			"c1",
			{ description: "scan", prompt: "scan the repo", subagent_type: "explore" },
			undefined,
			undefined,
			ctx,
		);
		await new Promise((r) => setImmediate(r));
		const [runId] = [...pending.keys()];
		const result = fakeSuccess(runId);
		(result.result?.result_data as unknown as SubagentResultFile).usage = {
			input: 1200,
			output: 300,
			cacheRead: 0,
			cacheWrite: 0,
			cost: 0.01,
		};
		pending.get(runId)?.(result);
		await call;

		const row = taskStore.agents().find((a) => a.id === runId);
		expect(row?.stats).toEqual({ input: 1200, output: 300, cost: 0.01 });
		// No type-keyed row exists to swallow the stats.
		expect(taskStore.agents().find((a) => a.id === "explore")).toBeUndefined();
	});
});

// ---------------------------------------------------------------------------
// Task panel: elapsed, orphans, cycles, notes
// ---------------------------------------------------------------------------

describe("task panel audit fixes", () => {
	beforeAll(() => {
		initTheme("dark");
	});

	it("advances a running task's elapsed time from the wall clock", () => {
		vi.useFakeTimers();
		const panel = new TaskPanelComponent();
		const task = taskStore.create("long build");
		taskStore.update(task.id, {
			status: "in_progress",
			usage: { input: 1000, output: 100, cacheRead: 0, cacheWrite: 0, cost: 0 },
		});

		vi.advanceTimersByTime(90_000);
		const header = stripAnsi(panel.render(120)[0] as string);
		// updatedAt is still ≈ createdAt, but the task is running: elapsed is
		// measured against the clock, not frozen at ~0s.
		expect(header).toContain("1m30s");
		panel.dispose();
	});

	it("renders orphaned children as roots so the header count matches the rows", () => {
		const panel = new TaskPanelComponent();
		panel.setView("subagents");
		const parent = taskStore.create("parent run", { source: "subagent", subagentMode: "explore" });
		const child = taskStore.create("orphaned child", {
			source: "subagent",
			subagentMode: "review",
			parentTaskId: parent.id,
		});
		taskStore.update(child.id, { status: "done" });
		taskStore.remove(parent.id);

		const lines = panel.render(120).map(stripAnsi);
		// The orphan is both counted and rendered (previously: counted, never drawn).
		expect(lines.some((l) => l.includes("orphaned child"))).toBe(true);
		expect(lines[0]).toContain("1/1");
		panel.dispose();
	});

	it("terminates on a parentTaskId cycle instead of walking forever", () => {
		const panel = new TaskPanelComponent();
		panel.setView("subagents");
		const a = taskStore.create("node a", { source: "subagent", subagentMode: "explore" });
		const b = taskStore.create("node b", { source: "subagent", subagentMode: "explore", parentTaskId: a.id });
		taskStore.update(a.id, { parentTaskId: b.id }); // a ⇄ b cycle, no roots

		// Must return promptly; cycle members have no root so they drop from the
		// lens — and the header agrees (nothing counted, nothing rendered).
		const lines = panel.render(120).map(stripAnsi);
		expect(lines.some((l) => l.includes("node a") || l.includes("node b"))).toBe(false);
		panel.dispose();
	});

	it("clears a task note when the next update omits it explicitly", () => {
		const panel = new TaskPanelComponent();
		const task = taskStore.create("retry-prone work", { source: "subagent", subagentMode: "explore" });
		taskStore.update(task.id, { status: "in_progress", note: "ran on inherited model" });
		panel.setView("subagents");
		expect(stripAnsi(panel.render(120).join("\n"))).toContain("⚠︎ ran on inherited model");

		// The finishing update passes note: undefined (as finalizeDispatchResult
		// does when there is no warning) — the sticky ⚠ cue must clear.
		taskStore.update(task.id, { status: "done", note: undefined });
		const text = stripAnsi(panel.render(120).join("\n"));
		expect(text).not.toContain("⚠");
		panel.dispose();
	});

	it("re-renders memoized rows after a store mutation (cache invalidates)", () => {
		const panel = new TaskPanelComponent();
		const task = taskStore.create("stable row");
		taskStore.update(task.id, { status: "done" });
		const before = panel.render(120).map(stripAnsi).join("\n");
		expect(before).toContain("stable row");

		taskStore.update(task.id, { title: "renamed row" });
		const after = panel.render(120).map(stripAnsi).join("\n");
		expect(after).toContain("renamed row");
		expect(after).not.toContain("stable row");
		panel.dispose();
	});
});

// ---------------------------------------------------------------------------
// Lifeguard: stall de-duplication
// ---------------------------------------------------------------------------

describe("lifeguard stall de-duplication", () => {
	it("emits stalled once per reap even when the check ticks again before exit", () => {
		const cwd = makeTempDir();
		const guard = new SubagentLifeguard(cwd);
		cleanups.push(() => guard.dispose());
		const script = join(cwd, "silent.js");
		writeFileSync(script, "setTimeout(() => {}, 10000)\n");
		const proc = spawn(process.execPath, [script], { detached: true });
		cleanups.push(() => {
			try {
				proc.kill("SIGKILL");
			} catch {}
		});

		const stalled: string[] = [];
		guard.on("stalled", (data) => stalled.push((data as { task_id: string }).task_id));

		guard.monitor("t1", "explore", proc);
		// @ts-expect-error – internal map access for the test
		guard.lastHeartbeat.set("t1", Date.now() - 70_000);

		// The heartbeat check fires every 5s; before the fix each tick re-emitted
		// "stalled" (and re-killed) until the process exit was observed.
		// @ts-expect-error – internal method access for the test
		guard.checkHeartbeats();
		// @ts-expect-error – internal method access for the test
		guard.checkHeartbeats();
		// @ts-expect-error – internal method access for the test
		guard.checkHeartbeats();

		expect(stalled).toEqual(["t1"]);
	});
});

// ---------------------------------------------------------------------------
// JSONL reader: bounded buffer, error/close handling
// ---------------------------------------------------------------------------

describe("jsonl line reader hardening", () => {
	it("drops an oversized un-terminated line instead of buffering it forever", () => {
		const stream = new PassThrough();
		const lines: string[] = [];
		attachJsonlLineReader(stream, (line) => lines.push(line), { maxBuffer: 16 });

		stream.write("x".repeat(64)); // grows past the cap with no newline
		stream.write("still the same giant line");
		stream.write("\n"); // terminates the dropped line
		stream.write('{"ok":true}\n');

		expect(lines).toEqual(['{"ok":true}']);
	});

	it("survives a stream error without throwing and detaches itself", () => {
		const stream = new PassThrough();
		const lines: string[] = [];
		attachJsonlLineReader(stream, (line) => lines.push(line));

		stream.write("partial");
		// With no 'error' listener this would crash the process.
		expect(() => stream.emit("error", new Error("pipe broke"))).not.toThrow();
		expect(stream.listenerCount("data")).toBe(0);
		expect(lines).toEqual([]);
	});

	it("flushes the final partial line when the stream closes", async () => {
		const stream = new PassThrough();
		const lines: string[] = [];
		attachJsonlLineReader(stream, (line) => lines.push(line));

		stream.write('{"a":1}\n{"b":2}');
		stream.end();
		await new Promise((r) => setImmediate(r));
		expect(lines).toEqual(['{"a":1}', '{"b":2}']);
	});

	it("reassembles multi-byte characters split across chunks", () => {
		const stream = new PassThrough();
		const lines: string[] = [];
		attachJsonlLineReader(stream, (line) => lines.push(line));

		const payload = Buffer.from('{"emoji":"🚀"}\n', "utf8");
		// Split inside the 4-byte emoji sequence.
		const mid = payload.indexOf(0x9a);
		stream.write(payload.subarray(0, mid));
		stream.write(payload.subarray(mid));
		expect(lines).toEqual(['{"emoji":"🚀"}']);
	});

	it("feeds TokenBudget per-line without a second chunk parser", () => {
		const budget = new TokenBudget("t1", "explore", { limit: 1000, cwd: makeTempDir() });
		budget.processLine(
			JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 400 } } }),
		);
		budget.processLine(
			JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 500 } } }),
		);
		expect(budget.getUsed()).toBe(900);
		expect(budget.isWarned()).toBe(true);
		expect(budget.isExceeded()).toBe(false);
	});
});

// ---------------------------------------------------------------------------
// Atomic writes
// ---------------------------------------------------------------------------

describe("atomic result writes", () => {
	it("writeFileAtomicSync leaves the target parseable and no temp litter", () => {
		const dir = makeTempDir();
		const path = join(dir, "nested", "result.json");
		writeFileAtomicSync(path, JSON.stringify({ v: 1 }));
		writeFileAtomicSync(path, JSON.stringify({ v: 2 }));
		expect(JSON.parse(readFileSync(path, "utf-8"))).toEqual({ v: 2 });
		expect(readdirSync(join(dir, "nested"))).toEqual(["result.json"]);
	});

	it("writeSubagentResult persists atomically under the dispatch dir", () => {
		const cwd = makeTempDir();
		writeSubagentResult(cwd, "task-1", {
			summary: "done",
			files_changed: [],
			confidence: 0.9,
			status: "complete",
		});
		const dir = join(cwd, ".hoocode", "dispatch", "task-1");
		expect(readdirSync(dir)).toEqual(["result.json"]);
		expect(JSON.parse(readFileSync(join(dir, "result.json"), "utf-8")).summary).toBe("done");
	});
});

// ---------------------------------------------------------------------------
// Pool: retry keeps the cumulative budget; process-group kill reaps grandchildren
// ---------------------------------------------------------------------------

describe("subagent pool reliability", () => {
	/**
	 * Mock hoocode that fails on its pinned model (quota-style error, emitting
	 * usage first) and succeeds on the inherited model, so the pool's
	 * inherited-model retry path runs. Each attempt reports 300 tokens.
	 */
	function createQuotaThenSuccessExecutable(dir: string): string {
		const path = join(dir, "mock-quota-retry.js");
		const content = `#!/usr/bin/env node
const fs = require("node:fs");
const p = require("node:path");
const argv = process.argv.slice(2);
const taskId = argv[argv.indexOf("--task-id") + 1];
const model = argv[argv.indexOf("--model") + 1];
const outDir = p.join(process.cwd(), ".hoocode", "dispatch", taskId);
fs.mkdirSync(outDir, { recursive: true });
console.log(JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 300 } } }));
if (model === "pinned-model") {
	fs.writeFileSync(p.join(outDir, "result.json"), JSON.stringify({ summary: "Task failed: usage limit reached for pinned-model.", files_changed: [], confidence: 0.5, status: "failed" }));
	process.exit(1);
}
fs.writeFileSync(p.join(outDir, "result.json"), JSON.stringify({ summary: "recovered on inherited model", files_changed: [], confidence: 0.9, status: "complete" }));
process.exit(0);
`;
		writeFileSync(path, content);
		chmodSync(path, 0o755);
		return path;
	}

	it("keeps one cumulative TokenBudget (and its listeners) across the inherited-model retry", async () => {
		const cwd = makeTempDir();
		const agentsDir = join(cwd, ".hoocode", "agents");
		mkdirSync(agentsDir, { recursive: true });
		writeFileSync(
			join(agentsDir, "pinned.md"),
			`---\nname: pinned\ndescription: agent with a pinned model.\nmodel: pinned-model\n---\nbody`,
		);
		const exe = createQuotaThenSuccessExecutable(cwd);
		const pool = new RealSubagentPool({ executable: exe, maxConcurrency: 1, cwd });
		cleanups.push(() => pool.dispose());

		const doneEvents: Array<{ tokens_used: number }> = [];
		const exceeded: Array<{ used: number }> = [];
		pool.on("task_done", (d) => doneEvents.push(d as { tokens_used: number }));
		pool.on("budget_exceeded", (d) => exceeded.push(d as { used: number }));

		pool.spawn({
			task_id: "retry-budget",
			agent_type: "pinned",
			task: "do work",
			model: "parent-model",
			token_budget: 500,
		});
		const result = await pool.wait_for("retry-budget");

		expect(result.ok).toBe(true);
		expect(result.usedInheritedModelFallback).toBe(true);
		// 300 tokens from the failed attempt + 300 from the retry: the budget was
		// reused, not recreated (the old .finally tore it down mid-retry, so the
		// retry restarted from zero and its threshold events went nowhere).
		expect(doneEvents[0]?.tokens_used).toBe(600);
		// The 500-token cap is only crossed cumulatively — this event firing proves
		// the budget's listeners survived the retry too.
		expect(exceeded).toHaveLength(1);
		expect(exceeded[0]?.used).toBe(600);
	});

	it.skipIf(process.platform === "win32")(
		"dispose kills a subagent's grandchildren via the process group",
		async () => {
			const cwd = makeTempDir();
			const exe = join(cwd, "mock-nested.js");
			writeFileSync(
				exe,
				`#!/usr/bin/env node
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const p = require("node:path");
const grandchild = spawn(process.execPath, ["-e", "setTimeout(() => {}, 30000)"], { stdio: "ignore" });
fs.writeFileSync(p.join(process.cwd(), "grandchild.pid"), String(grandchild.pid));
setTimeout(() => {}, 30000);
`,
			);
			chmodSync(exe, 0o755);
			const pool = new RealSubagentPool({ executable: exe, maxConcurrency: 1, cwd });
			cleanups.push(() => pool.dispose());
			pool.spawn({ task_id: "nested", agent_type: "explore", task: "spawn a grandchild" });

			// Wait for the grandchild to exist.
			const pidFile = join(cwd, "grandchild.pid");
			for (let i = 0; i < 100 && !existsSync(pidFile); i++) {
				await new Promise((r) => setTimeout(r, 20));
			}
			expect(existsSync(pidFile)).toBe(true);
			const grandchildPid = Number.parseInt(readFileSync(pidFile, "utf-8"), 10);
			expect(Number.isInteger(grandchildPid) && grandchildPid > 0).toBe(true);
			expect(() => process.kill(grandchildPid, 0)).not.toThrow();

			pool.dispose();

			// The group kill must reach the grandchild, not just the direct child.
			let dead = false;
			for (let i = 0; i < 100 && !dead; i++) {
				try {
					process.kill(grandchildPid, 0);
					await new Promise((r) => setTimeout(r, 20));
				} catch {
					dead = true;
				}
			}
			expect(dead).toBe(true);
		},
	);
});

// ---------------------------------------------------------------------------
// User-initiated cancellation
// ---------------------------------------------------------------------------

describe("subagent cancellation", () => {
	/** Mock hoocode that just sleeps, so a cancel is the only way it settles. */
	function createSleepingExecutable(dir: string): string {
		const path = join(dir, "mock-sleep.js");
		writeFileSync(path, "#!/usr/bin/env node\nsetTimeout(() => {}, 30000);\n");
		chmodSync(path, 0o755);
		return path;
	}

	it("cancel() on a running child settles it as cancelled (not failed/stalled)", async () => {
		const cwd = makeTempDir();
		const pool = new RealSubagentPool({ executable: createSleepingExecutable(cwd), maxConcurrency: 1, cwd });
		cleanups.push(() => pool.dispose());
		const cancelledEvents: string[] = [];
		pool.on("task_cancelled", (d: { task_id: string }) => cancelledEvents.push(d.task_id));

		pool.spawn({ task_id: "c1", agent_type: "explore", task: "work" });
		await new Promise((r) => setTimeout(r, 80));
		expect(pool.cancel("c1")).toBe(true);

		const result = await pool.wait_for("c1");
		expect(result.ok).toBe(false);
		expect(result.status).toBe("cancelled");
		expect(pool.get_status("c1")).toBe("cancelled");
		expect(cancelledEvents).toEqual(["c1"]);
		expect(pool.running_count()).toBe(0);
	});

	it("cancel() on a queued task settles it immediately without spawning", async () => {
		const cwd = makeTempDir();
		const pool = new RealSubagentPool({ executable: createSleepingExecutable(cwd), maxConcurrency: 1, cwd });
		cleanups.push(() => pool.dispose());

		pool.spawn({ task_id: "blocker", agent_type: "explore", task: "block" });
		pool.spawn({ task_id: "queued", agent_type: "explore", task: "wait" });
		expect(pool.queued_count()).toBe(1);

		expect(pool.cancel("queued")).toBe(true);
		expect(pool.queued_count()).toBe(0);
		const result = await pool.wait_for("queued");
		expect(result.status).toBe("cancelled");
		expect(pool.get_status("queued")).toBe("cancelled");
		// Unknown/settled ids report false rather than pretending to cancel.
		expect(pool.cancel("queued")).toBe(false);
		expect(pool.cancel("nope")).toBe(false);
	});

	it("Task tool abort marks the run cancelled across store, roster, and inbox", async () => {
		// Fake pool whose dispatch hangs until cancel(id) settles it as cancelled —
		// the same contract the real pool implements.
		const pending = new Map<string, (r: TaskResult) => void>();
		const fake = new EventEmitter() as EventEmitter & {
			dispatch: (prompt: string, options: { taskId?: string }) => Promise<TaskResult>;
			cancel: (id: string) => boolean;
		};
		fake.dispatch = (_p, options) =>
			new Promise<TaskResult>((resolve) => {
				pending.set(options.taskId ?? "t", resolve);
			});
		fake.cancel = (id: string) => {
			const resolve = pending.get(id);
			if (!resolve) return false;
			resolve({
				handled_inline: false,
				task_id: id,
				agent_type: "general-purpose",
				result: {
					task_id: id,
					ok: false,
					stdout: "",
					stderr: "",
					exit_code: null,
					error: "cancelled",
					status: "cancelled",
				},
			});
			return true;
		};
		setSubagentPoolForTesting(fake as unknown as SubagentPool);

		const controller = new AbortController();
		const tool = createTaskToolDefinition();
		// general-purpose runs foreground, so the abort must settle the await.
		const call = tool.execute(
			"c1",
			{ description: "long job", prompt: "do the long job", subagent_type: "general-purpose" },
			controller.signal,
			undefined,
			{ cwd: makeTempDir(), hasUI: true } as never,
		);
		await new Promise((r) => setImmediate(r));
		controller.abort();

		await expect(call).rejects.toThrow(/cancelled by user/);
		const task = taskStore.list().find((t) => t.source === "subagent");
		expect(task?.status).toBe("cancelled");
		const row = taskStore.agents().find((a) => a.kind === "subagent");
		expect(row?.state).toBe("cancelled");
	});
});
