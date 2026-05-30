import { spawn } from "node:child_process";
import { existsSync, mkdirSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { SubagentLifeguard } from "../src/core/lifeguard.js";

describe("SubagentLifeguard", () => {
	let testCwd: string;
	let guard: SubagentLifeguard;

	beforeEach(() => {
		testCwd = join(tmpdir(), `hoocode-lifeguard-test-${Date.now()}`);
		mkdirSync(testCwd, { recursive: true });
	});

	afterEach(() => {
		guard?.dispose();
		if (existsSync(testCwd)) {
			rmSync(testCwd, { recursive: true, force: true });
		}
	});

	it("monitors a child process", () => {
		guard = new SubagentLifeguard(testCwd);
		const script = join(testCwd, "sleep.js");
		writeFileSync(script, "setTimeout(() => {}, 10000)\n");
		const proc = spawn(process.execPath, [script], { detached: true });
		guard.monitor("t1", "explore", proc);
		expect(guard.isMonitoring("t1")).toBe(true);
		expect(guard.lastHeartbeatAt("t1")).not.toBeNull();
		proc.kill("SIGKILL");
	});

	it("records heartbeats", () => {
		guard = new SubagentLifeguard(testCwd);
		const script = join(testCwd, "sleep.js");
		writeFileSync(script, "setTimeout(() => {}, 10000)\n");
		const proc = spawn(process.execPath, [script], { detached: true });
		guard.monitor("t1", "explore", proc);
		const before = guard.lastHeartbeatAt("t1")!;
		// Simulate heartbeat
		guard.recordHeartbeat("t1");
		const after = guard.lastHeartbeatAt("t1")!;
		expect(after).toBeGreaterThanOrEqual(before);
		proc.kill("SIGKILL");
	});

	it("emits stalled when heartbeat is missed", async () => {
		guard = new SubagentLifeguard(testCwd);
		// Use a script that does NOT emit heartbeats
		const script = join(testCwd, "silent.js");
		writeFileSync(script, "setTimeout(() => {}, 10000)\n");
		const proc = spawn(process.execPath, [script], { detached: true });

		let stalledEvent: { task_id: string; pid: number } | undefined;
		guard.on("stalled", (data) => {
			stalledEvent = data as { task_id: string; pid: number };
		});

		guard.monitor("t1", "explore", proc);
		// Manually backdate the heartbeat so the 60s threshold is crossed immediately
		// @ts-expect-error – accessing internal map for test
		guard.lastHeartbeat.set("t1", Date.now() - 70_000);

		// Trigger check immediately
		// @ts-expect-error – accessing internal method for test
		guard.checkHeartbeats();

		expect(stalledEvent).toBeDefined();
		expect(stalledEvent?.task_id).toBe("t1");
		proc.kill("SIGKILL");
	});

	it("emits timeout when hard timeout is exceeded", async () => {
		guard = new SubagentLifeguard(testCwd);
		const script = join(testCwd, "slow.js");
		writeFileSync(script, "setTimeout(() => {}, 10000)\n");
		const proc = spawn(process.execPath, [script], { detached: true });

		let timeoutEvent: { task_id: string; pid: number } | undefined;
		guard.on("timeout", (data) => {
			timeoutEvent = data as { task_id: string; pid: number };
		});

		// Override explore timeout to 10ms for testing
		guard.monitor("t1", "explore", proc);

		// Clear the original long timeout and set a short one
		// @ts-expect-error
		const existingTimeout = guard.timeouts.get("t1");
		if (existingTimeout) clearTimeout(existingTimeout);
		// @ts-expect-error
		guard.timeouts.set(
			"t1",
			setTimeout(() => {
				// @ts-expect-error
				guard.handleTimeout("t1");
			}, 10),
		);

		await new Promise((r) => setTimeout(r, 100));
		expect(timeoutEvent).toBeDefined();
		expect(timeoutEvent?.task_id).toBe("t1");
		proc.kill("SIGKILL");
	});

	it("untracks process on exit", async () => {
		guard = new SubagentLifeguard(testCwd);
		const script = join(testCwd, "exit.js");
		writeFileSync(script, "process.exit(0)\n");
		const proc = spawn(process.execPath, [script]);

		guard.monitor("t1", "explore", proc);
		expect(guard.isMonitoring("t1")).toBe(true);

		await new Promise<void>((resolve) => {
			proc.on("exit", () => {
				// Give the lifeguard a tick to process the exit
				setTimeout(() => {
					expect(guard.isMonitoring("t1")).toBe(false);
					resolve();
				}, 50);
			});
		});
	});

	it("sweeps old agent directories on init", () => {
		const agentsDir = join(testCwd, ".hoocode", "agents");
		mkdirSync(agentsDir, { recursive: true });

		const oldDir = join(agentsDir, "old-task");
		mkdirSync(oldDir, { recursive: true });
		const oldFile = join(oldDir, "result.json");
		writeFileSync(oldFile, "{}");

		// Backdate the directory itself
		const past = new Date(Date.now() - 25 * 60 * 60 * 1000); // 25 hours ago
		utimesSync(oldDir, past, past);

		expect(existsSync(oldDir)).toBe(true);
		guard = new SubagentLifeguard(testCwd);
		// Sweep runs synchronously in constructor
		expect(existsSync(oldDir)).toBe(false);
	});

	it("does not sweep directories with running PIDs", () => {
		const agentsDir = join(testCwd, ".hoocode", "agents");
		mkdirSync(agentsDir, { recursive: true });

		const oldDir = join(agentsDir, "old-task-with-pid");
		mkdirSync(oldDir, { recursive: true });
		const oldFile = join(oldDir, "result.json");
		writeFileSync(oldFile, "{}");
		writeFileSync(join(oldDir, "pid"), String(process.pid));

		const past = new Date(Date.now() - 25 * 60 * 60 * 1000);
		utimesSync(oldFile, past, past);

		expect(existsSync(oldDir)).toBe(true);
		guard = new SubagentLifeguard(testCwd);
		expect(existsSync(oldDir)).toBe(true);
	});

	it("dispose kills all monitored processes", () => {
		guard = new SubagentLifeguard(testCwd);
		const script = join(testCwd, "sleep.js");
		writeFileSync(script, "setTimeout(() => {}, 10000)\n");
		const proc = spawn(process.execPath, [script], { detached: true });
		guard.monitor("t1", "explore", proc);
		guard.dispose();
		expect(guard.isMonitoring("t1")).toBe(false);
	});
});
