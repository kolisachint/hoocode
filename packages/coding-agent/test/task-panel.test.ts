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
		panel.setView("flat");
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

	test("shows active subagent tasks with status icons, task IDs, and mode tags", () => {
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
		// In the flat view, the origin tag precedes the title (design: .mode tag).
		expect(text).toContain("[explore]");
		expect(text).toContain("[edit]");
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

	test("shows a source glyph for subagent and MCP rows, but not for plain tasks", () => {
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
		// The glyph is a source marker; the row also carries its origin tag.
		expect(lines.join("\n")).toContain("[explore]");
		// Each row is still padded to the full pane width (glyph cell didn't break alignment).
		for (const row of [subRow, mcpRow, plainRow]) {
			expect(visibleWidth(row as string)).toBe(120);
		}
	});

	test("title column stays aligned across single- and double-digit ids", () => {
		for (let i = 1; i <= 10; i++) {
			const t = taskStore.create(`task-${i}`);
			taskStore.update(t.id, { status: "in_progress" });
		}

		const lines = renderPanel().map(stripAnsi);
		// The #1 row contains "task-1" but not "task-10"; the #10 row contains "task-10".
		const row1 = lines.find((l) => l.includes("task-1") && !l.includes("task-10"));
		const row10 = lines.find((l) => l.includes("task-10"));
		expect(row1).toBeDefined();
		expect(row10).toBeDefined();

		// The id column is padded to the widest id, so the title starts at the same
		// column on both rows (no jagged indentation between #1 and #10).
		expect((row1 as string).indexOf("task-1")).toBe((row10 as string).indexOf("task-10"));
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

	test("header shows the view switcher with the active lens highlighted", () => {
		const t = taskStore.create("Plain work");
		taskStore.update(t.id, { status: "in_progress" });

		expect(stripAnsi(renderPanel()[0] as string)).toContain("tasks · subagents · teams");
	});

	test("cycleView advances flat → subagents → teams → flat", () => {
		expect(panel.getView()).toBe("flat");
		expect(panel.cycleView()).toBe("subagents");
		expect(panel.cycleView()).toBe("teams");
		expect(panel.cycleView()).toBe("flat");
	});

	test("subagents view groups tasks under owner headers with glyphs and per-agent counts", () => {
		taskStore.upsertAgent({ id: "main", name: "main", role: "orchestrator", kind: "main", state: "running" });
		taskStore.upsertAgent({
			id: "explore",
			name: "explore",
			role: "subagent",
			kind: "subagent",
			state: "done",
			stats: { input: 2800, output: 360, cost: 0.011 },
		});

		const mine = taskStore.create("Map fetch() call sites");
		taskStore.update(mine.id, { status: "done" });
		const sub = taskStore.create("Port runtime http client", {
			source: "subagent",
			subagentMode: "explore",
			agent: "explore",
		});
		taskStore.update(sub.id, { status: "done" });

		panel.setView("subagents");
		const lines = renderPanel().map(stripAnsi);
		// 1 header + 2 group headers + 2 task rows.
		expect(lines.length).toBe(5);

		// Group order is deterministic: main first, then roster order.
		const mainHeader = lines[1] as string;
		expect(mainHeader).toContain("◆ main");
		expect(mainHeader).toContain("orchestrator");
		expect(mainHeader).toContain("[running]");
		expect(mainHeader).toMatch(/1\/1\s*$/);

		const subHeader = lines[3] as string;
		expect(subHeader).toContain("⊕ explore");
		expect(subHeader).toContain("[done]");
		// Per-agent stats: own token/cost totals before the count.
		expect(subHeader).toContain("↑2.8k ↓360 · $0.011");

		// Grouped rows are indented on a guide and drop the origin glyph/tag.
		const subRow = lines[4] as string;
		expect(subRow.startsWith("▎ │ ")).toBe(true);
		expect(subRow).toContain("Port runtime http client");
		expect(subRow).not.toContain("[explore]");
		expect(subRow).not.toContain("⚙");
	});

	test("teams view renders role-agents with ▸ glyphs, states, and handoff arrows", () => {
		taskStore.upsertAgent({
			id: "planner",
			name: "planner",
			role: "architect",
			kind: "role",
			state: "done",
			handoff: "→ builder",
			stats: { input: 1400, output: 260, cost: 0.004 },
		});
		taskStore.upsertAgent({ id: "builder", name: "builder", role: "engineer", kind: "role", state: "active" });

		const draft = taskStore.create("Draft the retry design", { agent: "planner" });
		taskStore.update(draft.id, { status: "done" });
		const impl = taskStore.create("Implement withRetry()", { agent: "builder" });
		taskStore.update(impl.id, { status: "in_progress" });

		panel.setView("teams");
		const lines = renderPanel().map(stripAnsi);
		expect(lines.length).toBe(5);

		const plannerHeader = lines[1] as string;
		expect(plannerHeader).toContain("▸ planner");
		expect(plannerHeader).toContain("· architect");
		expect(plannerHeader).toContain("[done]");
		expect(plannerHeader).toContain("→ builder");
		expect(plannerHeader).toContain("↑1.4k ↓260 · $0.004");

		const builderHeader = lines[3] as string;
		expect(builderHeader).toContain("▸ builder");
		expect(builderHeader).toContain("[active]");
		expect(builderHeader).toMatch(/0\/1\s*$/);

		// Rows are still padded to the full pane width in grouped views.
		for (const line of renderPanel()) {
			expect(visibleWidth(line)).toBe(120);
		}
	});

	test("grouped views fall back to default owner metadata without a roster", () => {
		const mine = taskStore.create("Init project");
		taskStore.update(mine.id, { status: "in_progress" });
		const sub = taskStore.create("find the bug", { source: "subagent", subagentMode: "explore" });
		taskStore.update(sub.id, { status: "in_progress" });

		panel.setView("subagents");
		const lines = renderPanel().map(stripAnsi);
		// Untagged tasks group under a default ◆ main; subagent-sourced ones under ⊕ subagent.
		expect(lines[1]).toContain("◆ main");
		expect(lines[1]).toContain("orchestrator");
		expect(lines[3]).toContain("⊕ subagent");
	});

	test("reset drops the roster with the tasks but keeps owners of live tasks", () => {
		taskStore.upsertAgent({ id: "explore", name: "explore", role: "subagent", kind: "subagent", state: "running" });
		taskStore.upsertAgent({ id: "review", name: "review", role: "subagent", kind: "subagent", state: "done" });
		const live = taskStore.create("Still running", { source: "subagent", agent: "explore" });
		taskStore.update(live.id, { status: "in_progress" });
		const finished = taskStore.create("Finished work", { source: "subagent", agent: "review" });
		taskStore.update(finished.id, { status: "done" });

		taskStore.reset();
		// The live task's owner survives; the settled one is dropped with its task.
		expect(taskStore.agents().map((a) => a.id)).toEqual(["explore"]);

		taskStore.update(live.id, { status: "done" });
		taskStore.reset();
		expect(taskStore.agents()).toEqual([]);
	});

	test("addAgentStats accumulates usage across dispatches of the same agent", () => {
		taskStore.upsertAgent({ id: "explore", name: "explore", kind: "subagent" });
		taskStore.addAgentStats("explore", { input: 1000, output: 200, cost: 0.01 });
		taskStore.addAgentStats("explore", { input: 500, output: 100, cost: 0.005 });

		const agent = taskStore.agents().find((a) => a.id === "explore");
		expect(agent?.stats).toEqual({ input: 1500, output: 300, cost: 0.015 });

		// Upserting the same id again (re-dispatch) keeps the accumulated stats.
		taskStore.upsertAgent({ id: "explore", name: "explore", kind: "subagent", state: "running" });
		expect(taskStore.agents().find((a) => a.id === "explore")?.stats?.input).toBe(1500);
	});
});
