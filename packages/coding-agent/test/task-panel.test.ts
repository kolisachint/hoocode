import { visibleWidth } from "@kolisachint/hoocode-tui";
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

	test("collapses to empty when there are no tasks", () => {
		const lines = renderPanel();
		expect(lines).toEqual([]);
	});

	test("shows active subagent tasks with status icons and task IDs, but no mode tags", () => {
		const explore = taskStore.create("SSE watch endpoint", { subagentMode: "explore" });
		const edit = taskStore.create("Auth refactor", { subagentMode: "edit" });
		const plain = taskStore.create("Init project");

		taskStore.update(explore.id, { status: "in_progress" });
		taskStore.update(edit.id, { status: "pending" });
		taskStore.update(plain.id, { status: "in_progress" });

		const lines = renderPanel();
		expect(lines.length).toBe(4); // 3 tasks + 1 header

		const text = lines.join("\n");
		expect(text).toContain("SSE watch endpoint");
		expect(text).toContain("Auth refactor");
		expect(text).toContain("Init project");
		// Subagent mode tags are intentionally not shown in the task pane.
		expect(text).not.toContain("[explore]");
		expect(text).not.toContain("[edit]");
		expect(text).toContain("◐");
		expect(text).toContain("●");
		// Task IDs should be visible
		expect(text).toContain(`#${explore.id}`);
		expect(text).toContain(`#${edit.id}`);
		expect(text).toContain(`#${plain.id}`);
	});

	test("completed and failed tasks stay visible with their status", () => {
		taskStore.create("Still running", { subagentMode: "explore" });
		const doneTask = taskStore.create("Finished work");
		taskStore.update(doneTask.id, { status: "done" });
		const failedTask = taskStore.create("Broken build");
		taskStore.update(failedTask.id, { status: "failed" });

		const lines = renderPanel();
		expect(lines.length).toBe(4); // 3 tasks + 1 header

		const text = lines.join("\n");
		expect(text).toContain("Still running");
		expect(text).toContain("Finished work");
		expect(text).toContain("Broken build");
		expect(text).toContain("✓");
		expect(text).toContain("✗");
	});

	test("finished tasks show combined token usage and elapsed time per row", () => {
		const done = taskStore.create("Investigate flaky test");
		taskStore.update(done.id, {
			status: "done",
			usage: { input: 9000, output: 1100, cacheRead: 0, cacheWrite: 0, cost: 0.02 },
		});

		const lines = renderPanel();
		const row = lines.find((l) => l.includes("Investigate flaky test"));
		expect(row).toBeDefined();
		// 9000 + 1100 = 10100 → "10k"; elapsed is derived from create/update stamps.
		expect(row).toContain("10k");
		expect(row).toMatch(/\d+(\.\d+)?(s|m\d{2}s)/);
		// The per-row stamp uses a single combined total, not split ↑/↓ arrows.
		expect(row).not.toContain("↑");
		expect(row).not.toContain("↓");
	});

	test("header shows the per-turn token and cost delta summed across tasks", () => {
		const a = taskStore.create("Explore module");
		taskStore.update(a.id, {
			status: "done",
			usage: { input: 3000, output: 500, cacheRead: 0, cacheWrite: 0, cost: 0.01 },
		});
		const b = taskStore.create("Run tests");
		taskStore.update(b.id, {
			status: "done",
			usage: { input: 2000, output: 700, cacheRead: 0, cacheWrite: 0, cost: 0.02 },
		});

		const header = renderPanel()[0];
		expect(header).toContain("turn");
		expect(header).toContain("↑");
		expect(header).toContain("↓");
		// 3000+2000=5000 → "5.0k"; 500+700=1200 → "1.2k"; cost 0.03 → "$0.030".
		expect(header).toContain("5.0k");
		expect(header).toContain("1.2k");
		expect(header).toContain("$0.030");
	});

	test("header omits the turn delta when no task reported usage", () => {
		const t = taskStore.create("Plain work");
		taskStore.update(t.id, { status: "in_progress" });

		const header = renderPanel()[0];
		expect(header).not.toContain("turn ↑");
		expect(header).toContain("0/1 done");
	});

	test("header turn delta omits cost when it is zero (e.g. subscription)", () => {
		const t = taskStore.create("Explore on subscription");
		taskStore.update(t.id, {
			status: "done",
			usage: { input: 4000, output: 600, cacheRead: 0, cacheWrite: 0, cost: 0 },
		});

		const header = renderPanel()[0];
		expect(header).toContain("turn ↑");
		expect(header).toContain("4.0k");
		expect(header).not.toContain("$");
	});

	test("header degrades to stamp + count on a very narrow terminal", () => {
		const t = taskStore.create("Explore module");
		taskStore.update(t.id, {
			status: "done",
			usage: { input: 3000, output: 500, cacheRead: 0, cacheWrite: 0, cost: 0.01 },
		});

		const header = renderPanel(20)[0];
		expect(visibleWidth(header)).toBeLessThanOrEqual(20);
		// The turn delta is dropped; the stamp/count survive (possibly truncated).
		expect(header).not.toContain("turn ↑");
	});

	test("retireFinished removes done and failed tasks but keeps active ones", () => {
		const running = taskStore.create("Still running", { subagentMode: "explore" });
		taskStore.update(running.id, { status: "in_progress" });
		const doneTask = taskStore.create("Finished work");
		taskStore.update(doneTask.id, { status: "done" });
		const failedTask = taskStore.create("Broken build");
		taskStore.update(failedTask.id, { status: "failed" });

		taskStore.retireFinished();

		const lines = renderPanel();
		const text = lines.join("\n");
		expect(lines.length).toBe(2); // 1 task + 1 header
		expect(text).toContain("Still running");
		expect(text).not.toContain("Finished work");
		expect(text).not.toContain("Broken build");
	});

	test("shows all tasks without a line limit", () => {
		for (let i = 0; i < 6; i++) {
			const t = taskStore.create(`Task ${i}`);
			taskStore.update(t.id, { status: "in_progress" });
		}

		const lines = renderPanel();
		expect(lines.length).toBe(7); // 6 tasks + 1 header

		const text = lines.join("\n");
		expect(text).toContain("Task 0");
		expect(text).toContain("Task 5");
	});
});
