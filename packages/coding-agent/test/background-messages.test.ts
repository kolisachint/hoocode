import type { BackgroundToolResult } from "@kolisachint/hoocode-agent-core";
import {
	BACKGROUND_TASK_CUSTOM_TYPE,
	createBackgroundPlaceholderText,
	createBackgroundTaskMessage,
	describeBackgroundTool,
} from "@kolisachint/hoocode-agent-core";
import { describe, expect, it } from "vitest";

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
			name: "Task",
			arguments: { subagent_type: "explore", description: "find the bug", prompt: "long prompt..." },
		});
		expect(info.isMcpTool).toBe(false);
		expect(info.subagentType).toBe("explore");
		expect(info.label).toContain("explore");
		expect(info.summary).toBe("find the bug");
	});

	it("falls back to the prompt's first line when no description is given", () => {
		const info = describeBackgroundTool({
			name: "Task",
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
	it("subagent placeholder is one compact line and the finish passes the tool's notification through", () => {
		const toolCall = { name: "Task", arguments: { subagent_type: "review", description: "review the diff" } };
		const placeholder = createBackgroundPlaceholderText(toolCall);
		const finish = createBackgroundTaskMessage(
			bgResult("Task", toolCall.arguments, "review#1 finished ✓ — looks good"),
		);

		const label = describeBackgroundTool(toolCall).label;
		expect(placeholder).toContain(label);
		// Compact: a single line that points at TaskOutput, not a multi-line explainer.
		expect(placeholder.split("\n")).toHaveLength(1);
		expect(placeholder).toContain("TaskOutput");
		// Subagent finish is the tool's compact notification verbatim — no extra header,
		// no inlined body (the body is pulled from the inbox).
		expect((finish.content as Array<{ text: string }>)[0].text).toBe("review#1 finished ✓ — looks good");
		expect(finish.content).toHaveLength(1);
		expect(finish.customType).toBe(BACKGROUND_TASK_CUSTOM_TYPE);
		expect(finish.details).toMatchObject({ subagentType: "review", isMcpTool: false, isError: false });
	});

	it("MCP placeholder is compact and the finish keeps a header + full body", () => {
		const toolCall = { name: "mcp_web_fetch", arguments: { url: "https://example.com" } };
		const placeholder = createBackgroundPlaceholderText(toolCall);
		const finish = createBackgroundTaskMessage(bgResult("mcp_web_fetch", toolCall.arguments, "{}"));

		const label = describeBackgroundTool(toolCall).label;
		const finishHeader = (finish.content as Array<{ text: string }>)[0].text;

		expect(placeholder).toContain(label);
		expect(placeholder.split("\n")).toHaveLength(1);
		expect(finishHeader).toContain(label);
		// MCP tools have no inbox, so the body is still inlined after the header.
		expect((finish.content as Array<{ text: string }>)[1].text).toBe("{}");
		expect(finish.details).toMatchObject({ isMcpTool: true });
	});

	it("marks a failed subagent finish via details, content passed through", () => {
		const finish = createBackgroundTaskMessage(
			bgResult("Task", { subagent_type: "explore" }, "explore#1 failed ✗ — boom", true),
		);
		expect((finish.content as Array<{ text: string }>)[0].text).toBe("explore#1 failed ✗ — boom");
		expect(finish.details).toMatchObject({ isError: true });
	});
});
