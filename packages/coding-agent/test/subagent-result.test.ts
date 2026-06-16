import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import { afterEach, describe, expect, it } from "vitest";
import { CONFIG_DIR_NAME } from "../src/config.js";
import { OutputVerifier } from "../src/core/output-verifier.js";
import { buildSubagentResult, buildTaskForest, writeSubagentResult } from "../src/core/subagent-result.js";
import type { Task } from "../src/core/task-store.js";

const dirs: string[] = [];

function tempCwd(): string {
	const dir = mkdtempSync(join(tmpdir(), "subagent-result-"));
	dirs.push(dir);
	return dir;
}

afterEach(() => {
	while (dirs.length > 0) {
		const dir = dirs.pop();
		if (dir && existsSync(dir)) rmSync(dir, { recursive: true, force: true });
	}
});

function assistantText(text: string): AgentMessage {
	return {
		role: "assistant",
		content: [{ type: "text", text }],
		stopReason: "stop",
	} as unknown as AgentMessage;
}

function assistantToolCall(name: string, args: Record<string, unknown>): AgentMessage {
	return {
		role: "assistant",
		content: [{ type: "toolCall", id: "x", name, arguments: args }],
		stopReason: "toolUse",
	} as unknown as AgentMessage;
}

describe("buildSubagentResult", () => {
	it("uses the last assistant text as the summary", () => {
		const result = buildSubagentResult([assistantText("first"), assistantText("final answer")]);
		expect(result.summary).toBe("final answer");
		expect(result.status).toBe("complete");
		expect(result.confidence).toBeGreaterThanOrEqual(0.5);
	});

	it("collects changed files from edit/write tool calls", () => {
		const result = buildSubagentResult([
			assistantToolCall("edit", { path: "src/a.ts" }),
			assistantToolCall("write", { file_path: "src/b.ts" }),
			assistantToolCall("read", { path: "src/c.ts" }),
			assistantText("done"),
		]);
		expect(result.files_changed.sort()).toEqual(["src/a.ts", "src/b.ts"]);
	});

	it("marks failed when the final assistant message is an error", () => {
		const errored = {
			role: "assistant",
			content: [],
			stopReason: "error",
			errorMessage: "boom",
		} as unknown as AgentMessage;
		const result = buildSubagentResult([assistantText("partial"), errored]);
		expect(result.status).toBe("failed");
	});

	it("surfaces the provider error message in the summary on failure", () => {
		const errored = {
			role: "assistant",
			content: [{ type: "text", text: "partial progress" }],
			stopReason: "error",
			errorMessage: "Anthropic usage limit reached. Please try again later.",
		} as unknown as AgentMessage;
		const result = buildSubagentResult([errored]);
		expect(result.status).toBe("failed");
		expect(result.summary).toContain("usage limit reached");
		// Verifier still passes (non-empty summary, confidence >= 0.5).
		expect(result.confidence).toBeGreaterThanOrEqual(0.5);
	});

	it("falls back to a placeholder summary when there is no assistant text", () => {
		const result = buildSubagentResult([assistantToolCall("edit", { path: "x.ts" })]);
		expect(result.summary.length).toBeGreaterThan(0);
		expect(result.confidence).toBeGreaterThanOrEqual(0.5);
	});

	it("reports a partial result with the agent's summary when stopped at the turn cap", () => {
		const aborted = {
			role: "assistant",
			content: [{ type: "text", text: "Found the bug in auth.ts; ran out of turns before fixing it." }],
			stopReason: "aborted",
		} as unknown as AgentMessage;
		const result = buildSubagentResult([assistantText("investigating"), aborted], undefined, {
			reachedMaxTurns: true,
		});
		expect(result.status).toBe("partial");
		expect(result.confidence).toBeGreaterThanOrEqual(0.5);
		expect(result.summary).toContain("Found the bug");
	});

	it("yields a verifier-passing partial result even with no assistant text at the turn cap", () => {
		const result = buildSubagentResult([assistantToolCall("edit", { path: "x.ts" })], undefined, {
			reachedMaxTurns: true,
		});
		expect(result.status).toBe("partial");
		expect(result.summary.length).toBeGreaterThan(0);
		expect(result.confidence).toBeGreaterThanOrEqual(0.5);
	});

	it("produces output that passes OutputVerifier", () => {
		const cwd = tempCwd();
		const result = buildSubagentResult([assistantText("all good")]);
		writeSubagentResult(cwd, "task-1", result);

		const path = join(cwd, CONFIG_DIR_NAME, "dispatch", "task-1", "result.json");
		expect(existsSync(path)).toBe(true);
		const parsed = JSON.parse(readFileSync(path, "utf-8"));
		expect(parsed.summary).toBe("all good");

		const verification = new OutputVerifier(cwd).verify("task-1");
		expect(verification.valid).toBe(true);
	});
});

describe("buildTaskForest", () => {
	const now = Date.now();
	const task = (id: number, fields: Partial<Task>): Task => ({
		id,
		title: `task-${id}`,
		status: "done",
		createdAt: now,
		updatedAt: now,
		...fields,
	});

	it("nests children under their parent via parentTaskId", () => {
		const tasks: Task[] = [
			task(1, { source: "subagent", subagentMode: "explore" }),
			task(2, { source: "subagent", subagentMode: "review", parentTaskId: 1 }),
			task(3, { source: "mcp", subagentMode: "web", parentTaskId: 2 }),
		];

		const forest = buildTaskForest(tasks);
		expect(forest.length).toBe(1);
		expect(forest[0]?.id).toBe(1);
		expect(forest[0]?.subagentMode).toBe("explore");
		expect(forest[0]?.children.map((c) => c.id)).toEqual([2]);
		// Grandchild nests two levels deep, preserving source for MCP rendering.
		const grandchild = forest[0]?.children[0]?.children[0];
		expect(grandchild?.id).toBe(3);
		expect(grandchild?.source).toBe("mcp");
	});

	it("returns multiple roots and preserves store order for siblings", () => {
		const tasks: Task[] = [
			task(1, { source: "subagent" }),
			task(2, { source: "subagent" }),
			task(3, { source: "subagent", parentTaskId: 1 }),
			task(4, { source: "subagent", parentTaskId: 1 }),
		];

		const forest = buildTaskForest(tasks);
		expect(forest.map((n) => n.id)).toEqual([1, 2]);
		expect(forest[0]?.children.map((c) => c.id)).toEqual([3, 4]);
	});
});
