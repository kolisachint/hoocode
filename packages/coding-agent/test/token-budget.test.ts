import assert from "node:assert";
import { readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it } from "node:test";
import { CONFIG_DIR_NAME } from "../src/config.js";
import { getDefaultBudget, TokenBudget } from "../src/core/token-budget.js";

describe("TokenBudget", () => {
	describe("getDefaultBudget", () => {
		it("returns correct defaults per agent type", () => {
			assert.strictEqual(getDefaultBudget("explore"), 8000);
			assert.strictEqual(getDefaultBudget("edit"), 16000);
			assert.strictEqual(getDefaultBudget("test"), 16000);
			assert.strictEqual(getDefaultBudget("review"), 12000);
			assert.strictEqual(getDefaultBudget("doc"), 10000);
			assert.strictEqual(getDefaultBudget("unknown"), 16000);
		});
	});

	describe("processStdout", () => {
		it("accumulates usage from message_end events", () => {
			const budget = new TokenBudget("t1", "explore");
			assert.strictEqual(budget.getUsed(), 0);

			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 100 } } }) + "\n",
			);
			assert.strictEqual(budget.getUsed(), 100);

			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 200 } } }) + "\n",
			);
			assert.strictEqual(budget.getUsed(), 300);
		});

		it("ignores non-assistant message_end events", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "user", usage: { totalTokens: 500 } } }) + "\n",
			);
			assert.strictEqual(budget.getUsed(), 0);
		});

		it("ignores non-message_end events", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout(
				JSON.stringify({ type: "message_update", message: { role: "assistant", usage: { totalTokens: 500 } } }) +
					"\n",
			);
			assert.strictEqual(budget.getUsed(), 0);
		});

		it("handles events split across chunks", () => {
			const budget = new TokenBudget("t1", "explore");
			const event = JSON.stringify({
				type: "message_end",
				message: { role: "assistant", usage: { totalTokens: 150 } },
			});
			budget.processStdout(event.slice(0, 20));
			assert.strictEqual(budget.getUsed(), 0);
			budget.processStdout(event.slice(20) + "\n");
			assert.strictEqual(budget.getUsed(), 150);
		});

		it("handles multiple events in one chunk", () => {
			const budget = new TokenBudget("t1", "explore");
			const line1 = JSON.stringify({
				type: "message_end",
				message: { role: "assistant", usage: { totalTokens: 50 } },
			});
			const line2 = JSON.stringify({
				type: "message_end",
				message: { role: "assistant", usage: { totalTokens: 75 } },
			});
			budget.processStdout(`${line1}\n${line2}\n`);
			assert.strictEqual(budget.getUsed(), 125);
		});

		it("ignores invalid JSON", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout("not json\n");
			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 42 } } }) + "\n",
			);
			assert.strictEqual(budget.getUsed(), 42);
		});

		it("handles empty lines", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout("\n\n");
			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 10 } } }) +
					"\n\n",
			);
			assert.strictEqual(budget.getUsed(), 10);
		});
	});

	describe("flush", () => {
		it("processes remaining buffered line without newline", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 99 } } }),
			);
			assert.strictEqual(budget.getUsed(), 0);
			budget.flush();
			assert.strictEqual(budget.getUsed(), 99);
		});
	});

	describe("thresholds", () => {
		it("emits budget_warning at 80%", () => {
			const budget = new TokenBudget("t1", "explore", { limit: 1000 });
			let warned = false;
			let warningData: unknown;

			budget.on("budget_warning", (data) => {
				warned = true;
				warningData = data;
			});

			// 80% = 800 tokens
			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 800 } } }) + "\n",
			);

			assert.strictEqual(warned, true);
			assert.deepStrictEqual(warningData, {
				task_id: "t1",
				message: "You are near token limit. Summarize and write result.json now.",
				used: 800,
				limit: 1000,
			});
			assert.strictEqual(budget.isWarned(), true);
		});

		it("emits budget_exceeded at 100%", () => {
			const budget = new TokenBudget("t1", "explore", { limit: 500 });
			let exceeded = false;
			let exceededData: unknown;

			budget.on("budget_exceeded", (data) => {
				exceeded = true;
				exceededData = data;
			});

			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 500 } } }) + "\n",
			);

			assert.strictEqual(exceeded, true);
			assert.deepStrictEqual(exceededData, {
				task_id: "t1",
				used: 500,
				limit: 500,
			});
			assert.strictEqual(budget.isExceeded(), true);
		});

		it("warns once even if threshold is crossed multiple times", () => {
			const budget = new TokenBudget("t1", "explore", { limit: 100 });
			let warningCount = 0;
			budget.on("budget_warning", () => {
				warningCount++;
			});

			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 80 } } }) + "\n",
			);
			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 10 } } }) + "\n",
			);

			assert.strictEqual(warningCount, 1);
		});

		it("exceeds once even if threshold is crossed multiple times", () => {
			const budget = new TokenBudget("t1", "explore", { limit: 100 });
			let exceededCount = 0;
			budget.on("budget_exceeded", () => {
				exceededCount++;
			});

			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 100 } } }) + "\n",
			);
			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 10 } } }) + "\n",
			);

			assert.strictEqual(exceededCount, 1);
		});
	});

	describe("persist", () => {
		it("saves budget state to disk", () => {
			const testCwd = join(tmpdir(), `hoocode-test-${Date.now()}`);
			const budget = new TokenBudget("persist-task", "edit", { cwd: testCwd });
			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 2500 } } }) +
					"\n",
			);

			const path = join(testCwd, CONFIG_DIR_NAME, "agents", "persist-task", "budget.json");
			const raw = readFileSync(path, "utf-8");
			const state = JSON.parse(raw) as {
				task_id: string;
				agent_type: string;
				budget: number;
				used: number;
				warned: boolean;
				exceeded: boolean;
				last_updated: number;
			};

			assert.strictEqual(state.task_id, "persist-task");
			assert.strictEqual(state.agent_type, "edit");
			assert.strictEqual(state.budget, 16000);
			assert.strictEqual(state.used, 2500);
			assert.strictEqual(state.warned, false);
			assert.strictEqual(state.exceeded, false);
			assert.ok(typeof state.last_updated === "number");
			assert.ok(state.last_updated > 0);

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("updates persisted file on each usage event", () => {
			const testCwd = join(tmpdir(), `hoocode-test-${Date.now()}`);
			const budget = new TokenBudget("persist-task2", "explore", { cwd: testCwd, limit: 500 });

			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 200 } } }) + "\n",
			);

			const path = join(testCwd, CONFIG_DIR_NAME, "agents", "persist-task2", "budget.json");
			const state1 = JSON.parse(readFileSync(path, "utf-8"));
			assert.strictEqual(state1.used, 200);
			assert.strictEqual(state1.warned, false);
			assert.strictEqual(state1.exceeded, false);

			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 250 } } }) + "\n",
			);

			const state2 = JSON.parse(readFileSync(path, "utf-8"));
			assert.strictEqual(state2.used, 450);
			assert.strictEqual(state2.warned, true);
			assert.strictEqual(state2.exceeded, false);

			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 100 } } }) + "\n",
			);

			const state3 = JSON.parse(readFileSync(path, "utf-8"));
			assert.strictEqual(state3.used, 550);
			assert.strictEqual(state3.warned, true);
			assert.strictEqual(state3.exceeded, true);

			rmSync(testCwd, { recursive: true, force: true });
		});
	});
});
