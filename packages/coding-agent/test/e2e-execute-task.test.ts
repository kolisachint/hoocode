/**
 * End-to-end tests for the ExecuteTask architecture:
 * - TodoWrite with complexity field
 * - ExecuteTask with model category resolution
 * - TaskOutput polling
 * - Background execution (non-blocking)
 */

import { mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { resolveModelReference } from "../src/core/model-categories.js";
import type { SubagentPool, TaskResult } from "../src/core/subagent-pool.js";
import { setSubagentPoolForTesting } from "../src/core/subagent-pool-instance.js";
import type { SubagentResultFile } from "../src/core/subagent-result.js";
import { taskStore } from "../src/core/task-store.js";
import { createExecuteTaskToolDefinition } from "../src/core/tools/subagent.js";
import { createTodoWriteToolDefinition } from "../src/core/tools/todo.js";

// ── Test helpers ─────────────────────────────────────────────────────────────

function makeTempDir(): string {
	const dir = join(tmpdir(), `e2e-execute-task-${Date.now()}-${Math.random().toString(36).slice(2)}`);
	mkdirSync(dir, { recursive: true });
	return dir;
}

function fakeResult(ok: boolean, data: Partial<SubagentResultFile> = {}): TaskResult {
	return {
		handled_inline: false,
		task_id: "fake-1",
		agent_type: "explore",
		reason: "test",
		duration: 100,
		result: {
			task_id: "fake-1",
			ok,
			stdout: "",
			stderr: "",
			exit_code: ok ? 0 : 1,
			status: ok ? "complete" : "failed",
			result_data: { summary: "Test completed", files_changed: [], confidence: 0.9, status: "complete", ...data },
		},
	};
}

function makeFakePool(result: TaskResult): SubagentPool {
	return {
		dispatch: async () => result,
		dispatchDetached: () => ({ handled_inline: false, task_id: "fake-1", agent_type: "explore", reason: "test" }),
		get_status: () => "done",
		collect: () => result.result,
		wait_for: async () => result.result!,
		wait_for_completion: async () => result.result,
		running_count: () => 0,
		queued_count: () => 0,
	} as unknown as SubagentPool;
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe("TodoWrite with complexity field", () => {
	const todoTool = createTodoWriteToolDefinition();

	beforeEach(() => taskStore.clear());
	afterEach(() => taskStore.clear());

	it("accepts complexity field on todo items", async () => {
		const result = await todoTool.execute(
			"todo-1",
			{
				todos: [
					{ content: "Read codebase", status: "in_progress", complexity: "fast" },
					{ content: "Implement feature", status: "pending", complexity: "standard" },
					{ content: "Refactor module", status: "pending", complexity: "capable" },
				],
			},
			undefined,
			undefined,
			{} as any,
		);

		expect(result.details).toEqual({
			total: 3,
			pending: 2,
			inProgress: 1,
			completed: 0,
		});

		const tasks = taskStore.list();
		expect(tasks).toHaveLength(3);
	});

	it("works without complexity field (backward compatible)", async () => {
		const result = await todoTool.execute(
			"todo-2",
			{
				todos: [
					{ content: "Simple task", status: "in_progress" },
					{ content: "Another task", status: "pending" },
				],
			},
			undefined,
			undefined,
			{} as any,
		);

		expect(result.details).toEqual({
			total: 2,
			pending: 1,
			inProgress: 1,
			completed: 0,
		});
	});
});

describe("ExecuteTask tool", () => {
	const executeTaskTool = createExecuteTaskToolDefinition();

	beforeEach(() => taskStore.clear());
	afterEach(() => {
		taskStore.clear();
		setSubagentPoolForTesting(undefined);
	});

	it("creates a task and dispatches to pool", async () => {
		const pool = makeFakePool(fakeResult(true, { summary: "Found 3 files" }));
		setSubagentPoolForTesting(pool);

		const result = await executeTaskTool.execute(
			"exec-1",
			{
				description: "Find files",
				prompt: "Find all TypeScript files",
				subagent_type: "explore",
			},
			undefined,
			undefined,
			{ cwd: makeTempDir() } as any,
		);

		expect(result.details).toMatchObject({
			subagent_type: "explore",
			ok: true,
		});
		expect((result.content[0] as { text: string }).text).toContain("Found 3 files");

		// Task should be in the store
		const tasks = taskStore.list();
		expect(tasks.length).toBeGreaterThan(0);
		expect(tasks.some((t) => t.title.includes("Find files"))).toBe(true);
	});

	it("all calls run as background (non-blocking)", () => {
		// The background flag should always return true
		expect(typeof executeTaskTool.background).toBe("function");
		const isBackground = executeTaskTool.background as (toolCall: { arguments: Record<string, unknown> }) => boolean;

		// All agent types should be background
		expect(isBackground({ arguments: { subagent_type: "explore" } })).toBe(true);
		expect(isBackground({ arguments: { subagent_type: "general-purpose" } })).toBe(true);
		expect(isBackground({ arguments: { subagent_type: "does-not-exist" } })).toBe(true);
	});

	it("tracks item_id linkage", async () => {
		const pool = makeFakePool(fakeResult(true));
		setSubagentPoolForTesting(pool);

		// Create a TodoWrite item first
		const todoTool = createTodoWriteToolDefinition();
		await todoTool.execute(
			"todo-1",
			{
				todos: [{ content: "Read codebase", status: "in_progress", complexity: "fast" }],
			},
			undefined,
			undefined,
			{} as any,
		);

		const todoTask = taskStore.list()[0];
		expect(todoTask).toBeDefined();

		// Dispatch via ExecuteTask with item_id
		const result = await executeTaskTool.execute(
			"exec-1",
			{
				description: "Read codebase",
				prompt: "Read the codebase structure",
				subagent_type: "explore",
				item_id: todoTask.id,
				complexity: "fast",
			},
			undefined,
			undefined,
			{ cwd: makeTempDir() } as any,
		);

		expect(result.details).toMatchObject({
			subagent_type: "explore",
			ok: true,
		});
	});
});

describe("TaskOutput tool", () => {
	// TaskOutput is already well-tested in subagent-pool tests
	// This verifies it works with the new ExecuteTask flow

	it("polls for results from background tasks", async () => {
		const taskOutputTool = (await import("../src/core/tools/subagent.js")).createTaskOutputToolDefinition();

		const result = fakeResult(true, { summary: "Analysis complete" });
		const pool = makeFakePool(result);
		setSubagentPoolForTesting(pool);

		const output = await taskOutputTool.execute(
			"to-1",
			{
				task_id: "fake-1",
			},
			undefined,
			undefined,
			{ cwd: makeTempDir() } as any,
		);

		expect(output.details).toMatchObject({
			task_id: "fake-1",
			status: "complete",
			ok: true,
		});
		expect((output.content[0] as { text: string }).text).toContain("Analysis complete");
	});
});

describe("Model category resolution", () => {
	it("resolves complexity to model ID via settings", () => {
		// With configured categories
		const settings = {
			modelCategories: {
				fast: "anthropic/claude-haiku-3-20240307",
				standard: "anthropic/claude-sonnet-4-20250514",
				capable: "anthropic/claude-opus-4-20250514",
			},
		};

		expect(resolveModelReference("fast", settings)).toBe("anthropic/claude-haiku-3-20240307");
		expect(resolveModelReference("standard", settings)).toBe("anthropic/claude-sonnet-4-20250514");
		expect(resolveModelReference("capable", settings)).toBe("anthropic/claude-opus-4-20250514");
	});

	it("falls back to defaults when not configured", () => {
		// Without configured categories
		expect(resolveModelReference("fast")).toBe("haiku");
		expect(resolveModelReference("standard")).toBe("sonnet");
		expect(resolveModelReference("capable")).toBe("opus");
	});

	it("passes through non-category strings", () => {
		expect(resolveModelReference("anthropic/claude-sonnet-4-20250514")).toBe("anthropic/claude-sonnet-4-20250514");
		expect(resolveModelReference("gpt-4")).toBe("gpt-4");
	});
});

describe("Complete flow: TodoWrite → ExecuteTask → TaskOutput", () => {
	beforeEach(() => taskStore.clear());
	afterEach(() => {
		taskStore.clear();
		setSubagentPoolForTesting(undefined);
	});

	it("executes a full workflow", async () => {
		const todoTool = createTodoWriteToolDefinition();
		const executeTaskTool = createExecuteTaskToolDefinition();
		const taskOutputTool = (await import("../src/core/tools/subagent.js")).createTaskOutputToolDefinition();

		// Mock pool
		const result = fakeResult(true, { summary: "Found 5 issues" });
		const pool = makeFakePool(result);
		setSubagentPoolForTesting(pool);

		// Step 1: Create a plan with TodoWrite
		await todoTool.execute(
			"todo-1",
			{
				todos: [
					{ content: "Analyze codebase", status: "in_progress", complexity: "fast" },
					{ content: "Fix issues", status: "pending", complexity: "standard" },
				],
			},
			undefined,
			undefined,
			{} as any,
		);

		const tasks = taskStore.list();
		expect(tasks).toHaveLength(2);
		expect(tasks[0].status).toBe("in_progress");

		// Step 2: Dispatch via ExecuteTask
		const execResult = await executeTaskTool.execute(
			"exec-1",
			{
				description: "Analyze codebase",
				prompt: "Analyze the codebase for issues",
				subagent_type: "explore",
				item_id: tasks[0].id,
				complexity: "fast",
			},
			undefined,
			undefined,
			{ cwd: makeTempDir() } as any,
		);

		expect(execResult.details).toMatchObject({
			subagent_type: "explore",
			ok: true,
		});

		// Step 3: Poll via TaskOutput
		const outputResult = await taskOutputTool.execute(
			"to-1",
			{
				task_id: "fake-1",
			},
			undefined,
			undefined,
			{ cwd: makeTempDir() } as any,
		);

		expect(outputResult.details).toMatchObject({
			status: "complete",
			ok: true,
		});
		expect((outputResult.content[0] as { text: string }).text).toContain("Found 5 issues");

		// Step 4: Update TodoWrite with completion
		await todoTool.execute(
			"todo-2",
			{
				todos: [
					{ content: "Analyze codebase", status: "completed", complexity: "fast" },
					{ content: "Fix issues", status: "in_progress", complexity: "standard" },
				],
			},
			undefined,
			undefined,
			{} as any,
		);

		const updatedTasks = taskStore.list();
		expect(updatedTasks[0].status).toBe("done");
		expect(updatedTasks[1].status).toBe("in_progress");
	});
});
