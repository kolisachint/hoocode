import type { BackgroundToolResult } from "@kolisachint/hoocode-agent-core";
import { describe, expect, it } from "vitest";
import {
	BACKGROUND_TASK_CUSTOM_TYPE,
	createBackgroundPlaceholderText,
	createBackgroundTaskMessage,
	describeBackgroundTool,
} from "../src/core/messages.js";

/** Build a minimal finished-tool result for createBackgroundTaskMessage. */
function bgResult(name: string, args: Record<string, unknown>, text: string, isError = false): BackgroundToolResult {
	return {
		toolCall: { type: "toolCall", id: "tc-1", name, arguments: args },
		result: { content: [{ type: "text", text }], details: undefined },
		isError,
	} as unknown as BackgroundToolResult;
}

describe("describeBackgroundTool", () => {
	it("describes a subagent Task call by its subagent_type and description", () => {
		const info = describeBackgroundTool({
			name: "ExecuteTask",
			arguments: { subagent_type: "explore", description: "find the bug", prompt: "long prompt..." },
		});
		expect(info.isMcpTool).toBe(false);
		expect(info.subagentType).toBe("explore");
		expect(info.label).toContain("explore");
		expect(info.summary).toBe("find the bug");
	});

	it("falls back to the prompt's first line when no description is given", () => {
		const info = describeBackgroundTool({
			name: "ExecuteTask",
			arguments: { subagent_type: "explore", prompt: "first line\nsecond line" },
		});
		expect(info.summary).toBe("first line");
	});

	it("describes an MCP tool by its de-prefixed name and an args summary", () => {
		const info = describeBackgroundTool({
			name: "mcp_web_fetch",
			arguments: { url: "https://example.com", limit: 5 },
		});
		expect(info.isMcpTool).toBe(true);
		expect(info.subagentType).toBe("mcp_web_fetch");
		// Prefix dropped for display.
		expect(info.label).toContain("web_fetch");
		expect(info.label).not.toContain("mcp_web_fetch");
		expect(info.summary).toContain("url: https://example.com");
	});
});

describe("background start/finish messages stay in sync", () => {
	it("subagent placeholder and finish message share the same label", () => {
		const toolCall = { name: "ExecuteTask", arguments: { subagent_type: "review", description: "review the diff" } };
		const placeholder = createBackgroundPlaceholderText(toolCall);
		const finish = createBackgroundTaskMessage(bgResult("Task", toolCall.arguments, "looks good"));

		const label = describeBackgroundTool(toolCall).label;
		const finishHeader = (finish.content as Array<{ text: string }>)[0].text;

		expect(placeholder).toContain(label);
		expect(finishHeader).toContain(label);
		// Verbose: the placeholder explains delegation + that the result arrives later.
		expect(placeholder).toContain("isolated context");
		expect(placeholder).toContain("follow-up message");
		// Finish carries the original result content after the header.
		expect((finish.content as Array<{ text: string }>)[1].text).toBe("looks good");
		expect(finish.customType).toBe(BACKGROUND_TASK_CUSTOM_TYPE);
		expect(finish.details).toMatchObject({ subagentType: "review", isMcpTool: false, isError: false });
	});

	it("MCP placeholder and finish message share the same label", () => {
		const toolCall = { name: "mcp_web_fetch", arguments: { url: "https://example.com" } };
		const placeholder = createBackgroundPlaceholderText(toolCall);
		const finish = createBackgroundTaskMessage(bgResult("mcp_web_fetch", toolCall.arguments, "{}"));

		const label = describeBackgroundTool(toolCall).label;
		const finishHeader = (finish.content as Array<{ text: string }>)[0].text;

		expect(placeholder).toContain(label);
		expect(finishHeader).toContain(label);
		// Verbose: the placeholder explains it is an external MCP server call.
		expect(placeholder).toContain("MCP server");
		expect(finish.details).toMatchObject({ isMcpTool: true });
	});

	it("marks a failed finish as failed", () => {
		const finish = createBackgroundTaskMessage(bgResult("Task", { subagent_type: "explore" }, "boom", true));
		const header = (finish.content as Array<{ text: string }>)[0].text;
		expect(header).toContain("failed");
		expect(finish.details).toMatchObject({ isError: true });
	});
});
