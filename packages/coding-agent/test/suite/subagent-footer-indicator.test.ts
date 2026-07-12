import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Type } from "typebox";
import { afterEach, describe, expect, it } from "vitest";
import { FooterDataProvider } from "../../src/core/footer-data-provider.js";
import { createTaskToolDefinition } from "../../src/core/tools/subagent.js";
import { createHarness, type Harness } from "./harness.js";

/**
 * Regression: enabling the subagent (`--enable-subagents` / `enableSubagent`) registers
 * the model-facing tool as `Task`, but the interactive footer/resources wiring
 * checked `getActiveToolNames().includes("subagent")` and so never lit up.
 *
 * This test locks both ends together:
 *  - the Task tool definition is named `Task`,
 *  - `getActiveToolNames()` surfaces that exact name,
 *  - the footer indicator wiring derives the right state from it.
 */
describe("subagent footer indicator", () => {
	const harnesses: Harness[] = [];

	afterEach(() => {
		while (harnesses.length > 0) {
			harnesses.pop()?.cleanup();
		}
	});

	const taskTool: AgentTool = {
		name: "Task",
		label: "Task",
		description: "Delegate to a subagent",
		parameters: Type.Object({ prompt: Type.String() }),
		execute: async () => ({ content: [{ type: "text", text: "ok" }], details: undefined }),
	};

	it("Task tool definition is named 'Task'", () => {
		expect(createTaskToolDefinition().name).toBe("Task");
	});

	it("lights up the footer and resources when the Task tool is active", async () => {
		const harness = await createHarness({ tools: [taskTool] });
		harnesses.push(harness);

		expect(harness.session.getActiveToolNames()).toContain("Task");

		const provider = new FooterDataProvider(harness.tempDir);
		// Mirror interactive-mode wiring.
		provider.setSubagentEnabled(harness.session.getActiveToolNames().includes("Task"));

		expect(provider.getSubagentEnabled()).toBe(true);
		expect(provider.getActiveMode()).toBe("build");
		provider.dispose();
	});

	it("leaves the footer untouched when the Task tool is absent", async () => {
		const harness = await createHarness();
		harnesses.push(harness);

		expect(harness.session.getActiveToolNames()).not.toContain("Task");

		const provider = new FooterDataProvider(harness.tempDir);
		provider.setSubagentEnabled(harness.session.getActiveToolNames().includes("Task"));

		expect(provider.getSubagentEnabled()).toBe(false);
		expect(provider.getActiveMode()).toBe("build");
		provider.dispose();
	});
});
