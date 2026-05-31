import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { CONFIG_DIR_NAME } from "../src/config.js";
import { getDefaultBudget, TokenBudget } from "../src/core/token-budget.js";

describe("TokenBudget", () => {
	describe("getDefaultBudget", () => {
		it("returns correct defaults per agent type", () => {
			expect(getDefaultBudget("explore")).toBe(35000);
			expect(getDefaultBudget("edit")).toBe(60000);
			expect(getDefaultBudget("test")).toBe(45000);
			expect(getDefaultBudget("fix")).toBe(45000);
			expect(getDefaultBudget("review")).toBe(35000);
			expect(getDefaultBudget("doc")).toBe(30000);
			expect(getDefaultBudget("unknown")).toBe(35000);
		});
	});

	describe("processStdout", () => {
		it("accumulates usage from message_end events", () => {
			const budget = new TokenBudget("t1", "explore");
			expect(budget.getUsed()).toBe(0);

			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 100 } } })}
`,
			);
			expect(budget.getUsed()).toBe(100);

			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 200 } } })}
`,
			);
			expect(budget.getUsed()).toBe(300);
		});

		it("ignores non-assistant message_end events", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "user", usage: { totalTokens: 500 } } })}
`,
			);
			expect(budget.getUsed()).toBe(0);
		});

		it("ignores non-message_end events", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout(
				`${JSON.stringify({ type: "message_update", message: { role: "assistant", usage: { totalTokens: 500 } } })}
`,
			);
			expect(budget.getUsed()).toBe(0);
		});

		it("handles events split across chunks", () => {
			const budget = new TokenBudget("t1", "explore");
			const event = JSON.stringify({
				type: "message_end",
				message: { role: "assistant", usage: { totalTokens: 150 } },
			});
			budget.processStdout(event.slice(0, 20));
			expect(budget.getUsed()).toBe(0);
			budget.processStdout(`${event.slice(20)}
`);
			expect(budget.getUsed()).toBe(150);
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
			expect(budget.getUsed()).toBe(125);
		});

		it("ignores invalid JSON", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout("not json\n");
			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 42 } } })}
`,
			);
			expect(budget.getUsed()).toBe(42);
		});

		it("handles empty lines", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout("\n\n");
			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 10 } } })}

`,
			);
			expect(budget.getUsed()).toBe(10);
		});
	});

	describe("flush", () => {
		it("processes remaining buffered line without newline", () => {
			const budget = new TokenBudget("t1", "explore");
			budget.processStdout(
				JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 99 } } }),
			);
			expect(budget.getUsed()).toBe(0);
			budget.flush();
			expect(budget.getUsed()).toBe(99);
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
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 800 } } })}
`,
			);

			expect(warned).toBe(true);
			expect(warningData).toEqual({
				task_id: "t1",
				message: "You are near token limit. Summarize and write result.json now.",
				used: 800,
				limit: 1000,
			});
			expect(budget.isWarned()).toBe(true);
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
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 500 } } })}
`,
			);

			expect(exceeded).toBe(true);
			expect(exceededData).toEqual({
				task_id: "t1",
				used: 500,
				limit: 500,
			});
			expect(budget.isExceeded()).toBe(true);
		});

		it("warns once even if threshold is crossed multiple times", () => {
			const budget = new TokenBudget("t1", "explore", { limit: 100 });
			let warningCount = 0;
			budget.on("budget_warning", () => {
				warningCount++;
			});

			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 80 } } })}
`,
			);
			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 10 } } })}
`,
			);

			expect(warningCount).toBe(1);
		});

		it("exceeds once even if threshold is crossed multiple times", () => {
			const budget = new TokenBudget("t1", "explore", { limit: 100 });
			let exceededCount = 0;
			budget.on("budget_exceeded", () => {
				exceededCount++;
			});

			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 100 } } })}
`,
			);
			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 10 } } })}
`,
			);

			expect(exceededCount).toBe(1);
		});
	});

	describe("persist", () => {
		it("saves budget state to disk", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const budget = new TokenBudget("persist-task", "edit", { cwd: testCwd });
			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 2500 } } })}
`,
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

			expect(state.task_id).toBe("persist-task");
			expect(state.agent_type).toBe("edit");
			expect(state.budget).toBe(60000);
			expect(state.used).toBe(2500);
			expect(state.warned).toBe(false);
			expect(state.exceeded).toBe(false);
			expect(typeof state.last_updated).toBe("number");
			expect(state.last_updated).toBeGreaterThan(0);

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("updates persisted file on each usage event", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const budget = new TokenBudget("persist-task2", "explore", { cwd: testCwd, limit: 500 });

			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 200 } } })}
`,
			);

			const path = join(testCwd, CONFIG_DIR_NAME, "agents", "persist-task2", "budget.json");
			const state1 = JSON.parse(readFileSync(path, "utf-8"));
			expect(state1.used).toBe(200);
			expect(state1.warned).toBe(false);
			expect(state1.exceeded).toBe(false);

			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 250 } } })}
`,
			);

			const state2 = JSON.parse(readFileSync(path, "utf-8"));
			expect(state2.used).toBe(450);
			expect(state2.warned).toBe(true);
			expect(state2.exceeded).toBe(false);

			budget.processStdout(
				`${JSON.stringify({ type: "message_end", message: { role: "assistant", usage: { totalTokens: 100 } } })}
`,
			);

			const state3 = JSON.parse(readFileSync(path, "utf-8"));
			expect(state3.used).toBe(550);
			expect(state3.warned).toBe(true);
			expect(state3.exceeded).toBe(true);

			rmSync(testCwd, { recursive: true, force: true });
		});
	});
});
