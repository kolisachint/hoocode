import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createEventBus } from "../src/core/event-bus.js";
import { AUTO_LOOP_DONE_TOKEN, LOOP_AUTO_CHANGED, LOOP_AUTO_START, setupLoop } from "../src/extensions/core/loop.js";

/**
 * Covers the LOOP_AUTO_START channel, which lets another extension (`/goal`)
 * drive the autonomous loop without reaching into its closure state, plus the
 * `/loop auto` behaviour that now shares the same starter.
 */

/** Minimal fake ExtensionAPI backed by a real event bus. */
function makeHarness() {
	const events = createEventBus();
	const handlers = new Map<string, Array<(e: unknown, ctx: unknown) => unknown>>();
	const commands = new Map<string, (args: string, ctx: any) => Promise<void>>();
	const sentMessages: string[] = [];

	const pi: any = {
		events,
		on: (event: string, handler: (e: unknown, ctx: unknown) => unknown) => {
			const arr = handlers.get(event) ?? [];
			arr.push(handler);
			handlers.set(event, arr);
		},
		registerTool: () => {},
		registerCommand: (name: string, opts: { handler: (args: string, ctx: any) => Promise<void> }) => {
			commands.set(name, opts.handler);
		},
		sendUserMessage: (content: string) => {
			sentMessages.push(content);
		},
	};

	const fire = (event: string, e: unknown, ctx: unknown) => {
		for (const h of handlers.get(event) ?? []) h(e, ctx);
	};

	return { pi, events, fire, sentMessages, getCommand: (n: string) => commands.get(n) };
}

function makeCtx(cwd: string) {
	const notifications: Array<{ message: string; type?: string }> = [];
	const ctx: any = {
		hasUI: true,
		cwd,
		isIdle: () => true,
		hasPendingMessages: () => false,
		ui: { notify: (message: string, type?: string) => notifications.push({ message, type }) },
	};
	return { ctx, notifications };
}

/** Fires an agent_end carrying one assistant message with the given text. */
function endTurn(h: ReturnType<typeof makeHarness>, ctx: unknown, text: string) {
	h.fire("agent_end", { type: "agent_end", messages: [{ role: "assistant", content: text }] }, ctx);
}

