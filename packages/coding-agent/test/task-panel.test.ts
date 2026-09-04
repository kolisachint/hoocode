import { visibleWidth } from "@kolisachint/hoocode-tui";
import { afterAll, beforeAll, beforeEach, describe, expect, test, vi } from "vitest";
import { taskStore } from "../src/core/task-store.js";
import { appKeyLabel } from "../src/modes/interactive/components/keybinding-hints.js";
import { TaskPanelComponent } from "../src/modes/interactive/components/task-panel.js";
import { initTheme, theme } from "../src/modes/interactive/theme/theme.js";
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
		// Only the flat lens has content, so there is nothing to switch to and the
		// header (which IS the switcher) collapses: 3 rows, no header.
		expect(lines.length).toBe(3);

		const text = lines.join("\n");
		expect(text).toContain("SSE watch endpoint");
		expect(text).toContain("Auth refactor");
		expect(text).toContain("Init project");
		// In the flat view, the origin tag precedes the title (design: .mode tag).
		expect(text).toContain("[explore]");
		expect(text).toContain("[edit]");
		// In-progress is the static ◐ of the design's glyph table (never a spinner
		// frame); pending is a hollow ○ — ● stays exclusive to the chat's tool dot.
		expect(text).toContain("◐");
		expect(text).toContain("○");
		expect(text).not.toContain("●");
		expect(text).not.toContain("⠋");
		// The glyphs carry the status, so no row restates it in words.
		expect(text).not.toContain("queued");
		expect(text).not.toContain("running…");
		// Main-owned rows carry NO owner glyph in the flat lens (the rail already
		// attributes the pane to the main agent).
		expect(text).not.toContain("◆");
	});

	test("each lens carries the owner glyph and tag for its own rows", () => {
		taskStore.upsertAgent({ id: "team:planner", name: "planner", kind: "role", state: "running" });
		const sub = taskStore.create("find the bug", { source: "subagent", subagentMode: "explore" });
		const mcp = taskStore.create("fetch", { source: "mcp", subagentMode: "web" });
		const role = taskStore.create("draft plan", { agent: "team:planner" });
		const plain = taskStore.create("init project");
		taskStore.update(sub.id, { status: "in_progress" });
		taskStore.update(mcp.id, { status: "in_progress" });
		taskStore.update(role.id, { status: "in_progress" });
		taskStore.update(plain.id, { status: "in_progress" });

		// flat ("tasks") shows only the main agent's own plan — glyph-free rows
		// (main needs no marker), no delegated rows.
		panel.setView("flat");
		const flat = renderPanel();
		const plainRow = flat.find((l) => l.includes("init project"));
		expect(plainRow).not.toContain("◆");
		expect(plainRow).not.toContain("◇");
		expect(flat.some((l) => l.includes("find the bug"))).toBe(false);
		expect(flat.some((l) => l.includes("fetch"))).toBe(false);

		// subagents tree shows delegated rows: ◇ subagent with [mode], ⧉ MCP with [server].
		panel.setView("subagents");
		const sa = renderPanel();
		const subRow = sa.find((l) => l.includes("find the bug"));
		const mcpRow = sa.find((l) => l.includes("fetch"));
		expect(subRow).toContain("◇");
		expect(subRow).toContain("[explore]");
		expect(mcpRow).toContain("⧉");
		expect(mcpRow).toContain("[web]");
		// Every row stays padded to the full pane width (the tree prefix didn't break it).
		for (const row of sa) expect(visibleWidth(row)).toBe(120);

		// teams shows the role row under its ▸ group header.
		panel.setView("teams");
		const teams = renderPanel();
		expect(teams.some((l) => l.includes("▸") && l.includes("planner"))).toBe(true);
		expect(teams.some((l) => l.includes("draft plan"))).toBe(true);
	});

	test("an MCP row without a recorded server falls back to the [MCP] tag", () => {
		const mcp = taskStore.create("legacy call", { source: "mcp" });
		taskStore.update(mcp.id, { status: "in_progress" });

		panel.setView("subagents");
		const text = renderPanel().join("\n");
		expect(text).toContain("⧉");
		expect(text).toContain("[MCP]");
	});

	test("subagents tree keeps each row's [mode]/[server] tag (no key-by-type collapse)", () => {
		const sub = taskStore.create("find the bug", { source: "subagent", subagentMode: "explore" });
		const mcp = taskStore.create("fetch", { source: "mcp", subagentMode: "web" });
		taskStore.update(sub.id, { status: "in_progress" });
		taskStore.update(mcp.id, { status: "in_progress" });
		panel.setView("subagents");

		const lines = renderPanel().map(stripAnsi);
		// Unlike the old grouped lens, the tree keeps every node's own origin tag.
		const subRow = lines.find((l) => l.includes("find the bug"));
		expect(subRow).toContain("[explore]");
		const mcpRow = lines.find((l) => l.includes("fetch"));
		expect(mcpRow).toContain("[web]");
	});

	test("an in-progress subagent row shows the owner's live tool activity", () => {
		taskStore.upsertAgent({
			id: "explore",
			name: "explore",
			kind: "subagent",
			state: "running",
			activity: "SearchCodebase",
		});
		const sub = taskStore.create("trace the auth flow", {
			source: "subagent",
			subagentMode: "explore",
			agent: "explore",
		});
		taskStore.update(sub.id, { status: "in_progress" });

		panel.setView("subagents");
		const row = renderPanel()
			.map(stripAnsi)
			.find((l) => l.includes("trace the auth flow"));
		// The live tool the subagent is running keeps the row reading as busy rather
		// than stuck, and a delegated row also carries its own run clock.
		expect(row).toContain("⋯ SearchCodebase");
		expect(row).toMatch(/⋯ SearchCodebase · \d/);
	});

	test("an idle in-progress subagent row keeps its run clock so a wedged run is visible", () => {
		taskStore.upsertAgent({ id: "explore", name: "explore", kind: "subagent", state: "running", activity: "" });
		const sub = taskStore.create("trace the auth flow", {
			source: "subagent",
			subagentMode: "explore",
			agent: "explore",
		});
		taskStore.update(sub.id, { status: "in_progress" });

		panel.setView("subagents");
		const row = renderPanel()
			.map(stripAnsi)
			.find((l) => l.includes("trace the auth flow"));
		// No activity to show, but the clock stays: it is the only signal that a
		// subagent which stopped reporting tools is still nominally running.
		expect(row).toMatch(/\d+(\.\d+)?s\s*$/);
		expect(row).not.toContain("⋯");
		expect(row).not.toContain("running…");
	});

	test("an in-progress main-plan row carries no run clock", () => {
		const t = taskStore.create("Patch the warning threshold");
		taskStore.update(t.id, { status: "in_progress" });

		const row = renderPanel()
			.map(stripAnsi)
			.find((l) => l.includes("Patch the warning threshold")) as string;
		// A TodoWrite item shares its createdAt with every other item in the plan, so
		// an elapsed time here would report the age of the PLAN, not of the task.
		expect(row).not.toMatch(/\ds/);
		expect(row.trimEnd()).toMatch(/threshold$/);
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
		expect(lines.length).toBe(3); // 3 rows, single lens so no header

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
		// VS15 (\ufe0e) rides along so terminals render the cue as single-cell
		// text, never a double-width emoji that would break the column math.
		expect(noteRow).toContain("\u26a0\ufe0e ran on inherited model");
		// The ⚠ cue takes over the right column, so the usage stamp is not shown.
		expect(noteRow).not.toContain("4.5k");

		const failRow = lines.find((l) => l.includes("Run suite"));
		expect(failRow).toContain("\u26a0\ufe0e anthropic exhausted");
	});

	test("long titles use the full left width regardless of task-id digit count", () => {
		const long = "A deliberately long task title that should fill the row up to the right column";
		const a = taskStore.create(long);
		taskStore.update(a.id, { status: "in_progress" });

		const width = 80;
		const row = renderPanel(width).find((l) => stripAnsi(l).includes("deliberately long"));
		expect(row).toBeDefined();
		// The row fills the full pane width: header + rows are padded to `width`.
		expect(visibleWidth(row as string)).toBe(width);
		// The title is truncated (too long to fit) but uses the full budget, so the
		// last visible title chars before the ellipsis reach deep into the row.
		expect(stripAnsi(row as string)).toMatch(/should fill the row\b/);
	});

	test("finished tasks show combined token usage per row", () => {
		const done = taskStore.create("Investigate flaky test");
		taskStore.update(done.id, {
			status: "done",
			usage: { input: 9000, output: 1100, cacheRead: 0, cacheWrite: 0, cost: 0.02 },
		});

		const lines = renderPanel();
		const row = lines.find((l) => l.includes("Investigate flaky test"));
		expect(row).toBeDefined();
		// 9000 + 1100 = 10100 → "10k".
		expect(row).toContain("10k");
		// The per-row stamp uses a single combined total, not split ↑/↓ arrows.
		expect(row).not.toContain("↑");
		expect(row).not.toContain("↓");
	});

	test("the pane carries no turn token/cost delta — that lives in the transcript", () => {
		const a = taskStore.create("Explore module", { source: "subagent", subagentMode: "explore" });
		taskStore.update(a.id, {
			status: "done",
			usage: { input: 3000, output: 500, cacheRead: 0, cacheWrite: 0, cost: 0.01 },
		});
		const b = taskStore.create("Run tests");
		taskStore.update(b.id, { status: "done" });

		const header = stripAnsi(renderPanel()[0] as string);
		// Header is the tab strip and nothing else: no turn delta, no state stamp,
		// no progress bar, no wall-clock.
		expect(header).not.toContain("turn");
		expect(header).not.toContain("$");
		expect(header).not.toContain("━");
		expect(header).not.toContain("WORKING");
		expect(header).not.toContain("REVIEWED");
	});

	test("each tab carries its own lens count so unfinished work elsewhere stays visible", () => {
		const plan = taskStore.create("Plain work");
		taskStore.update(plan.id, { status: "in_progress" });
		const sub = taskStore.create("trace the auth flow", { source: "subagent", subagentMode: "explore" });
		taskStore.update(sub.id, { status: "done" });

		const header = stripAnsi(renderPanel()[0] as string);
		expect(header).toContain("tasks 0/1");
		expect(header).toContain("subagents 1/1");
	});

	test("a filled tab marks the selected lens only while it has live work", () => {
		const plan = taskStore.create("Plain work");
		taskStore.update(plan.id, { status: "in_progress" });
		const sub = taskStore.create("trace the auth flow", { source: "subagent", subagentMode: "explore" });
		taskStore.update(sub.id, { status: "done" });

		const bgAnsi = theme.getBgAnsi("selectedBg");
		// flat lens is live → the selected tab fills.
		expect(renderPanel()[0]).toContain(bgAnsi);

		// Settle it: the tab keeps its bold accent label but drops the fill, so the
		// background means "running", not merely "here".
		taskStore.update(plan.id, { status: "done" });
		expect(renderPanel()[0]).not.toContain(bgAnsi);
	});

	test("header keeps the tab strip and drops the cycle hint on a narrow terminal", () => {
		const plan = taskStore.create("Explore module");
		taskStore.update(plan.id, { status: "done" });
		const sub = taskStore.create("trace the auth flow", { source: "subagent", subagentMode: "explore" });
		taskStore.update(sub.id, { status: "done" });

		const header = renderPanel(20)[0] as string;
		expect(visibleWidth(header)).toBeLessThanOrEqual(20);
		// The hint goes first; the tabs survive. This inverts the old priority, where
		// the switcher was dropped before the token counter.
		expect(stripAnsi(header)).not.toContain("cycle");
		expect(stripAnsi(header)).toContain("tasks");
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
		expect(stripAnsi(renderPanel().join("\n"))).toContain("Fresh task");
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
		expect(lines.length).toBe(6); // 6 rows, single lens so no header

		const text = lines.join("\n");
		expect(text).toContain("Task 0");
		expect(text).toContain("Task 5");
	});

	test("the header exists only when there is more than one lens to switch to", () => {
		const t = taskStore.create("Plain work");
		taskStore.update(t.id, { status: "in_progress" });

		// Plain session: only the flat lens has content. The header IS the switcher,
		// so with nothing to switch to the pane is rows only.
		expect(stripAnsi(renderPanel()[0] as string)).toContain("Plain work");
		expect(renderPanel().length).toBe(1);

		// Subagent work appears: the strip lists tasks + subagents, not teams, and a
		// cycle hint resolved from the configured keybinding sits at the right edge.
		taskStore.create("find the bug", { source: "subagent", subagentMode: "explore" });
		const two = stripAnsi(renderPanel()[0] as string);
		expect(two).toContain("tasks 0/1");
		expect(two).toContain("subagents 0/1");
		expect(two).not.toContain("teams");
		expect(two).toContain(`${appKeyLabel("app.tasks.cycleView")} cycle`);

		// A role agent registers (hooteams): the full strip shows.
		taskStore.upsertAgent({ id: "planner", name: "planner", kind: "role" });
		expect(stripAnsi(renderPanel()[0] as string)).toContain("teams 0/0");
	});

	test("cycleView advances through the lenses that have content", () => {
		taskStore.create("plan the work"); // main/TodoWrite task → flat lens has content
		taskStore.create("find the bug", { source: "subagent", subagentMode: "explore" });
		taskStore.upsertAgent({ id: "planner", name: "planner", kind: "role" });

		expect(panel.getView()).toBe("flat");
		expect(panel.cycleView()).toBe("subagents");
		expect(panel.cycleView()).toBe("teams");
		expect(panel.cycleView()).toBe("flat");
	});

	test("cycleView skips empty lenses", () => {
		// Nothing delegated: the cycle is a no-op on flat.
		const t = taskStore.create("Plain work");
		taskStore.update(t.id, { status: "in_progress" });
		expect(panel.cycleView()).toBe("flat");

		// Subagent work but no role agents: teams is skipped.
		taskStore.create("find the bug", { source: "subagent", subagentMode: "explore" });
		expect(panel.cycleView()).toBe("subagents");
		expect(panel.cycleView()).toBe("flat");
	});

	test("a selected lens that empties falls back to flat rendering", () => {
		const t = taskStore.create("Plain work");
		taskStore.update(t.id, { status: "in_progress" });

		// teams was selected but no role agents exist, so the lens resolves back to
		// flat — which leaves one lens and therefore no header at all.
		panel.setView("teams");
		const lines = renderPanel().map(stripAnsi);
		expect(lines.join("\n")).toContain("Plain work");
		expect(lines.length).toBe(1);

		// The next cycle press lands on a real lens, not a dead one.
		expect(panel.cycleView()).toBe("flat");
	});

	test("subagents view renders a recursive task tree (depth-2 nesting indented)", () => {
		// Root dispatched explore; explore in turn dispatched review, whose subtree
		// was merged back under explore via parentTaskId (cross-process propagation).
		const explore = taskStore.create("Explore module", { source: "subagent", subagentMode: "explore" });
		taskStore.update(explore.id, { status: "done" });
		const review = taskStore.create("Review findings", {
			source: "subagent",
			subagentMode: "review",
			parentTaskId: explore.id,
		});
		taskStore.update(review.id, { status: "done" });

		panel.setView("subagents");
		const lines = renderPanel().map(stripAnsi);
		// 2 task rows, no group headers in the tree, and no pane header: every task
		// here is delegated, so subagents is the only lens with content.
		expect(lines.length).toBe(2);

		const exploreRow = lines.find((l) => l.includes("Explore module")) as string;
		const reviewRow = lines.find((l) => l.includes("Review findings")) as string;
		// The root carries no connector (reads flat) and keeps its own [explore] tag.
		expect(exploreRow).not.toContain("└─");
		expect(exploreRow).not.toContain("├─");
		expect(exploreRow).toContain("[explore]");
		// The nested child is drawn with a └─ connector and keeps its own [review] tag.
		expect(reviewRow).toContain("└─");
		expect(reviewRow).toContain("[review]");
		// The child title is indented past the root (depth is visible).
		expect(reviewRow.indexOf("Review findings")).toBeGreaterThan(exploreRow.indexOf("Explore module"));
	});

	test("subagents view draws ├─/│ connectors for a sibling under a deeper parent", () => {
		const explore = taskStore.create("Explore module", { source: "subagent", subagentMode: "explore" });
		taskStore.update(explore.id, { status: "done" });
		const first = taskStore.create("First child", {
			source: "subagent",
			subagentMode: "edit",
			parentTaskId: explore.id,
		});
		taskStore.update(first.id, { status: "done" });
		const second = taskStore.create("Second child", {
			source: "subagent",
			subagentMode: "review",
			parentTaskId: explore.id,
		});
		taskStore.update(second.id, { status: "done" });

		panel.setView("subagents");
		const lines = renderPanel().map(stripAnsi);
		// A non-last child uses ├─; the last child uses └─.
		const firstRow = lines.find((l) => l.includes("First child")) as string;
		const secondRow = lines.find((l) => l.includes("Second child")) as string;
		expect(firstRow).toContain("├─");
		expect(secondRow).toContain("└─");
	});

	test("subagents tree with only top-level tasks renders flat (no extra indent)", () => {
		const a = taskStore.create("Explore A", { source: "subagent", subagentMode: "explore" });
		const b = taskStore.create("Explore B", { source: "subagent", subagentMode: "explore" });
		taskStore.update(a.id, { status: "in_progress" });
		taskStore.update(b.id, { status: "in_progress" });

		panel.setView("subagents");
		const lines = renderPanel().map(stripAnsi);
		expect(lines.length).toBe(2); // 2 roots, single lens so no header
		const rowA = lines.find((l) => l.includes("Explore A")) as string;
		const rowB = lines.find((l) => l.includes("Explore B")) as string;
		// Roots carry no tree connectors: they read exactly like flat rows.
		for (const row of [rowA, rowB]) {
			expect(row).not.toContain("└─");
			expect(row).not.toContain("├─");
		}
		// Sibling roots start at the same column (no indent drift).
		expect(rowA.indexOf("Explore A")).toBe(rowB.indexOf("Explore B"));
	});

	test("the tasks and subagents lenses split work by ownership", () => {
		const plan = taskStore.create("Write the plan"); // main TodoWrite plan
		taskStore.update(plan.id, { status: "in_progress" });
		const sub = taskStore.create("Explore the API", { source: "subagent", subagentMode: "explore" });
		taskStore.update(sub.id, { status: "in_progress" });
		const mcp = taskStore.create("Fetch the spec", { source: "mcp", subagentMode: "web" });
		taskStore.update(mcp.id, { status: "in_progress" });

		// flat ("tasks"): only the main agent's own plan.
		panel.setView("flat");
		const flat = renderPanel().map(stripAnsi).join("\n");
		expect(flat).toContain("Write the plan");
		expect(flat).not.toContain("Explore the API");
		expect(flat).not.toContain("Fetch the spec");

		// subagents: only the delegated/MCP work, never the main plan.
		panel.setView("subagents");
		const sa = renderPanel().map(stripAnsi).join("\n");
		expect(sa).not.toContain("Write the plan");
		expect(sa).toContain("Explore the API");
		expect(sa).toContain("Fetch the spec");
	});

	test("each tab's count is scoped to its own lens, not all tasks", () => {
		const plan = taskStore.create("Write the plan");
		taskStore.update(plan.id, { status: "in_progress" });
		const sub = taskStore.create("Explore the API", { source: "subagent", subagentMode: "explore" });
		taskStore.update(sub.id, { status: "done" });
		const mcp = taskStore.create("Fetch the spec", { source: "mcp", subagentMode: "web" });
		taskStore.update(mcp.id, { status: "done" });

		// 3 total tasks: 1 main (in_progress) + 2 delegated (done). Both counts are
		// present at once, whichever lens is selected — that is the point of the
		// per-tab count: unfinished work in the other lens stays visible.
		panel.setView("flat");
		const flatHeader = stripAnsi(renderPanel()[0]);
		expect(flatHeader).toContain("tasks 0/1");
		expect(flatHeader).toContain("subagents 2/2");
		expect(flatHeader).not.toContain("0/3");

		panel.setView("subagents");
		const saHeader = stripAnsi(renderPanel()[0]);
		expect(saHeader).toContain("tasks 0/1");
		expect(saHeader).toContain("subagents 2/2");
	});

	test("an empty flat lens falls through to the subagents tree", () => {
		// Only delegated work exists (no main TodoWrite task), so the default flat
		// lens has nothing to draw and the pane shows the subagents tree instead.
		const sub = taskStore.create("Explore the API", { source: "subagent", subagentMode: "explore" });
		taskStore.update(sub.id, { status: "in_progress" });

		// Stored view is still flat, but flat has no content → falls through.
		expect(panel.getView()).toBe("flat");
		const text = renderPanel().map(stripAnsi).join("\n");
		expect(text).toContain("Explore the API");
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
		// header + planner-header + planner-task + connector + builder-header + builder-task
		expect(lines.length).toBe(6);

		const plannerHeader = lines[1] as string;
		expect(plannerHeader).toContain("▸ planner");
		expect(plannerHeader).toContain("· architect");
		expect(plannerHeader).toContain("[done]");
		expect(plannerHeader).toContain("→ builder");
		expect(plannerHeader).toContain("↑1.4k ↓260 · $0.004");

		// Connector line injected after planner's task row (handoff "→ builder", builder exists).
		expect(lines.some((l) => l.includes("└──→") && l.includes("builder"))).toBe(true);

		const builderHeader = lines[4] as string;
		expect(builderHeader).toContain("▸ builder");
		expect(builderHeader).toContain("[active]");
		expect(builderHeader).toMatch(/0\/1\s*$/);

		// Rows are still padded to the full pane width in grouped views.
		for (const line of renderPanel()) {
			expect(visibleWidth(line)).toBe(120);
		}
	});

	test("subagents tree shows delegated work and excludes role-agent tasks", () => {
		taskStore.upsertAgent({ id: "planner", name: "planner", kind: "role" });

		const workerTask = taskStore.create("worker task", {
			source: "subagent",
			subagentMode: "build",
			agent: "worker",
		});
		taskStore.update(workerTask.id, { status: "in_progress" });
		const plannerTask = taskStore.create("planner task", { agent: "planner" });
		taskStore.update(plannerTask.id, { status: "in_progress" });

		panel.setView("subagents");
		const text = renderPanel().map(stripAnsi).join("\n");

		// A role task (no subagent/MCP source) must not appear in the subagents tree.
		expect(text).not.toContain("planner task");
		// Delegated (subagent-sourced) work appears.
		expect(text).toContain("worker task");
	});

	test("teams view hides non-role-agent groups", () => {
		taskStore.upsertAgent({ id: "main-ag", name: "main-ag", kind: "main" });
		taskStore.upsertAgent({ id: "worker", name: "worker", kind: "subagent" });
		taskStore.upsertAgent({ id: "architect", name: "architect", kind: "role" });

		const mainTask = taskStore.create("main task", { agent: "main-ag" });
		taskStore.update(mainTask.id, { status: "in_progress" });
		const workerTask = taskStore.create("worker task", { agent: "worker" });
		taskStore.update(workerTask.id, { status: "in_progress" });
		const archTask = taskStore.create("arch task", { agent: "architect" });
		taskStore.update(archTask.id, { status: "in_progress" });

		panel.setView("teams");
		const text = renderPanel().map(stripAnsi).join("\n");

		// Non-role task titles must not appear.
		expect(text).not.toContain("main task");
		expect(text).not.toContain("worker task");
		// Role agent task must appear.
		expect(text).toContain("arch task");
		expect(text).toContain("architect");
	});

	test("teams view renders queued role placeholder with no tasks", () => {
		taskStore.upsertAgent({
			id: "queued-role",
			name: "queued-role",
			role: "pending",
			kind: "role",
			state: "queued",
		});

		panel.setView("teams");
		const lines = renderPanel().map(stripAnsi);
		// pane header + one group header for the queued agent (no task rows).
		expect(lines.length).toBe(2);
		expect(lines.some((l) => l.includes("queued-role") && l.includes("0/0"))).toBe(true);
	});

	test("teams view renders idle role placeholder with no tasks", () => {
		taskStore.upsertAgent({
			id: "team:planner",
			name: "planner",
			kind: "role",
			state: "idle",
		});

		panel.setView("teams");
		const lines = renderPanel().map(stripAnsi);
		// pane header + one group header for the idle role (no task rows).
		expect(lines.length).toBe(2);
		expect(lines.some((l) => l.includes("planner") && l.includes("[idle]") && l.includes("0/0"))).toBe(true);
	});

	test("empty flat view falls through to the team roster at startup", () => {
		// --team registers idle roles before any task exists; the default flat
		// lens has nothing to draw, so the pane shows the teams roster instead
		// of collapsing.
		taskStore.upsertAgent({ id: "team:planner", name: "planner", kind: "role", state: "idle" });
		taskStore.upsertAgent({ id: "team:builder", name: "builder", kind: "role", state: "idle" });

		const lines = renderPanel().map(stripAnsi);
		// pane header + one group header per idle role.
		expect(lines.length).toBe(3);
		expect(lines.some((l) => l.includes("planner"))).toBe(true);
		expect(lines.some((l) => l.includes("builder"))).toBe(true);

		// The fallback never sticks: once a task exists the stored flat lens resumes.
		const task = taskStore.create("Init project");
		taskStore.update(task.id, { status: "in_progress" });
		const flatLines = renderPanel().map(stripAnsi);
		expect(flatLines.some((l) => l.includes("Init project"))).toBe(true);
		// Roster placeholders belong to the teams lens, not flat.
		expect(flatLines.some((l) => l.includes("planner"))).toBe(false);
	});

	test("subagents tree renders delegated roots without a roster", () => {
		const sub = taskStore.create("find the bug", { source: "subagent", subagentMode: "explore" });
		taskStore.update(sub.id, { status: "in_progress" });

		panel.setView("subagents");
		const lines = renderPanel().map(stripAnsi);
		// The one delegated root; no group headers, no roster needed, and no pane
		// header — subagents is the only lens with content.
		expect(lines.length).toBe(1);
		expect(lines[0]).toContain("find the bug");
		expect(lines[0]).toContain("[explore]");
		expect(lines[0]).toContain("◇");
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

describe("task store", () => {
	beforeEach(() => {
		taskStore.clear();
	});

	test("addAgentStats warns on unknown agent id", () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		taskStore.addAgentStats("nonexistent", { input: 100 });
		expect(warn).toHaveBeenCalledWith(expect.stringMatching(/unknown agent id/));
		warn.mockRestore();
	});

	test("update warns on unknown task id", () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		taskStore.update(9999, { status: "done" });
		expect(warn).toHaveBeenCalledWith(expect.stringMatching(/unknown task id/));
		warn.mockRestore();
	});

	test("reset preserves role agents with non-zero stats but drops settled subagent runs", () => {
		// Create a task so reset() actually runs (not early-return path).
		const dummy = taskStore.create("dummy");
		taskStore.update(dummy.id, { status: "done" });
		// Role agents carry cross-turn team cost accounting — preserved with stats.
		taskStore.upsertAgent({ id: "team:planner", name: "planner", kind: "role" });
		taskStore.addAgentStats("team:planner", { input: 100, output: 50, cost: 0.001 });
		// Subagent rows are per-run; a settled run must not outlive its task even
		// with stats, or the roster would grow without bound across turns.
		taskStore.upsertAgent({ id: "run-1", name: "explore#1", kind: "subagent", state: "done" });
		taskStore.addAgentStats("run-1", { input: 200, output: 20, cost: 0.002 });

		taskStore.reset();
		expect(taskStore.agents().some((a) => a.id === "team:planner")).toBe(true);
		expect(taskStore.agents().some((a) => a.id === "run-1")).toBe(false);

		// Register a zero-stats role agent; create + finish a dummy to force reset
		// to run (nextId is 1 after the previous reset, so a new task makes it 2).
		taskStore.upsertAgent({ id: "team:zero", name: "zero", kind: "role" });
		const dummy2 = taskStore.create("dummy2");
		taskStore.update(dummy2.id, { status: "done" });
		taskStore.reset();
		// Zero-stats agent is dropped.
		expect(taskStore.agents().some((a) => a.id === "team:zero")).toBe(false);
		// Stats-carrying role agent still preserved.
		expect(taskStore.agents().some((a) => a.id === "team:planner")).toBe(true);
	});
});
