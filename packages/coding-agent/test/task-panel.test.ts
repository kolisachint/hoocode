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

	// The id is dimmed and titles can be muted, so an ANSI reset sits between the
	// id and the title. Strip color when asserting they render adjacent.
	const stripAnsi = (s: string): string => s.replace(/\x1b\[[0-9;]*m/g, "");

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
		// Active work shows the WORKING stamp (◐) and a pending dot (●).
		expect(text).toContain("◐");
		expect(text).toContain("●");
		// A pending task is tagged queued.
		expect(text).toContain("queued");
		// Task IDs should be visible
		expect(text).toContain(`#${explore.id}`);
		expect(text).toContain(`#${edit.id}`);
		expect(text).toContain(`#${plain.id}`);
	});

	test("shows a source glyph for subagent and MCP rows, but not for plain tasks or mode tags", () => {
		const sub = taskStore.create("find the bug", { source: "subagent", subagentMode: "explore" });
		const mcp = taskStore.create("web › fetch", { source: "mcp" });
		const plain = taskStore.create("init project");
		taskStore.update(sub.id, { status: "in_progress" });
		taskStore.update(mcp.id, { status: "in_progress" });
		taskStore.update(plain.id, { status: "in_progress" });

		const lines = renderPanel();
		const subRow = lines.find((l) => l.includes("find the bug"));
		const mcpRow = lines.find((l) => l.includes("web › fetch"));
		const plainRow = lines.find((l) => l.includes("init project"));

		// Subagent row carries the ⚙ glyph; MCP row carries ⧉.
		expect(subRow).toContain("⚙");
		expect(mcpRow).toContain("⧉");
		// Plain task gets neither glyph.
		expect(plainRow).not.toContain("⚙");
		expect(plainRow).not.toContain("⧉");
		// The glyph is a source marker, NOT a mode tag — the pane stays tag-free.
		expect(lines.join("\n")).not.toContain("[explore]");
		// Each row is still padded to the full pane width (glyph cell didn't break alignment).
		for (const row of [subRow, mcpRow, plainRow]) {
			expect(visibleWidth(row as string)).toBe(120);
		}
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

	test("a task note renders as a \u26a0 cue, replacing the usage/status stamp", () => {
		const done = taskStore.create("Audit reconnect path");
		taskStore.update(done.id, {
			status: "done",
			usage: { input: 4000, output: 500, cacheRead: 0, cacheWrite: 0, cost: 0 },
			note: "ran on inherited model",
		});
		const skipped = taskStore.create("Run suite");
		taskStore.update(skipped.id, { status: "failed", note: "anthropic exhausted" });

		const lines = renderPanel();
		const noteRow = lines.find((l) => l.includes("Audit reconnect path"));
		expect(noteRow).toBeDefined();
		expect(noteRow).toContain("\u26a0 ran on inherited model");
		// The ⚠ cue takes over the right column, so the usage stamp is not shown.
		expect(noteRow).not.toContain("4.5k");

		const failRow = lines.find((l) => l.includes("Run suite"));
		expect(failRow).toContain("\u26a0 anthropic exhausted");
	});

	test("long titles use the full left width regardless of task-id digit count", () => {
		const long = "A deliberately long task title that should fill the row up to the right column";
		const a = taskStore.create(long);
		taskStore.update(a.id, { status: "in_progress" });

		const width = 80;
		const row = renderPanel(width).find((l) => stripAnsi(l).includes("#1"));
		expect(row).toBeDefined();
		// The row fills the full pane width: header + rows are padded to `width`.
		expect(visibleWidth(row as string)).toBe(width);
		// The title is truncated (too long to fit) but uses the full budget, so the
		// last visible title chars before the ellipsis reach deep into the row.
		expect(stripAnsi(row as string)).toMatch(/should fill the row\b/);
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
		// Both tasks done → REVIEWED stamp + a full progress bar.
		expect(stripAnsi(header)).toContain("REVIEWED");
		expect(header).toContain("━");
		expect(stripAnsi(header)).toContain("2/2");
	});

	test("header omits the turn delta when no task reported usage", () => {
		const t = taskStore.create("Plain work");
		taskStore.update(t.id, { status: "in_progress" });

		const header = renderPanel()[0];
		expect(header).not.toContain("turn ↑");
		// Active task → WORKING stamp + 0/1 count.
		expect(stripAnsi(header)).toContain("WORKING");
		expect(stripAnsi(header)).toContain("0/1");
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

	test("reset clears finished tasks and restarts numbering from #1", () => {
		const doneTask = taskStore.create("Finished work");
		taskStore.update(doneTask.id, { status: "done" });
		const failedTask = taskStore.create("Broken build");
		taskStore.update(failedTask.id, { status: "failed" });

		taskStore.reset();

		expect(renderPanel()).toEqual([]); // empty pane, no header

		// Next turn starts numbering over at #1.
		const next = taskStore.create("Fresh task");
		expect(next.id).toBe(1);
		expect(stripAnsi(renderPanel().join("\n"))).toContain("#1 Fresh task");
	});

	test("reset keeps active tasks and does not restart numbering while one survives", () => {
		const running = taskStore.create("Still running", { subagentMode: "explore" });
		taskStore.update(running.id, { status: "in_progress" });
		const doneTask = taskStore.create("Finished work");
		taskStore.update(doneTask.id, { status: "done" });

		taskStore.reset();

		const text = renderPanel().join("\n");
		expect(text).toContain("Still running");
		expect(text).not.toContain("Finished work");

		// A late status update on the surviving task still lands (id not orphaned).
		taskStore.update(running.id, { status: "done" });
		expect(renderPanel().join("\n")).toContain("Still running");

		// Numbering did not reset: the next task keeps counting up, not back to #1.
		const next = taskStore.create("Next task");
		expect(next.id).not.toBe(1);
		expect(next.id).toBeGreaterThan(running.id);
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
