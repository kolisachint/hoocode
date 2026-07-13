import { type AssistantMessage, type AssistantMessageEvent, type Context, EventStream } from "@kolisachint/hoocode-ai";
import { Type } from "typebox";
import { describe, expect, it } from "vitest";
import { Agent } from "../src/index.js";
import type { AgentTool } from "../src/types.js";

// Mock stream that mimics AssistantMessageEventStream
class MockAssistantStream extends EventStream<AssistantMessageEvent, AssistantMessage> {
	constructor() {
		super(
			(event) => event.type === "done" || event.type === "error",
			(event) => {
				if (event.type === "done") return event.message;
				if (event.type === "error") return event.error;
				throw new Error("Unexpected event type");
			},
		);
	}
}

function createUsage() {
	return {
		input: 0,
		output: 0,
		cacheRead: 0,
		cacheWrite: 0,
		totalTokens: 0,
		cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
	};
}

function createAssistantMessage(
	content: AssistantMessage["content"],
	stopReason: AssistantMessage["stopReason"] = "stop",
): AssistantMessage {
	return {
		role: "assistant",
		content,
		api: "openai-responses",
		provider: "openai",
		model: "mock",
		usage: createUsage(),
		stopReason,
		timestamp: Date.now(),
	};
}

describe("prepareNextTurn mid-run context refresh", () => {
	it("delivers a mid-run systemPrompt/tools change to the next provider request in the SAME run", async () => {
		// The scenario behind single-turn plugin install → use: a tool call mutates
		// agent.state (new capability in the system prompt), and prepareNextTurn —
		// assigned AFTER the agent was constructed, like AgentSession does — must
		// hand the refreshed context to the loop before the next provider request.
		const seenSystemPrompts: string[] = [];
		const seenToolNames: string[][] = [];

		let callIndex = 0;
		const streamFn = (_model: unknown, context: Context) => {
			seenSystemPrompts.push(context.systemPrompt ?? "");
			seenToolNames.push((context.tools ?? []).map((t: { name: string }) => t.name));
			const stream = new MockAssistantStream();
			queueMicrotask(() => {
				if (callIndex === 0) {
					const message = createAssistantMessage(
						[{ type: "toolCall", id: "tool-1", name: "install", arguments: {} }],
						"toolUse",
					);
					stream.push({ type: "done", reason: "toolUse", message });
				} else {
					const message = createAssistantMessage([{ type: "text", text: "done" }]);
					stream.push({ type: "done", reason: "stop", message });
				}
				callIndex++;
			});
			return stream as never;
		};

		const extraTool: AgentTool = {
			name: "installed-capability",
			label: "Installed capability",
			description: "Appears mid-run",
			parameters: Type.Object({}),
			async execute() {
				return { content: [{ type: "text", text: "ok" }], details: undefined };
			},
		};

		let contextDirty = false;
		const installTool: AgentTool = {
			name: "install",
			label: "Install",
			description: "Simulates InstallPlugin: mutates state mid-run",
			parameters: Type.Object({}),
			async execute() {
				agent.state.systemPrompt = "UPDATED: new skill available";
				agent.state.tools = [...agent.state.tools, extraTool];
				contextDirty = true;
				return { content: [{ type: "text", text: "installed" }], details: undefined };
			},
		};

		const agent = new Agent({
			initialState: { systemPrompt: "INITIAL", tools: [installTool] },
			streamFn: streamFn as never,
		});

		// Late assignment (after construction), exactly like AgentSession wires it.
		agent.prepareNextTurn = (loopContext) => {
			if (!contextDirty) return undefined;
			contextDirty = false;
			return {
				context: {
					systemPrompt: agent.state.systemPrompt,
					messages: loopContext.context.messages,
					tools: agent.state.tools.slice(),
				},
			};
		};

		await agent.prompt("install something and use it");

		expect(seenSystemPrompts).toEqual(["INITIAL", "UPDATED: new skill available"]);
		expect(seenToolNames[0]).toEqual(["install"]);
		expect(seenToolNames[1]).toEqual(["install", "installed-capability"]);
	});
});
