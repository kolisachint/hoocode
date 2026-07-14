import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createEventBus } from "../src/core/event-bus.js";
import { setupAskOptions } from "../src/extensions/core/ask-options.js";
import { LOOP_AUTO_CHANGED, LOOP_HALT, setupLoop } from "../src/extensions/core/loop.js";

/**
 * These tests cover the autonomous-loop behavior of the ask_options tool:
 * while a `/loop auto` run is active it must never block on the interactive
 * pane — it decides questions that carry a recommended default and halts the
 * loop when one does not.
 */

/** Minimal fake ExtensionAPI backed by a real event bus. */
function makeHarness() {
	const events = createEventBus();
	const handlers = new Map<string, Array<(e: unknown, ctx: unknown) => unknown>>();
	let askTool: any;
	const commands = new Map<string, (args: string, ctx: any) => Promise<void>>();
	const sentMessages: string[] = [];

	const pi: any = {
		events,
		on: (event: string, handler: (e: unknown, ctx: unknown) => unknown) => {
			const arr = handlers.get(event) ?? [];
			arr.push(handler);
			handlers.set(event, arr);
		},
		registerTool: (tool: any) => {
			if (tool.name === "ask_options") askTool = tool;
		},
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

	return { pi, events, fire, sentMessages, getAskTool: () => askTool, getCommand: (n: string) => commands.get(n) };
}

function makeCtx(overrides: Partial<any> = {}) {
	const notifications: Array<{ message: string; type?: string }> = [];
	const askCalls: unknown[][] = [];
	const ctx: any = {
		hasUI: true,
		cwd: overrides.cwd,
		isIdle: () => true,
		ui: {
			notify: (message: string, type?: string) => notifications.push({ message, type }),
			askOptions: async (questions: unknown[]) => {
				askCalls.push(questions);
				return ["USER_PICKED"];
			},
			...(overrides.ui ?? {}),
		},
	};
	return { ctx, notifications, askCalls };
}

describe("ask_options under autonomous loop", () => {
	it("blocks on the pane normally when no loop is active", async () => {
		const h = makeHarness();
		setupAskOptions(h.pi);
		const { ctx, askCalls } = makeCtx();
		h.fire("session_start", { type: "session_start" }, ctx);

		const res = await h
			.getAskTool()
			.execute(
				"id",
				{ questions: [{ question: "which?", options: [{ label: "a" }, { label: "b" }] }] },
				undefined,
				undefined,
			);

		expect(askCalls).toHaveLength(1); // pane was shown
		expect(res.content[0].text).toContain("USER_PICKED");
	});

	it("auto-selects the recommended default without showing the pane", async () => {
		const h = makeHarness();
		setupAskOptions(h.pi);
		const { ctx, askCalls } = makeCtx();
		h.fire("session_start", { type: "session_start" }, ctx);
		h.events.emit(LOOP_AUTO_CHANGED, { active: true });

		const res = await h.getAskTool().execute(
			"id",
			{
				questions: [{ question: "which store?", options: [{ label: "a" }, { label: "b", recommended: true }] }],
			},
			undefined,
			undefined,
		);

		expect(askCalls).toHaveLength(0); // never blocked
		expect(res.content[0].text).toContain("→ b");
		expect(res.content[0].text).toContain("recommended default");
	});

	it("halts the loop and reports when a question has no recommended default", async () => {
		const h = makeHarness();
		setupAskOptions(h.pi);
		const { ctx, askCalls } = makeCtx();
		h.fire("session_start", { type: "session_start" }, ctx);
		h.events.emit(LOOP_AUTO_CHANGED, { active: true });

		let halt: { reason?: string } | undefined;
		h.events.on(LOOP_HALT, (d) => {
			halt = d as { reason?: string };
		});

		const res = await h.getAskTool().execute(
			"id",
			{
				questions: [
					{ question: "safe one?", options: [{ label: "a", recommended: true }] },
					{ question: "risky one?", options: [{ label: "x" }, { label: "y" }] },
				],
			},
			undefined,
			undefined,
		);

		expect(askCalls).toHaveLength(0); // never blocked
		expect(halt).toBeDefined();
		expect(res.content[0].text).toContain("risky one?");
		expect(res.content[0].text).toContain("stopped");
	});

	it("resumes blocking once the loop ends", async () => {
		const h = makeHarness();
		setupAskOptions(h.pi);
		const { ctx, askCalls } = makeCtx();
		h.fire("session_start", { type: "session_start" }, ctx);

		h.events.emit(LOOP_AUTO_CHANGED, { active: true });
		h.events.emit(LOOP_AUTO_CHANGED, { active: false });

		await h
			.getAskTool()
			.execute(
				"id",
				{ questions: [{ question: "which?", options: [{ label: "a" }, { label: "b" }] }] },
				undefined,
				undefined,
			);

		expect(askCalls).toHaveLength(1); // blocking restored
	});
});

describe("loop ↔ ask_options integration", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "loop-ask-"));
	});
	afterEach(() => fs.rmSync(tempDir, { recursive: true, force: true }));

	it("/loop auto flips ask_options into non-blocking mode, and a blocker halts it", async () => {
		const h = makeHarness();
		setupAskOptions(h.pi);
		setupLoop(h.pi);
		const { ctx, notifications, askCalls } = makeCtx({ cwd: tempDir });
		h.fire("session_start", { type: "session_start" }, ctx);

		// Start the autonomous loop.
		await h.getCommand("loop")!("auto do the thing", ctx);

		// A blocker (no recommended default) should halt the loop instead of blocking.
		const res = await h
			.getAskTool()
			.execute(
				"id",
				{ questions: [{ question: "no default here?", options: [{ label: "x" }, { label: "y" }] }] },
				undefined,
				undefined,
			);

		expect(askCalls).toHaveLength(0);
		expect(res.content[0].text).toContain("stopped");
		expect(notifications.some((n) => n.message.includes("halted"))).toBe(true);

		// After halt, blocking behavior is restored.
		await h
			.getAskTool()
			.execute(
				"id",
				{ questions: [{ question: "again?", options: [{ label: "a" }, { label: "b" }] }] },
				undefined,
				undefined,
			);
		expect(askCalls).toHaveLength(1);

		h.fire("session_shutdown", { type: "session_shutdown", reason: "quit" }, ctx);
	});
});
