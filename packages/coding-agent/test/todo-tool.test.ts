import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { taskOwnerId, taskStore } from "../src/core/task-store.js";
import { createTodoWriteToolDefinition } from "../src/core/tools/todo.js";

const tool = createTodoWriteToolDefinition();

function write(
	todos: Array<{ content: string; status: "pending" | "in_progress" | "completed"; activeForm?: string }>,
) {
	return tool.execute("todo-test", { todos }, undefined, undefined, {} as any);
}

function mainTasks() {
	return taskStore.list().filter((t) => taskOwnerId(t) === "main");
}

describe("TodoWrite tool", () => {
	beforeEach(() => taskStore.clear());
	afterEach(() => taskStore.clear());

	it("creates main-agent tasks and maps statuses", async () => {
		await write([
			{ content: "First", status: "completed" },
			{ content: "Second", status: "in_progress", activeForm: "Doing second" },
			{ content: "Third", status: "pending" },
		]);

		const tasks = mainTasks();
		expect(tasks.map((t) => [t.title, t.status])).toEqual([
			["First", "done"],
			["Doing second", "in_progress"], // activeForm used while in_progress
			["Third", "pending"],
		]);
	});

	it("replaces the list each call, reconciling by position (update, add, remove)", async () => {
		await write([
			{ content: "A", status: "in_progress" },
			{ content: "B", status: "pending" },
		]);
		const firstIds = mainTasks().map((t) => t.id);

		await write([
			{ content: "A", status: "completed" }, // updated in place
			{ content: "B", status: "in_progress" }, // updated in place
			{ content: "C", status: "pending" }, // added
		]);

		const tasks = mainTasks();
		expect(tasks.map((t) => [t.title, t.status])).toEqual([
			["A", "done"],
			["B", "in_progress"],
			["C", "pending"],
		]);
		// Kept items reuse their ids (no flicker in the panel).
		expect(tasks.slice(0, 2).map((t) => t.id)).toEqual(firstIds);
	});

	it("drops the removed tail when the new list is shorter", async () => {
		await write([
			{ content: "A", status: "pending" },
			{ content: "B", status: "pending" },
			{ content: "C", status: "pending" },
		]);

		await write([{ content: "A", status: "completed" }]);

		expect(mainTasks().map((t) => t.title)).toEqual(["A"]);
	});

	it("clears all main tasks on an empty list", async () => {
		await write([{ content: "A", status: "pending" }]);
		const result = await write([]);

		expect(mainTasks()).toHaveLength(0);
		expect((result.content[0] as { text?: string }).text).toContain("cleared");
	});

	it("reports counts in the result text", async () => {
		const result = await write([
			{ content: "A", status: "in_progress" },
			{ content: "B", status: "pending" },
			{ content: "C", status: "completed" },
		]);

		expect((result.content[0] as { text?: string }).text).toContain("1 in progress, 1 pending, 1 completed");
		expect(result.details).toEqual({ total: 3, pending: 1, inProgress: 1, completed: 1 });
	});
});
