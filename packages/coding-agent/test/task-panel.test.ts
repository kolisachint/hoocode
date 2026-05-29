import { afterAll, beforeAll, beforeEach, describe, expect, test } from "vitest";
import { taskStore } from "../src/core/task-store.js";
import { TaskPanelComponent } from "../src/modes/interactive/components/task-panel.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";
import { createHarness, type Harness } from "./suite/harness.js";

describe("task panel rendering", () => {
	let harness: Harness;
	let panel: TaskPanelComponent;

	beforeAll(async () => {
		initTheme("dark");
		harness = await createHarness();
		panel = new TaskPanelComponent();
	});

	beforeEach(() => {
		taskStore.clear();
	});

	afterAll(() => {
		harness.cleanup();
	});

	function renderPanel(width = 120): string[] {
		return panel.render(width);
	}

	test("collapses to empty when no active tasks", () => {
		const lines = renderPanel();
		expect(lines).toEqual([]);
	});

	test("shows active subagent tasks with status icons, mode tags, and task IDs", () => {
		const explore = taskStore.create("SSE watch endpoint", { subagentMode: "explore" });
		const edit = taskStore.create("Auth refactor", { subagentMode: "edit" });
		const plain = taskStore.create("Init project");

		taskStore.update(explore.id, { status: "in_progress" });
		taskStore.update(edit.id, { status: "pending" });
		taskStore.update(plain.id, { status: "in_progress" });

		const lines = renderPanel();
		expect(lines.length).toBe(3);

		const text = lines.join("\n");
		expect(text).toContain("SSE watch endpoint");
		expect(text).toContain("Auth refactor");
		expect(text).toContain("Init project");
		expect(text).toContain("[explore]");
		expect(text).toContain("[edit]");
		expect(text).toContain("◐");
		expect(text).toContain("●");
		// Task IDs should be visible
		expect(text).toContain(`#${explore.id}`);
		expect(text).toContain(`#${edit.id}`);
		expect(text).toContain(`#${plain.id}`);
	});

	test("completed tasks drop out of the panel", () => {
		taskStore.create("Still running", { subagentMode: "explore" });
		const doneTask = taskStore.create("Finished work");
		taskStore.update(doneTask.id, { status: "done" });

		const lines = renderPanel();
		const text = lines.join("\n");
		expect(text).not.toContain("Finished work");
		expect(text).toContain("Still running");
	});

	test("failed tasks drop out of the panel", () => {
		taskStore.create("Still running", { subagentMode: "explore" });
		const failedTask = taskStore.create("Broken build");
		taskStore.update(failedTask.id, { status: "failed" });

		const lines = renderPanel();
		const text = lines.join("\n");
		expect(text).not.toContain("Broken build");
		expect(text).toContain("Still running");
	});

	test("shows all active tasks without a line limit", () => {
		for (let i = 0; i < 6; i++) {
			const t = taskStore.create(`Task ${i}`);
			taskStore.update(t.id, { status: "in_progress" });
		}

		const lines = renderPanel();
		expect(lines.length).toBe(6);

		const text = lines.join("\n");
		expect(text).toContain("Task 0");
		expect(text).toContain("Task 5");
	});
});
