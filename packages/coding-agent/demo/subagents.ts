/**
 * Demo: subagents — options pane + task pane + TodoWrite, with depth-2 subagents
 * -----------------------------------------------------------------------------
 * One interactive demo stitched from the REAL interactive-mode internals:
 *   - `AskOptionsComponent` — the inline "INPUT NEEDED" options pane (ask_options)
 *   - `TaskPanelComponent`  — the live task/subagent ledger
 *   - `taskStore`           — the singleton both the agent and the pane read from
 * Nothing is reimplemented; the demo writes tasks/agents into the store exactly
 * the way a running session does, and the real components render them.
 *
 * Flow:
 *   1. The agent asks you a scoping question (the options pane).
 *   2. It lays out a TodoWrite plan (the "tasks" lens).
 *   3. It dispatches subagents — and one of them dispatches a subagent of its
 *      own, so the subagents lens shows a depth-2 tree (◇ explore → ◇ scan).
 *
 * Keys: ↑/↓ · → next · ← back · 1-9 pick (options pane) · Ctrl+N swaps the pane
 * between tasks and agents · Ctrl+C exits.
 *
 * Run:  npx tsx packages/coding-agent/demo/subagents.ts
 */

import { matchesKey, ProcessTerminal, setKeybindings, Text, TUI } from "@kolisachint/hoocode-tui";
import type { AskQuestion } from "../src/core/extensions/types.js";
import { KeybindingsManager } from "../src/core/keybindings.js";
import { taskStore } from "../src/core/task-store.js";
import { AskOptionsComponent } from "../src/modes/interactive/components/ask-options.js";
import { TaskPanelComponent } from "../src/modes/interactive/components/task-panel.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

initTheme("dark");
// The options pane reads app bindings (app.options.next = →, app.options.back = ←).
setKeybindings(KeybindingsManager.create());

const terminal = new ProcessTerminal();
const tui = new TUI(terminal);

const header = new Text("hoocode · subagents demo (options pane → task pane, depth-2 subagents)");
const hint = new Text("Ctrl+N swaps the pane between tasks and agents · Ctrl+C exits\n");
tui.addChild(header);
tui.addChild(hint);

const panel = new TaskPanelComponent(tui);
const status = new Text("");

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const set = (id: number, status: "pending" | "in_progress" | "done" | "failed") => taskStore.update(id, { status });
const cost = (input: number, output: number, c: number) => ({ input, output, cacheRead: 0, cacheWrite: 0, cost: c });

// ── Stage 1: the options pane (the agent asks before it acts) ──────────────────
const questions: AskQuestion[] = [
	{
		question: "How thorough should this change be?",
		short: "Scope",
		detail: "Controls how many subagents I dispatch.",
		options: [
			{ label: "Standard", description: "explore (+ nested scan) then review", recommended: true },
			{ label: "Deep", description: "explore, nested scan, and a longer review" },
			{ label: "Quick", description: "explore only — skip the review" },
		],
		allowCustom: true,
	},
];

const ask = new AskOptionsComponent(
	questions,
	(answers) => {
		const choice = answers[0] ?? "Standard";
		tui.removeChild(ask);
		tui.setFocus(null);
		status.setText(`Scope: ${choice} — dispatching subagents…\n`);
		tui.addChild(panel);
		tui.addChild(status);
		tui.requestRender();
		void run(choice);
	},
	() => {
		void run("Quick");
	},
);

tui.addChild(ask);
tui.setFocus(ask);