describe("LOOP_AUTO_START", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "loop-auto-start-"));
	});
	afterEach(() => fs.rmSync(tempDir, { recursive: true, force: true }));

	it("starts a run and announces it as active on the bus", () => {
		const h = makeHarness();
		setupLoop(h.pi);
		const { ctx, notifications } = makeCtx(tempDir);
		h.fire("session_start", { type: "session_start" }, ctx);

		const active: boolean[] = [];
		h.events.on(LOOP_AUTO_CHANGED, (d) => active.push((d as { active: boolean }).active));

		h.events.emit(LOOP_AUTO_START, { task: "make the parser tests pass" });

		expect(h.sentMessages[0]).toContain("make the parser tests pass");
		expect(h.sentMessages[0]).toContain(AUTO_LOOP_DONE_TOKEN);
		expect(active).toEqual([true]);
		expect(notifications.some((n) => n.message.includes("Autonomous loop started"))).toBe(true);

		h.fire("session_shutdown", { type: "session_shutdown", reason: "quit" }, ctx);
	});

	it("ignores a payload with no usable task rather than arming an empty loop", () => {
		const h = makeHarness();
		setupLoop(h.pi);
		const { ctx } = makeCtx(tempDir);
		h.fire("session_start", { type: "session_start" }, ctx);

		h.events.emit(LOOP_AUTO_START, {});
		h.events.emit(LOOP_AUTO_START, { task: "   " });
		h.events.emit(LOOP_AUTO_START, undefined);

		expect(h.sentMessages).toHaveLength(0);

		h.fire("session_shutdown", { type: "session_shutdown", reason: "quit" }, ctx);
	});

	it("re-sends the caller's continuePrompt on each continuation", () => {
		const h = makeHarness();
		setupLoop(h.pi);
		const { ctx } = makeCtx(tempDir);
		h.fire("session_start", { type: "session_start" }, ctx);

		h.events.emit(LOOP_AUTO_START, {
			task: "ship it",
			continuePrompt: "Keep going until `npm test` passes.",
		});
		endTurn(h, ctx, "made some progress");

		expect(h.sentMessages[1]).toBe("Keep going until `npm test` passes.");

		h.fire("session_shutdown", { type: "session_shutdown", reason: "quit" }, ctx);
	});

	it("falls back to the generic nudge when no continuePrompt is given", () => {
		const h = makeHarness();
		setupLoop(h.pi);
		const { ctx } = makeCtx(tempDir);
		h.fire("session_start", { type: "session_start" }, ctx);

		h.events.emit(LOOP_AUTO_START, { task: "ship it" });
		endTurn(h, ctx, "made some progress");

		expect(h.sentMessages[1]).toContain("Continue working toward the goal");

		h.fire("session_shutdown", { type: "session_shutdown", reason: "quit" }, ctx);
	});

	it("stops when the agent reports the done token", () => {
		const h = makeHarness();
		setupLoop(h.pi);
		const { ctx, notifications } = makeCtx(tempDir);
		h.fire("session_start", { type: "session_start" }, ctx);

		const active: boolean[] = [];
		h.events.on(LOOP_AUTO_CHANGED, (d) => active.push((d as { active: boolean }).active));

		h.events.emit(LOOP_AUTO_START, { task: "ship it" });
		endTurn(h, ctx, `all done — ${AUTO_LOOP_DONE_TOKEN}`);

		expect(h.sentMessages).toHaveLength(1); // no continuation queued
		expect(active).toEqual([true, false]);
		expect(notifications.some((n) => n.message.includes("complete"))).toBe(true);

		h.fire("session_shutdown", { type: "session_shutdown", reason: "quit" }, ctx);
	});

	it("honours an explicit zero budget instead of substituting the default", () => {
		const h = makeHarness();
		setupLoop(h.pi);
		const { ctx, notifications } = makeCtx(tempDir);
		h.fire("session_start", { type: "session_start" }, ctx);

		h.events.emit(LOOP_AUTO_START, { task: "ship it", maxTurns: 0 });
		endTurn(h, ctx, "still working");

		expect(h.sentMessages).toHaveLength(1); // budget spent, nothing re-sent
		expect(notifications.some((n) => n.message.includes("max turns reached"))).toBe(true);

		h.fire("session_shutdown", { type: "session_shutdown", reason: "quit" }, ctx);
	});

	it("substitutes the default budget when the payload's is nonsensical", () => {
		const h = makeHarness();
		setupLoop(h.pi);
		const { ctx, notifications } = makeCtx(tempDir);
		h.fire("session_start", { type: "session_start" }, ctx);

		h.events.emit(LOOP_AUTO_START, { task: "ship it", maxTurns: Number.NaN });

		expect(notifications.some((n) => n.message.includes("max 10 turns"))).toBe(true);

		h.fire("session_shutdown", { type: "session_shutdown", reason: "quit" }, ctx);
	});
});

describe("/loop auto after sharing the starter", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "loop-auto-cmd-"));
	});
	afterEach(() => fs.rmSync(tempDir, { recursive: true, force: true }));

	it("still delivers the task, applies --max-turns, and uses the generic continuation", async () => {
		const h = makeHarness();
		setupLoop(h.pi);
		const { ctx, notifications } = makeCtx(tempDir);
		h.fire("session_start", { type: "session_start" }, ctx);

		await h.getCommand("loop")!("auto --max-turns 2 refactor the parser", ctx);

		expect(h.sentMessages[0]).toContain("refactor the parser");
		expect(h.sentMessages[0]).toContain(AUTO_LOOP_DONE_TOKEN);
		expect(notifications.some((n) => n.message.includes("max 2 turns"))).toBe(true);

		endTurn(h, ctx, "progress");
		expect(h.sentMessages[1]).toContain("Continue working toward the goal");

		h.fire("session_shutdown", { type: "session_shutdown", reason: "quit" }, ctx);
	});
});
