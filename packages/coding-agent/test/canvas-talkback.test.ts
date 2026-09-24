/**
 * A canvas talking back: when its `session.send` reaches the agent, and what it
 * hears through `session.on`.
 *
 * Both halves are pure policy (`core/canvas/inbox.ts`, `core/canvas/events.ts`)
 * with injected effects, so these run with no session, no child and no clock.
 */

import { describe, expect, it } from "vitest";
import { CanvasEventSource, toolTitle } from "../src/core/canvas/events.js";
import {
	CANVAS_FLUSH_RETRIES,
	CANVAS_MAX_UNATTENDED_TURNS,
	CANVAS_STEER_INTERVAL_MS,
	CanvasInbox,
} from "../src/core/canvas/inbox.js";
import type { CanvasSessionEvent } from "../src/core/canvas/protocol.js";

function harness(initiallyIdle = true) {
	const state = { idle: initiallyIdle, now: 0 };
	const delivered: Array<{ text: string; deliverAs: string }> = [];
	const notices: string[] = [];
	const timers: Array<() => void> = [];
	const inbox = new CanvasInbox({
		isIdle: () => state.idle,
		deliver: (text, deliverAs) => delivered.push({ text, deliverAs }),
		notify: (message) => notices.push(message),
		now: () => state.now,
		schedule: (callback) => timers.push(callback),
	});
	const runTimers = () => {
		while (timers.length > 0) timers.shift()?.();
	};
	return { state, delivered, notices, inbox, runTimers };
}

describe("canvas inbox", () => {
	it("starts a turn at once when the agent is idle, labelled with the canvas", () => {
		const { inbox, delivered } = harness();
		expect(inbox.submit("drawio-canvas", "review my changes")).toBe("started");
		expect(delivered).toEqual([{ text: "[canvas drawio-canvas] review my changes", deliverAs: "followUp" }]);
	});

	it("holds messages while the agent is busy, the latest replacing the older", () => {
		const { inbox, delivered, state, runTimers } = harness(false);
		expect(inbox.submit("board", "2 open asks")).toBe("queued");
		expect(inbox.submit("board", "3 open asks")).toBe("queued");
		expect(delivered).toEqual([]);
		state.idle = true;
		inbox.agentEnded();
		runTimers();
		expect(delivered).toEqual([{ text: "[canvas board] 3 open asks", deliverAs: "followUp" }]);
	});

	it("delivers held messages from several canvases as one message", () => {
		const { inbox, delivered, state } = harness(false);
		inbox.submit("a", "one");
		inbox.submit("b", "two");
		state.idle = true;
		inbox.agentEnded();
		expect(delivered).toEqual([{ text: "[canvas a] one\n\n[canvas b] two", deliverAs: "followUp" }]);
	});

	it("waits for the session to settle after agent_end, and gives up if a new turn starts", () => {
		const { inbox, delivered, state, runTimers } = harness(false);
		inbox.submit("a", "one");
		inbox.agentEnded();
		expect(delivered).toEqual([]);
		// Still busy for every retry: the person started a turn. Nothing is lost.
		for (let i = 0; i <= CANVAS_FLUSH_RETRIES; i += 1) runTimers();
		expect(delivered).toEqual([]);
		expect(inbox.waiting()).toBe(1);
		state.idle = true;
		inbox.agentEnded();
		expect(delivered).toHaveLength(1);
	});

	it("steers with immediate, at most once per interval per canvas", () => {
		const { inbox, delivered, state } = harness(false);
		expect(inbox.submit("a", "stop, wrong page", "immediate")).toBe("steered");
		expect(inbox.submit("a", "and again", "immediate")).toBe("queued");
		state.now += CANVAS_STEER_INTERVAL_MS;
		expect(inbox.submit("a", "later", "immediate")).toBe("steered");
		expect(delivered.map((entry) => entry.deliverAs)).toEqual(["steer", "steer"]);
	});

	it("treats immediate as enqueue when the agent is idle", () => {
		const { inbox, delivered } = harness();
		expect(inbox.submit("a", "now", "immediate")).toBe("started");
		expect(delivered[0]?.deliverAs).toBe("followUp");
	});

	it("stops a canvas looping on its own until the person speaks", () => {
		const { inbox, delivered, notices } = harness();
		for (let i = 0; i < CANVAS_MAX_UNATTENDED_TURNS; i += 1) expect(inbox.submit("a", `turn ${i}`)).toBe("started");
		expect(inbox.submit("a", "one more")).toBe("held");
		expect(inbox.submit("a", "and more")).toBe("held");
		expect(notices).toHaveLength(1);
		expect(delivered).toHaveLength(CANVAS_MAX_UNATTENDED_TURNS);
		inbox.personSpoke();
		inbox.agentEnded();
		expect(delivered.at(-1)?.text).toBe("[canvas a] and more");
	});
});

describe("canvas event source", () => {
	function capture() {
		const events: CanvasSessionEvent[] = [];
		const source = new CanvasEventSource(
			(event) => events.push(event),
			() => new Date(0),
		);
		return { events, source };
	}

	it("emits upstream-shaped envelopes for a turn", () => {
		const { events, source } = capture();
		source.turnStart();
		source.toolStart("c1", "read", { path: "src/a.ts" });
		source.toolEnd("c1", false);
		source.idle();
		expect(events.map((event) => event.type)).toEqual([
			"assistant.turn_start",
			"tool.execution_start",
			"assistant.intent",
			"tool.execution_complete",
			"session.idle",
		]);
		expect(events[1]).toMatchObject({
			timestamp: "1970-01-01T00:00:00.000Z",
			parentId: null,
			data: { toolCallId: "c1", toolName: "read", toolTitle: "read src/a.ts" },
		});
		expect(events[3]?.data).toEqual({ toolCallId: "c1", success: true, toolTitle: "read src/a.ts" });
	});

	it("never forwards tool arguments, only a one-line title", () => {
		const { events, source } = capture();
		source.toolStart("c1", "write", { path: "a.txt", content: "secret file body" });
		expect(JSON.stringify(events)).not.toContain("secret file body");
	});

	it("sends an intent only when it changes", () => {
		const { events, source } = capture();
		source.toolStart("c1", "invoke_canvas_action", { action: "edit_diagram" });
		source.toolStart("c2", "invoke_canvas_action", { action: "edit_diagram" });
		expect(events.filter((event) => event.type === "assistant.intent")).toHaveLength(1);
	});

	it("sends todos only when the list changes", () => {
		const { events, source } = capture();
		source.todos([{ id: 1, title: "Draw the VPC", status: "in_progress" }]);
		source.todos([{ id: 1, title: "Draw the VPC", status: "in_progress" }]);
		source.todos([{ id: 1, title: "Draw the VPC", status: "done" }]);
		expect(events.map((event) => event.data.todos)).toEqual([
			[{ id: 1, title: "Draw the VPC", status: "in_progress" }],
			[{ id: 1, title: "Draw the VPC", status: "done" }],
		]);
	});

	it("titles tools a person would recognise, and clips long ones", () => {
		expect(toolTitle("bash", { command: "npm test" })).toBe("$ npm test");
		expect(toolTitle("SearchCodebase", { query: "canvas inbox" })).toBe("SearchCodebase canvas inbox");
		expect(toolTitle("mystery", {})).toBe("mystery");
		expect(toolTitle("bash", { command: "x".repeat(200) }).length).toBeLessThanOrEqual(80);
	});
});
