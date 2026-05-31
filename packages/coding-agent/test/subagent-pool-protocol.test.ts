/**
 * Integration coverage for the spawned-subagent protocol the pool depends on:
 *
 * - the child writes its own `.hoocode/dispatch/<task_id>/result.json` (the real
 *   round-trip, not a test-prewritten file), and the pool verifies + surfaces it;
 * - the child emits `{"ping":true}` heartbeats on stdout, which the pool parses
 *   and forwards to the lifeguard's recordHeartbeat.
 *
 * Uses a mock executable (no LLM) so the protocol is exercised deterministically.
 */

import { chmodSync, existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { SubagentLifeguard } from "../src/core/lifeguard.js";
import { SubagentPool, type SubagentPoolTask } from "../src/core/subagent-pool.js";

/**
 * Mock hoocode that mimics a spawned subagent: it parses --task-id from argv,
 * emits a heartbeat, writes a valid result.json (with usage), and exits 0.
 */
function createProtocolMock(dir: string): string {
	const path = join(dir, "mock-protocol.js");
	const content = `#!/usr/bin/env node
const { mkdirSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

const argv = process.argv.slice(2);
const idIdx = argv.indexOf("--task-id");
const taskId = idIdx !== -1 ? argv[idIdx + 1] : "unknown";

// Heartbeat the parent lifeguard watches for.
process.stdout.write(JSON.stringify({ ping: true }) + "\\n");

// Normal json-mode event.
process.stdout.write(
	JSON.stringify({ type: "done", reason: "stop", message: { role: "assistant", content: [{ type: "text", text: "ok" }] } }) + "\\n",
);

// The subagent writes its own audit file, exactly as print-mode does for real.
const resultDir = join(process.cwd(), ".hoocode", "dispatch", taskId);
mkdirSync(resultDir, { recursive: true });
writeFileSync(
	join(resultDir, "result.json"),
	JSON.stringify({
		summary: "child-written summary",
		files_changed: ["src/changed.ts"],
		confidence: 0.9,
		status: "complete",
		usage: { input: 100, output: 20, cacheRead: 0, cacheWrite: 0, cost: 0.001 },
	}),
);

process.exit(0);
`;
	writeFileSync(path, content);
	chmodSync(path, 0o755);
	return path;
}

describe("SubagentPool spawned-subagent protocol", () => {
	let tmpDir: string;
	let pool: SubagentPool | undefined;

	beforeEach(() => {
		tmpDir = join(tmpdir(), `subagent-protocol-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(tmpDir, { recursive: true });
	});

	afterEach(() => {
		pool?.dispose();
		vi.restoreAllMocks();
		if (existsSync(tmpDir)) {
			rmSync(tmpDir, { recursive: true });
		}
	});

	test("reads the result.json the child writes itself and forwards heartbeats to the lifeguard", async () => {
		const recordSpy = vi.spyOn(SubagentLifeguard.prototype, "recordHeartbeat");

		const exe = createProtocolMock(tmpDir);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });
		const task: SubagentPoolTask = { task_id: "proto-1", agent_type: "explore", task: "hello" };

		// Intentionally do NOT pre-write result.json: the child must produce it.
		pool.spawn(task);
		const result = await pool.wait_for("proto-1");

		expect(result.ok).toBe(true);
		expect(result.status).toBe("complete");
		// The verified, child-written result.json is surfaced to callers.
		expect(result.result_data).toMatchObject({
			summary: "child-written summary",
			files_changed: ["src/changed.ts"],
			status: "complete",
		});

		// The pool parsed the {"ping":true} line and forwarded it to the lifeguard.
		expect(recordSpy).toHaveBeenCalledWith("proto-1");
	});

	test("dispatch surfaces the child-written summary and usage", async () => {
		const exe = createProtocolMock(tmpDir);
		pool = new SubagentPool({ executable: exe, maxConcurrency: 1, cwd: tmpDir });

		const dispatchResult = await pool.dispatch("investigate the parser thoroughly", { forceAgent: "explore" });

		expect(dispatchResult.handled_inline).toBe(false);
		expect(dispatchResult.result?.ok).toBe(true);
		const data = dispatchResult.result?.result_data as { summary: string; usage?: { input: number } } | undefined;
		expect(data?.summary).toBe("child-written summary");
		expect(data?.usage?.input).toBe(100);
	});
});
