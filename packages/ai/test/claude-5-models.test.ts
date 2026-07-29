import { describe, expect, it, vi } from "vitest";
import { getModel, getModels, getSupportedThinkingLevels } from "../src/models.js";
import type { Context } from "../src/types.js";

const mockState = vi.hoisted(() => ({
	constructorOpts: undefined as Record<string, unknown> | undefined,
	createParams: undefined as Record<string, unknown> | undefined,
}));

vi.mock("@anthropic-ai/sdk", () => {
	function createSseResponse(): Response {
		const body = [
			`event: message_start\ndata: ${JSON.stringify({
				type: "message_start",
				message: {
					id: "msg_test",
					usage: { input_tokens: 10, output_tokens: 0 },
				},
			})}\n`,
			`event: message_delta\ndata: ${JSON.stringify({
				type: "message_delta",
				delta: { stop_reason: "end_turn" },
				usage: { output_tokens: 5 },
			})}\n`,
		].join("\n");

		return new Response(body, {
			status: 200,
			headers: { "content-type": "text/event-stream" },
		});
	}

	class FakeAnthropic {
		constructor(opts: Record<string, unknown>) {
			mockState.constructorOpts = opts;
		}
		messages = {
			create: (params: Record<string, unknown>) => {
				mockState.createParams = params;
				return {
					asResponse: async () => createSseResponse(),
				};
			},
		};
	}

	return { default: FakeAnthropic };
});

const context: Context = {
	systemPrompt: "You are a helpful assistant.",
	messages: [{ role: "user", content: "Hello", timestamp: Date.now() }],
};

describe("Claude Opus 5 catalog", () => {
	it("registers Claude Opus 5 on the Anthropic provider", () => {
		const model = getModel("anthropic", "claude-opus-5");

		expect(model).toBeDefined();
		expect(model.api).toBe("anthropic-messages");
		expect(model.provider).toBe("anthropic");
		expect(model.baseUrl).toBe("https://api.anthropic.com");
		expect(model.reasoning).toBe(true);
		expect(model.input).toEqual(["text", "image"]);
		expect(model.contextWindow).toBe(1000000);
		expect(model.maxTokens).toBe(128000);
		expect(model.cost).toEqual({
			input: 5,
			output: 25,
			cacheRead: 0.5,
			cacheWrite: 6.25,
		});
	});

	it("registers Claude Opus 5 on GitHub Copilot via the Anthropic Messages API", () => {
		// Regression: the Copilot API router only recognised `claude-*-4*`, so
		// Claude 5 models fell through to the OpenAI-compatible endpoint.
		const model = getModel("github-copilot", "claude-opus-5");

		expect(model).toBeDefined();
		expect(model.api).toBe("anthropic-messages");
		expect(model.provider).toBe("github-copilot");
		expect(model.baseUrl).toBe("https://api.individual.githubcopilot.com");
		expect(model.reasoning).toBe(true);
		expect(model.input).toEqual(["text", "image"]);
		expect(model.contextWindow).toBe(1000000);
		expect(model.maxTokens).toBe(64000);
		expect(model.cost).toEqual({
			input: 5,
			output: 25,
			cacheRead: 0.5,
			cacheWrite: 6.25,
		});
		expect(model.headers?.["Copilot-Integration-Id"]).toBe("vscode-chat");
		// openai-completions compat must not leak onto an Anthropic Messages model.
		expect(model.compat).toBeUndefined();
	});

	it("lists Claude Opus 5 for both configured providers so /model can offer it", () => {
		expect(getModels("anthropic").map((model) => model.id)).toContain("claude-opus-5");
		expect(getModels("github-copilot").map((model) => model.id)).toContain("claude-opus-5");
	});

	it("exposes xhigh thinking for Claude Opus 5 on both providers", () => {
		expect(getSupportedThinkingLevels(getModel("anthropic", "claude-opus-5"))).toContain("xhigh");
		expect(getSupportedThinkingLevels(getModel("github-copilot", "claude-opus-5"))).toContain("xhigh");
	});
});

