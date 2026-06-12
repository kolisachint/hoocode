import { beforeEach, describe, expect, test } from "vitest";
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
