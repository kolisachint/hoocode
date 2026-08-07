import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createEventBus } from "../src/core/event-bus.js";
import { LOOP_AUTO_CHANGED, LOOP_AUTO_START, type LoopAutoStartPayload } from "../src/extensions/core/loop.js";
import { setupMode } from "../src/extensions/core/modes.js";

/**
 * Handler-level coverage for the /grill and /goal commands: plan-file lookup,
 * the notify paths when there is nothing to act on, and what each one actually
 * puts on the wire. The message-construction helpers they call are unit-tested
 * separately in grill-command.test.ts and goal-command.test.ts.
 */

const SESSION_ID = "test-session";

/** Minimal fake ExtensionAPI backed by a real event bus. */
function makeHarness() {
	const events = createEventBus();
	const commands = new Map<string, (args: string, ctx: any) => Promise<void>>();
	const sentMessages: string[] = [];

	const pi: any = {
		events,
		on: () => {},
		registerTool: () => {},
		registerCommand: (name: string, opts: { handler: (args: string, ctx: any) => Promise<void> }) => {
			commands.set(name, opts.handler);
		},
		sendUserMessage: (content: string) => {
			sentMessages.push(content);
		},
		getModeSearchPaths: () => [],
		setActiveTools: () => {},
	};

	const autoStarts: Partial<LoopAutoStartPayload>[] = [];
	events.on(LOOP_AUTO_START, (d) => autoStarts.push((d ?? {}) as Partial<LoopAutoStartPayload>));

	setupMode(pi);

	return {
		pi,
		events,
		sentMessages,
		autoStarts,
		run: (name: string, args: string, ctx: unknown) => commands.get(name)!(args, ctx as any),
	};
}

function makeCtx(cwd: string) {
	const notifications: Array<{ message: string; type?: string }> = [];
	const ctx: any = {
		hasUI: true,
		cwd,
		sessionManager: { getSessionId: () => SESSION_ID },
		ui: { notify: (message: string, type?: string) => notifications.push({ message, type }) },
	};
	return { ctx, notifications };
}

const PLAN = `## Goal
Ship the /goal command.

## Files to modify
- packages/coding-agent/src/extensions/core/modes.ts

## Verification
npm run check && npm test
`;

describe("/grill and /goal handlers", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "mode-commands-"));
	});
	afterEach(() => fs.rmSync(tempDir, { recursive: true, force: true }));

	/** Writes the per-session plan file the commands look for. */
	function writePlan(contents = PLAN) {
		const planPath = path.join(tempDir, ".hoocode", "plans", `${SESSION_ID}.md`);
		fs.mkdirSync(path.dirname(planPath), { recursive: true });
		fs.writeFileSync(planPath, contents);
		return planPath;
	}

	describe("/grill", () => {
		it("runs both phases against the plan by default", async () => {
			const h = makeHarness();
			const { ctx } = makeCtx(tempDir);
			writePlan();

			await h.run("grill", "", ctx);

			expect(h.sentMessages).toHaveLength(1);
			expect(h.sentMessages[0]).toContain("ask_options");
			expect(h.sentMessages[0]).toContain("attacking the plan");
			expect(h.sentMessages[0]).toContain("Ship the /goal command.");
		});

		it("restricts itself to the requested phase", async () => {
			const h = makeHarness();
			const { ctx } = makeCtx(tempDir);
			writePlan();

			await h.run("grill", "plan", ctx);

			expect(h.sentMessages[0]).not.toContain("ask_options");
		});

		it("declines with a pointer to /plan when there is no plan", async () => {
			const h = makeHarness();
			const { ctx, notifications } = makeCtx(tempDir);

			await h.run("grill", "", ctx);

			expect(h.sentMessages).toHaveLength(0);
			expect(notifications[0].message).toContain("No plan to grill");
			expect(notifications[0].message).toContain("Run /plan first");
		});

		it("shows usage for an unknown subcommand instead of guessing", async () => {
			const h = makeHarness();
			const { ctx, notifications } = makeCtx(tempDir);
			writePlan();

			await h.run("grill", "everything", ctx);

			expect(h.sentMessages).toHaveLength(0);
			expect(notifications[0].message).toContain("Usage: /grill");
		});

		it("drops the interrogation phase while an autonomous loop is running", async () => {
			const h = makeHarness();
			const { ctx, notifications } = makeCtx(tempDir);
			writePlan();

			h.events.emit(LOOP_AUTO_CHANGED, { active: true });
			await h.run("grill", "me", ctx);

			// Nobody is present to answer, so the critique runs alone.
			expect(h.sentMessages[0]).not.toContain("ask_options");
			expect(h.sentMessages[0]).toContain("attacking the plan");
			expect(notifications.some((n) => n.message.includes("skipping questions"))).toBe(true);
		});

		it("asks again once the loop stops", async () => {
			const h = makeHarness();
			const { ctx } = makeCtx(tempDir);
			writePlan();

			h.events.emit(LOOP_AUTO_CHANGED, { active: true });
			h.events.emit(LOOP_AUTO_CHANGED, { active: false });
			await h.run("grill", "me", ctx);

			expect(h.sentMessages[0]).toContain("ask_options");
		});
	});

	describe("/goal", () => {
		it("takes the objective and completion condition from the plan", async () => {
			const h = makeHarness();
			const { ctx } = makeCtx(tempDir);
			writePlan();

			await h.run("goal", "", ctx);

			expect(h.autoStarts).toHaveLength(1);
			expect(h.autoStarts[0].task).toContain("Ship the /goal command.");
			expect(h.autoStarts[0].task).toContain("npm run check && npm test");
			expect(h.autoStarts[0].continuePrompt).toContain("Ship the /goal command.");
		});

		it("prefers an explicit objective but keeps the plan's verification", async () => {
			const h = makeHarness();
			const { ctx } = makeCtx(tempDir);
			writePlan();

			await h.run("goal", "make the parser tests pass", ctx);

			expect(h.autoStarts[0].task).toContain("make the parser tests pass");
			expect(h.autoStarts[0].task).not.toContain("Ship the /goal command.");
			expect(h.autoStarts[0].task).toContain("npm run check && npm test");
		});

		it("runs free-standing with no plan at all", async () => {
			const h = makeHarness();
			const { ctx } = makeCtx(tempDir);

			await h.run("goal", "make the parser tests pass", ctx);

			expect(h.autoStarts).toHaveLength(1);
			expect(h.autoStarts[0].task).toContain("make the parser tests pass");
		});

		it("passes the turn budget through to the loop", async () => {
			const h = makeHarness();
			const { ctx } = makeCtx(tempDir);

			await h.run("goal", "--max-turns 25 ship it", ctx);

			expect(h.autoStarts[0].maxTurns).toBe(25);
		});

		it("declines when there is neither an objective nor a plan goal", async () => {
			const h = makeHarness();
			const { ctx, notifications } = makeCtx(tempDir);

			await h.run("goal", "", ctx);

			expect(h.autoStarts).toHaveLength(0);
			expect(notifications[0].message).toContain("No objective");
		});

		it("shows usage for a malformed budget rather than silently defaulting it", async () => {
			const h = makeHarness();
			const { ctx, notifications } = makeCtx(tempDir);
			writePlan();

			await h.run("goal", "--max-turns lots ship it", ctx);

			expect(h.autoStarts).toHaveLength(0);
			expect(notifications[0].message).toContain("Usage: /goal");
		});
	});
});
