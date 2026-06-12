import { beforeEach, describe, expect, test } from "vitest";
import { taskStore } from "../src/core/task-store.js";
import { TaskPanelComponent } from "../src/modes/interactive/components/task-panel.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

function stripAnsi(text: string): string {
	return text.replace(/\x1b\[[0-9;]*m/g, "");
}

const DOWN = "\x1b[B";
const UP = "\x1b[A";

describe("task panel team focus", () => {
	beforeEach(() => {
		initTheme("dark");
		taskStore.clear();
		taskStore.upsertAgent({ id: "team:planner", name: "planner", kind: "role", state: "idle" });
		taskStore.upsertAgent({ id: "team:coder", name: "coder", kind: "role", state: "active" });
	});

	test("focused panel renders the teams roster with a ▶ cursor on the selected role", () => {
		const panel = new TaskPanelComponent();
		panel.focused = true;
		const lines = panel.render(80).map(stripAnsi);
		const plannerRow = lines.find((line) => line.includes("planner"));
		const coderRow = lines.find((line) => line.includes("coder"));
		expect(plannerRow).toContain("▶");
		expect(coderRow).toContain("▸");
		// Team-focus key hints are visible while focused.
		expect(lines.join("\n")).toContain("n nudge");
	});

	test("unfocused panel keeps the plain ▸ role glyph and no hints", () => {
		const panel = new TaskPanelComponent();
		panel.setView("teams");
		const text = panel.render(80).map(stripAnsi).join("\n");
		expect(text).not.toContain("▶");
		expect(text).not.toContain("n nudge");
	});

	test("↑/↓ move the cursor and clamp at the roster edges", () => {
		const panel = new TaskPanelComponent();
		panel.focused = true;
		expect(panel.focusedRole()).toBe("planner");
		panel.handleInput(DOWN);
		expect(panel.focusedRole()).toBe("coder");
		panel.handleInput(DOWN);
		expect(panel.focusedRole()).toBe("coder");
		panel.handleInput(UP);
		expect(panel.focusedRole()).toBe("planner");
		panel.handleInput(UP);
		expect(panel.focusedRole()).toBe("planner");
	});

	test("n nudges and a attaches the focused role; q and escape exit focus", () => {
		const panel = new TaskPanelComponent();
		panel.focused = true;
		const nudged: string[] = [];
		const attached: string[] = [];
		let exits = 0;
		panel.onNudge = (role) => nudged.push(role);
		panel.onAttach = (role) => attached.push(role);
		panel.onExitFocus = () => exits++;

		panel.handleInput("n");
		panel.handleInput(DOWN);
		panel.handleInput("a");
		expect(nudged).toEqual(["planner"]);
		expect(attached).toEqual(["coder"]);

		panel.handleInput("q");
		panel.handleInput("\x1b");
		expect(exits).toBe(2);
	});

	test("the cursor follows its role by name when the roster reorders", () => {
		const panel = new TaskPanelComponent();
		panel.focused = true;
		panel.handleInput(DOWN); // -> coder
		expect(panel.focusedRole()).toBe("coder");
		taskStore.clear();
		taskStore.upsertAgent({ id: "team:coder", name: "coder", kind: "role", state: "active" });
		taskStore.upsertAgent({ id: "team:planner", name: "planner", kind: "role", state: "idle" });
		expect(panel.focusedRole()).toBe("coder");
	});

	test("an emptied roster exits focus instead of trapping the keyboard", () => {
		const panel = new TaskPanelComponent();
		panel.focused = true;
		let exits = 0;
		panel.onExitFocus = () => exits++;
		taskStore.clear();
		panel.handleInput(DOWN);
		expect(exits).toBe(1);
	});
});