describe("Claude Sonnet 5 catalog", () => {
	it("registers Claude Sonnet 5 on the Anthropic provider", () => {
		const model = getModel("anthropic", "claude-sonnet-5");

		expect(model).toBeDefined();
		expect(model.api).toBe("anthropic-messages");
		expect(model.provider).toBe("anthropic");
		expect(model.baseUrl).toBe("https://api.anthropic.com");
		expect(model.reasoning).toBe(true);
		expect(model.input).toEqual(["text", "image"]);
		expect(model.contextWindow).toBe(1000000);
		expect(model.maxTokens).toBe(128000);
		expect(model.cost).toEqual({
			input: 2,
			output: 10,
			cacheRead: 0.2,
			cacheWrite: 2.5,
		});
	});

	it("registers Claude Sonnet 5 on GitHub Copilot via the Anthropic Messages API", () => {
		// Regression: this entry was generated as openai-completions by the old
		// `claude-*-4*` API router.
		const model = getModel("github-copilot", "claude-sonnet-5");

		expect(model).toBeDefined();
		expect(model.api).toBe("anthropic-messages");
		expect(model.provider).toBe("github-copilot");
		expect(model.baseUrl).toBe("https://api.individual.githubcopilot.com");
		expect(model.reasoning).toBe(true);
		expect(model.input).toEqual(["text", "image"]);
		expect(model.contextWindow).toBe(1000000);
		expect(model.maxTokens).toBe(128000);
		expect(model.cost).toEqual({
			input: 2,
			output: 10,
			cacheRead: 0.2,
			cacheWrite: 2.5,
		});
		expect(model.headers?.["Copilot-Integration-Id"]).toBe("vscode-chat");
		expect(model.compat).toBeUndefined();
	});

	it("exposes xhigh thinking for Claude Sonnet 5 on both providers", () => {
		expect(getSupportedThinkingLevels(getModel("anthropic", "claude-sonnet-5"))).toContain("xhigh");
		expect(getSupportedThinkingLevels(getModel("github-copilot", "claude-sonnet-5"))).toContain("xhigh");
	});
});

describe("Claude Fable 5 catalog", () => {
	it("registers Claude Fable 5 on the Anthropic provider", () => {
		const model = getModel("anthropic", "claude-fable-5");

		expect(model).toBeDefined();
		expect(model.api).toBe("anthropic-messages");
		expect(model.provider).toBe("anthropic");
		expect(model.baseUrl).toBe("https://api.anthropic.com");
		expect(model.reasoning).toBe(true);
		expect(model.input).toEqual(["text", "image"]);
		expect(model.contextWindow).toBe(1000000);
		expect(model.maxTokens).toBe(128000);
		expect(model.cost).toEqual({
			input: 10,
			output: 50,
			cacheRead: 1,
			cacheWrite: 12.5,
		});
	});

	it("registers Claude Fable 5 on GitHub Copilot via the Anthropic Messages API", () => {
		// Regression: `fable` was missing from the Copilot API router's family
		// list, so this entry was generated as openai-completions.
		const model = getModel("github-copilot", "claude-fable-5");

		expect(model).toBeDefined();
		expect(model.api).toBe("anthropic-messages");
		expect(model.provider).toBe("github-copilot");
		expect(model.baseUrl).toBe("https://api.individual.githubcopilot.com");
		expect(model.reasoning).toBe(true);
		expect(model.input).toEqual(["text", "image"]);
		expect(model.contextWindow).toBe(1000000);
		expect(model.maxTokens).toBe(128000);
		expect(model.cost).toEqual({
			input: 10,
			output: 50,
			cacheRead: 1,
			cacheWrite: 12.5,
		});
		expect(model.headers?.["Copilot-Integration-Id"]).toBe("vscode-chat");
		expect(model.compat).toBeUndefined();
	});

	it("exposes xhigh thinking for Claude Fable 5 on both providers", () => {
		expect(getSupportedThinkingLevels(getModel("anthropic", "claude-fable-5"))).toContain("xhigh");
		expect(getSupportedThinkingLevels(getModel("github-copilot", "claude-fable-5"))).toContain("xhigh");
	});
});

