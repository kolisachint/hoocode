import { setKeybindings } from "@kolisachint/hoocode-tui";
import { beforeEach, describe, expect, test } from "vitest";
import { KeybindingsManager } from "../src/core/keybindings.js";
import type { TeamViewConnection, TeamViewEvent } from "../src/core/team-view.js";
import { RingBuffer, TeamAttachPanelComponent } from "../src/modes/interactive/components/team-attach-panel.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

function stripAnsi(text: string): string {
	return text.replace(/\x1b\[[0-9;]*m/g, "");
}

/** In-memory stand-in for the Phase 3 team client's shared event stream. */
function fakeConnection(): { connection: TeamViewConnection; emit: (event: TeamViewEvent) => void } {
	const listeners = new Set<(event: TeamViewEvent) => void>();
	return {
		connection: {
			stop() {},
			async steer() {},
			async resume() {},
			pendingApprovals: async () => [],
			subscribe(listener) {
				listeners.add(listener);
				return () => {
					listeners.delete(listener);
				};
			},
			subscriberCount: () => listeners.size,
		},
		emit(event) {
			for (const listener of [...listeners]) listener(event);
		},
	};
}

describe("RingBuffer", () => {
	test("keeps only the newest entries once capacity is reached", () => {
		const buffer = new RingBuffer<number>(3);
		for (let i = 1; i <= 5; i++) buffer.push(i);
		expect(buffer.length).toBe(3);
		expect(buffer.toArray()).toEqual([3, 4, 5]);
	});

	test("rejects nonsense capacities", () => {
		expect(() => new RingBuffer(0)).toThrow();
		expect(() => new RingBuffer(-1)).toThrow();
	});
});

describe("TeamAttachPanelComponent", () => {
	beforeEach(() => {
		initTheme("dark");
		setKeybindings(new KeybindingsManager());
	});

	test("attach → detach → attach 10× leaves zero leaked subscribers", () => {
		const { connection } = fakeConnection();
		for (let i = 0; i < 10; i++) {
			const panel = new TeamAttachPanelComponent("coder", connection, {
				onDetach: () => {},
				onNudge: () => {},
			});
			expect(connection.subscriberCount()).toBe(1);
			panel.dispose();
			expect(connection.subscriberCount()).toBe(0);
		}
		expect(connection.subscriberCount()).toBe(0);
	});

	test("dispose is idempotent", () => {
		const { connection } = fakeConnection();
		const panel = new TeamAttachPanelComponent("coder", connection, { onDetach: () => {}, onNudge: () => {} });
		panel.dispose();
		panel.dispose();
		expect(connection.subscriberCount()).toBe(0);
	});

	test("renders only the attached role's events, StreamRenderer style", () => {
		const { connection, emit } = fakeConnection();
		const panel = new TeamAttachPanelComponent("coder", connection, { onDetach: () => {}, onNudge: () => {} });

		emit({ type: "agent_start", role: "coder" });
		emit({ type: "agent_start", role: "planner" });
		emit({ type: "message_update", role: "coder", assistantMessageEvent: { type: "text_delta", delta: "hel" } });
		emit({ type: "message_update", role: "coder", assistantMessageEvent: { type: "text_delta", delta: "lo\n" } });
		emit({ type: "tool_execution_start", role: "coder", toolName: "bash", args: { cmd: "ls" } });
		emit({ type: "tool_execution_end", role: "coder", toolName: "bash", isError: false });
		emit({ type: "tool_execution_end", role: "planner", toolName: "read", isError: true });
		emit({
			type: "turn_end",
			role: "coder",
			message: { role: "assistant", usage: { input: 10, output: 5, cost: { total: 0.1234 } } },
		});
		emit({ type: "agent_end", role: "coder" });

		const text = panel.render(80).map(stripAnsi).join("\n");
		expect(text).toContain("◉ coder — attached");
		expect(text).toContain("◉ coder started");
		expect(text).toContain("hello");
		expect(text).toContain("◉ tool: bash");
		expect(text).toContain("✓ bash done");
		expect(text).toContain("— turn: 10 in / 5 out tokens $0.1234");
		expect(text).toContain("◉ coder idle");
		// The other role's stream never leaks in.
		expect(text).not.toContain("planner");
		expect(text).not.toContain("✗ read failed");
		panel.dispose();
	});

	test("streaming text not yet newline-terminated is still visible", () => {
		const { connection, emit } = fakeConnection();
		const panel = new TeamAttachPanelComponent("coder", connection, { onDetach: () => {}, onNudge: () => {} });
		emit({
			type: "message_update",
			role: "coder",
			assistantMessageEvent: { type: "text_delta", delta: "partial tail" },
		});
		const text = panel.render(80).map(stripAnsi).join("\n");
		expect(text).toContain("partial tail");
		panel.dispose();
	});

	test("the event buffer is bounded", () => {
		const { connection, emit } = fakeConnection();
		const panel = new TeamAttachPanelComponent(
			"coder",
			connection,
			{ onDetach: () => {}, onNudge: () => {} },
			undefined,
			25,
		);
		for (let i = 0; i < 500; i++) emit({ type: "agent_start", role: "coder" });
		expect(panel.bufferedLineCount()).toBe(25);
		panel.dispose();
	});

	test("task lifecycle events render as stream lines, filtered by role", () => {
		const { connection, emit } = fakeConnection();
		const panel = new TeamAttachPanelComponent("coder", connection, { onDetach: () => {}, onNudge: () => {} });

		emit({ type: "task_started", role: "coder", taskId: "t1" });
		emit({ type: "task_paused", role: "coder", taskId: "t1", question: "Deploy to production?" });
		emit({ type: "task_resumed", role: "coder", taskId: "t1", chosenOption: "no" });
		emit({ type: "task_finished", role: "coder", taskId: "t1", status: "done" });
		emit({ type: "task_finished", role: "planner", taskId: "t2", status: "error" });

		const text = panel.render(80).map(stripAnsi).join("\n");
		expect(text).toContain("◉ task t1 started");
		expect(text).toContain("⏸ awaiting approval: Deploy to production?");
		expect(text).toContain("▶ task t1 resumed: no");
		expect(text).toContain("✓ task t1 done");
		expect(text).not.toContain("t2");
		panel.dispose();
	});

	test("presentApproval embeds the gate and resolves with the picked option", async () => {
		const { connection } = fakeConnection();
		let detached = 0;
		const panel = new TeamAttachPanelComponent("coder", connection, {
			onDetach: () => detached++,
			onNudge: () => {},
		});

		const promise = panel.presentApproval(
			{ taskId: "t1", question: "Deploy to production?", options: ["yes", "no"], role: "coder" },
			new AbortController().signal,
		);

		let text = panel.render(80).map(stripAnsi).join("\n");
		expect(text).toContain("INPUT NEEDED");
		expect(text).toContain("Deploy to production?");
		expect(text).toContain("1 yes");
		expect(text).toContain("2 no");

		// The gate owns the keyboard: q must not detach while it is open.
		panel.handleInput("q");
		expect(detached).toBe(0);

		panel.handleInput("2"); // quick-pick "no"
		await expect(promise).resolves.toBe("no");

		text = panel.render(80).map(stripAnsi).join("\n");
		expect(text).not.toContain("INPUT NEEDED");
		expect(text).toContain("✓ answered: no");

		// With the gate settled, panel keys work again.
		panel.handleInput("q");
		expect(detached).toBe(1);
		panel.dispose();
	});

	test("esc skips the gate without detaching and leaves no answered stamp", async () => {
		const { connection } = fakeConnection();
		let detached = 0;
		const panel = new TeamAttachPanelComponent("coder", connection, {
			onDetach: () => detached++,
			onNudge: () => {},
		});

		const promise = panel.presentApproval(
			{ taskId: "t1", question: "Deploy?", options: ["yes"] },
			new AbortController().signal,
		);
		panel.handleInput("\x1b");
		await expect(promise).resolves.toBeUndefined();
		expect(detached).toBe(0);

		const text = panel.render(80).map(stripAnsi).join("\n");
		expect(text).not.toContain("INPUT NEEDED");
		expect(text).not.toContain("answered:");
		panel.dispose();
	});

	test("the abort signal (answered elsewhere) dismisses the gate", async () => {
		const { connection } = fakeConnection();
		const panel = new TeamAttachPanelComponent("coder", connection, { onDetach: () => {}, onNudge: () => {} });

		const controller = new AbortController();
		const promise = panel.presentApproval({ taskId: "t1", question: "Deploy?", options: ["yes"] }, controller.signal);
		controller.abort();
		await expect(promise).resolves.toBeUndefined();

		const text = panel.render(80).map(stripAnsi).join("\n");
		expect(text).not.toContain("INPUT NEEDED");
		panel.dispose();
	});

	test("dispose settles a pending gate as skipped", async () => {
		const { connection } = fakeConnection();
		const panel = new TeamAttachPanelComponent("coder", connection, { onDetach: () => {}, onNudge: () => {} });

		const promise = panel.presentApproval(
			{ taskId: "t1", question: "Deploy?", options: ["yes"] },
			new AbortController().signal,
		);
		panel.dispose();
		await expect(promise).resolves.toBeUndefined();
	});

	test("presenting on a disposed panel resolves immediately", async () => {
		const { connection } = fakeConnection();
		const panel = new TeamAttachPanelComponent("coder", connection, { onDetach: () => {}, onNudge: () => {} });
		panel.dispose();
		await expect(
			panel.presentApproval({ taskId: "t1", question: "Deploy?", options: ["yes"] }, new AbortController().signal),
		).resolves.toBeUndefined();
	});

	test("q detaches, n nudges the attached role", () => {
		const { connection } = fakeConnection();
		let detached = 0;
		const nudged: string[] = [];
		const panel = new TeamAttachPanelComponent("coder", connection, {
			onDetach: () => detached++,
			onNudge: (role) => nudged.push(role),
		});
		panel.handleInput("q");
		expect(detached).toBe(1);
		panel.handleInput("n");
		expect(nudged).toEqual(["coder"]);
		// Escape detaches too (tui.select.cancel).
		panel.handleInput("\x1b");
		expect(detached).toBe(2);
		panel.dispose();
	});
});