// ── Stage 2+3: the TodoWrite plan and the depth-2 subagent run ─────────────────
async function run(choice: string) {
	const withReview = !/^quick/i.test(choice);

	// Main agent + its TodoWrite plan → the "tasks" lens.
	taskStore.upsertAgent({ id: "main", name: "hoocode", role: "orchestrator", kind: "main", state: "running" });
	const survey = taskStore.create("Survey the codebase");
	const implement = taskStore.create("Implement provider handoff");
	const test = taskStore.create("Add cross-provider tests");
	taskStore.create("Update the docs");
	tui.requestRender();
	await wait(900);

	// Depth 1: dispatch the `explore` subagent (a root of the delegated tree).
	set(survey.id, "in_progress");
	taskStore.upsertAgent({ id: "explore", name: "explore", role: "subagent", kind: "subagent", state: "running" });
	const exploreCall = taskStore.create("explore: map the ai providers", {
		source: "subagent",
		agent: "explore",
		subagentMode: "explore",
	});
	const e1 = taskStore.create("list provider modules", {
		source: "subagent",
		agent: "explore",
		parentTaskId: exploreCall.id,
	});
	tui.requestRender();
	await wait(1000);

	// Depth 2: `explore` itself dispatches a `scan` subagent (nested one level deeper).
	taskStore.upsertAgent({ id: "scan", name: "scan", role: "subagent", kind: "subagent", state: "running" });
	const scanCall = taskStore.create("scan: grep for stream() impls", {
		source: "subagent",
		agent: "scan",
		subagentMode: "scan",
		parentTaskId: exploreCall.id,
	});
	const s1 = taskStore.create("read openai-responses.ts", {
		source: "subagent",
		agent: "scan",
		parentTaskId: scanCall.id,
	});
	const s2 = taskStore.create("read anthropic.ts", { source: "subagent", agent: "scan", parentTaskId: scanCall.id });
	tui.requestRender();
	await wait(1200);

	// scan finishes, bubbling up to explore.
	set(s1.id, "done");
	set(s2.id, "done");
	set(scanCall.id, "done");
	taskStore.upsertAgent({
		id: "scan",
		name: "scan",
		kind: "subagent",
		state: "done",
		stats: { input: 4300, output: 700, cost: 0.03 },
	});
	set(e1.id, "done");
	taskStore.update(exploreCall.id, { status: "done", usage: cost(8200, 1400, 0.06) });
	taskStore.upsertAgent({
		id: "explore",
		name: "explore",
		kind: "subagent",
		state: "done",
		stats: { input: 12500, output: 2100, cost: 0.09 },
	});
	set(survey.id, "done");
	set(implement.id, "in_progress");
	tui.requestRender();
	await wait(1000);

	// Optional review subagent (skipped on "Quick").
	set(implement.id, "done");
	if (withReview) {
		set(test.id, "in_progress");
		taskStore.upsertAgent({ id: "reviewer", name: "reviewer", role: "subagent", kind: "subagent", state: "running" });
		const reviewCall = taskStore.create("reviewer: audit the handoff diff", {
			source: "subagent",
			agent: "reviewer",
			subagentMode: "reviewer",
		});
		const r1 = taskStore.create("check message ordering", {
			source: "subagent",
			agent: "reviewer",
			parentTaskId: reviewCall.id,
		});
		const r2 = taskStore.create("flag missing abort test", {
			source: "subagent",
			agent: "reviewer",
			parentTaskId: reviewCall.id,
		});
		taskStore.update(r2.id, { note: "follow-up" });
		tui.requestRender();
		await wait(1200);
		set(r1.id, "done");
		set(r2.id, "done");
		taskStore.update(reviewCall.id, { status: "done", usage: cost(5100, 900, 0.04) });
		taskStore.upsertAgent({
			id: "reviewer",
			name: "reviewer",
			kind: "subagent",
			state: "done",
			stats: { input: 5100, output: 900, cost: 0.04 },
		});
		set(test.id, "done");
	} else {
		set(test.id, "failed");
		taskStore.update(test.id, { note: "skipped (quick)" });
	}

	for (const t of taskStore.list()) if (t.status === "pending") set(t.id, "done");
	taskStore.upsertAgent({ id: "main", name: "hoocode", kind: "main", state: "done" });
	status.setText("Run complete. Press Ctrl+N to swap between the tasks and agents lenses. Ctrl+C to exit.\n");
	tui.requestRender();
}

tui.addInputListener((data) => {
	if (matchesKey(data, "ctrl+c")) {
		panel.dispose();
		tui.stop();
		process.exit(0);
	}
	if (matchesKey(data, "ctrl+n")) {
		panel.cycleView();
		return { consume: true };
	}
	return undefined;
});

tui.start();
