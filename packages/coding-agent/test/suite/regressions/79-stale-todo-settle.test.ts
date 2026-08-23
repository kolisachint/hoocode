/**
 * Regression #79: main-plan rows left pinned at in_progress after a request ends.
 *
 * TodoWrite items are bookkeeping the model performs by hand, and even strong
 * models drop the final call that marks the plan complete. Nothing else writes
 * main-plan rows, so the task panel kept claiming live work until the next user
 * message triggered taskStore.reset(). The request is over by then, so the row
 * has to settle at agent_end instead.
 */

import type { AssistantMessage } from "@kolisachint/hoocode-ai";
import { Container } from "@kolisachint/hoocode-tui";
import { beforeEach, describe, expect, test, vi } from "vitest";
import type { ExtensionContext } from "../../../src/core/extensions/types.js";
import { type Task, taskStore } from "../../../src/core/task-store.js";
import { createTodoWriteToolDefinition, settleDanglingMainTasks } from "../../../src/core/tools/todo.js";
import { InteractiveMode } from "../../../src/modes/interactive/interactive-mode.js";

const todoWrite = createTodoWriteToolDefinition();

type TodoItem = { content: string; status: "pending" | "in_progress" | "completed"; activeForm?: string };

/** Drive the real TodoWrite tool so the store is populated the way the model populates it. */
async function writeTodos(todos: TodoItem[]): Promise<void> {
	await todoWrite.execute("call-79", { todos }, undefined, undefined, {} as ExtensionContext);
}

function statusOf(title: string): Task["status"] | undefined {
	return taskStore.list().find((t) => (t.todoContent ?? t.title) === title)?.status;
}

/**
 * Minimal stand-in for the InteractiveMode fields `settleDanglingPlanItems` and
 * `settleRequestOnIdle` touch, so the real prototype methods can be exercised
 * without booting a TUI.
 */
type SettleThis = {
	turnStopReason: AssistantMessage["stopReason"] | undefined;
	chimePendingRetry: boolean;
	session: { pendingMessageCount: number; isStreaming: boolean; isCompacting: boolean };
	ui: { requestRender: () => void };
	chime?: { onTurnComplete: (options: { aborted: boolean }) => void };
	showTurnCost: () => void;
	settleDanglingPlanItems: () => void;
	closeOpenChain: (outcome: "done" | "interrupted") => void;
	chatContainer: Container;
	openChain: undefined;
};

function createSettleThis(overrides: Partial<SettleThis> = {}): SettleThis {
	const proto = InteractiveMode.prototype as unknown as Pick<SettleThis, "settleDanglingPlanItems" | "closeOpenChain">;
	return {
		turnStopReason: "stop",
		chimePendingRetry: false,
		session: { pendingMessageCount: 0, isStreaming: false, isCompacting: false },
		ui: { requestRender: vi.fn() },
		chime: { onTurnComplete: vi.fn() },
		showTurnCost: vi.fn(),
		settleDanglingPlanItems: proto.settleDanglingPlanItems,
		// Settling also closes the turn's last radar chain. Borrowed rather than
		// stubbed so this test keeps exercising the real settle path.
		closeOpenChain: proto.closeOpenChain,
		chatContainer: new Container(),
		openChain: undefined,
		...overrides,
	};
}

function settleDanglingPlanItems(context: SettleThis): void {
	(
		InteractiveMode.prototype as unknown as { settleDanglingPlanItems: (this: SettleThis) => void }
	).settleDanglingPlanItems.call(context);
}

async function settleRequestOnIdle(context: SettleThis): Promise<void> {
	(
		InteractiveMode.prototype as unknown as { settleRequestOnIdle: (this: SettleThis) => void }
	).settleRequestOnIdle.call(context);
	// settleRequestOnIdle defers a tick so the streaming flag can settle.
	await new Promise((resolve) => setTimeout(resolve, 1));
}