describe("Claude Opus 5 request format", () => {
	it("uses adaptive thinking instead of a thinking budget on Anthropic", async () => {
		const model = getModel("anthropic", "claude-opus-5");
		const { streamSimpleAnthropic } = await import("../src/providers/anthropic.js");

		const s = streamSimpleAnthropic(model, context, { apiKey: "sk-ant-test", reasoning: "xhigh" });
		for await (const event of s) {
			if (event.type === "error") break;
		}

		const params = mockState.createParams as Record<string, unknown>;
		expect(params.model).toBe("claude-opus-5");
		// Adaptive-thinking models reject `thinking.type = "enabled"` / budget_tokens.
		expect(params.thinking).toEqual({ type: "adaptive", display: "omitted" });
		expect(params.output_config).toEqual({ effort: "xhigh" });
	});

	it("uses adaptive thinking and Copilot auth headers on GitHub Copilot", async () => {
		const model = getModel("github-copilot", "claude-opus-5");
		const { streamSimpleAnthropic } = await import("../src/providers/anthropic.js");

		const s = streamSimpleAnthropic(model, context, {
			apiKey: "tid_copilot_session_test_token",
			reasoning: "high",
		});
		for await (const event of s) {
			if (event.type === "error") break;
		}

		const opts = mockState.constructorOpts!;
		expect(opts.apiKey).toBeNull();
		expect(opts.authToken).toBe("tid_copilot_session_test_token");
		const headers = opts.defaultHeaders as Record<string, string>;
		expect(headers["User-Agent"]).toContain("GitHubCopilotChat");
		expect(headers["Copilot-Integration-Id"]).toBe("vscode-chat");

		const params = mockState.createParams as Record<string, unknown>;
		expect(params.model).toBe("claude-opus-5");
		expect(params.thinking).toEqual({ type: "adaptive", display: "omitted" });
		expect(params.output_config).toEqual({ effort: "high" });
	});

	it.each([
		["anthropic", "claude-sonnet-5"],
		["github-copilot", "claude-sonnet-5"],
		["anthropic", "claude-fable-5"],
		["github-copilot", "claude-fable-5"],
	] as const)("uses adaptive thinking for %s %s", async (provider, modelId) => {
		const model = getModel(provider, modelId);
		const { streamSimpleAnthropic } = await import("../src/providers/anthropic.js");

		const s = streamSimpleAnthropic(model, context, { apiKey: "test-token", reasoning: "xhigh" });
		for await (const event of s) {
			if (event.type === "error") break;
		}

		const params = mockState.createParams as Record<string, unknown>;
		expect(params.model).toBe(modelId);
		// Sonnet 5 and Fable 5 keep the visible "summarized" display; only the
		// Opus tier defaults to "omitted".
		expect(params.thinking).toEqual({ type: "adaptive", display: "summarized" });
		expect(params.output_config).toEqual({ effort: "xhigh" });
	});
});

describe("OpenRouter rolling Claude aliases", () => {
	// The `~anthropic/claude-<family>-latest` aliases resolve to the Claude 5
	// releases (their pricing and limits match exactly), so they need the same
	// xhigh thinking level as the pinned ids.
	it.each([
		"~anthropic/claude-opus-latest",
		"~anthropic/claude-sonnet-latest",
		"~anthropic/claude-fable-latest",
	] as const)("exposes xhigh thinking for %s", (modelId) => {
		const model = getModel("openrouter", modelId);
		expect(model).toBeDefined();
		expect(getSupportedThinkingLevels(model)).toContain("xhigh");
	});

	it("does not expose xhigh for the Haiku alias, whose current release is 4.5", () => {
		const model = getModel("openrouter", "~anthropic/claude-haiku-latest");
		expect(model).toBeDefined();
		expect(getSupportedThinkingLevels(model)).not.toContain("xhigh");
	});
});