describe("regression #79: dangling main-plan tasks settle when the request ends", () => {
	beforeEach(() => {
		taskStore.clear();
	});

	test("settles the in_progress item the model never marked completed", async () => {
		await writeTodos([
			{ content: "Read the config", status: "completed" },
			{ content: "Patch the parser", status: "in_progress", activeForm: "Patching the parser" },
		]);
		expect(statusOf("Patch the parser")).toBe("in_progress");

		expect(settleDanglingMainTasks("done")).toBe(1);

		expect(statusOf("Patch the parser")).toBe("done");
		expect(statusOf("Read the config")).toBe("done");
	});

	test("leaves pending items alone — never started is already the truth", async () => {
		await writeTodos([
			{ content: "Patch the parser", status: "in_progress" },
			{ content: "Update the docs", status: "pending" },
		]);

		expect(settleDanglingMainTasks("done")).toBe(1);

		expect(statusOf("Patch the parser")).toBe("done");
		expect(statusOf("Update the docs")).toBe("pending");
	});

	test("is a no-op when the model did send the final TodoWrite call", async () => {
		await writeTodos([{ content: "Patch the parser", status: "completed" }]);
		expect(settleDanglingMainTasks("done")).toBe(0);
		expect(statusOf("Patch the parser")).toBe("done");
	});

	test("never touches subagent- or MCP-sourced rows", async () => {
		await writeTodos([{ content: "Patch the parser", status: "in_progress" }]);
		const mcp = taskStore.create("github: list issues", { source: "mcp", subagentMode: "github" });
		taskStore.update(mcp.id, { status: "done" });
		const delegated = taskStore.create("explore the module", { source: "subagent", subagentMode: "explore" });
		taskStore.update(delegated.id, { status: "failed" });

		expect(settleDanglingMainTasks("cancelled")).toBe(1);

		expect(statusOf("Patch the parser")).toBe("cancelled");
		expect(taskStore.list().find((t) => t.id === mcp.id)?.status).toBe("done");
		expect(taskStore.list().find((t) => t.id === delegated.id)?.status).toBe("failed");
	});

	test("holds off while a delegated task is still running", async () => {
		await writeTodos([{ content: "Patch the parser", status: "in_progress" }]);
		const delegated = taskStore.create("explore the module", { source: "subagent", subagentMode: "explore" });
		taskStore.update(delegated.id, { status: "in_progress" });

		expect(settleDanglingMainTasks("done")).toBe(0);
		expect(statusOf("Patch the parser")).toBe("in_progress");

		// Once the subagent settles, the plan item is free to settle too.
		taskStore.update(delegated.id, { status: "done" });
		expect(settleDanglingMainTasks("done")).toBe(1);
		expect(statusOf("Patch the parser")).toBe("done");
	});

	test("a clean stop settles to done, an abort or error to cancelled", async () => {
		for (const [stopReason, expected] of [
			["stop", "done"],
			["aborted", "cancelled"],
			["error", "cancelled"],
			["length", "cancelled"],
		] as const) {
			taskStore.clear();
			await writeTodos([{ content: "Patch the parser", status: "in_progress" }]);

			settleDanglingPlanItems(createSettleThis({ turnStopReason: stopReason }));

			expect(statusOf("Patch the parser"), `stopReason ${stopReason}`).toBe(expected);
		}
	});

	test("does not settle while a follow-up or steer message is queued", async () => {
		await writeTodos([{ content: "Patch the parser", status: "in_progress" }]);

		settleDanglingPlanItems(
			createSettleThis({
				session: { pendingMessageCount: 1, isStreaming: false, isCompacting: false },
			}),
		);

		expect(statusOf("Patch the parser")).toBe("in_progress");
	});

	test("settleRequestOnIdle settles the plan once the request is genuinely idle", async () => {
		await writeTodos([{ content: "Patch the parser", status: "in_progress" }]);

		await settleRequestOnIdle(createSettleThis());

		expect(statusOf("Patch the parser")).toBe("done");
	});

	test("settleRequestOnIdle leaves the plan alone when a retry is pending or the session is busy", async () => {
		for (const busy of [
			{ chimePendingRetry: true },
			{ session: { pendingMessageCount: 0, isStreaming: true, isCompacting: false } },
			{ session: { pendingMessageCount: 0, isStreaming: false, isCompacting: true } },
		] as Partial<SettleThis>[]) {
			taskStore.clear();
			await writeTodos([{ content: "Patch the parser", status: "in_progress" }]);

			await settleRequestOnIdle(createSettleThis(busy));

			expect(statusOf("Patch the parser"), JSON.stringify(busy)).toBe("in_progress");
		}
	});
});
