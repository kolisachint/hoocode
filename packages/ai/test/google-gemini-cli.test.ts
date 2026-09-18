/**
 * Offline coverage for the Cloud Code Assist (Gemini CLI / Antigravity) provider:
 * the request envelope both providers share, the retry-delay parser, and the
 * two OAuth providers the registry has to expose for `/login` to list them.
 */

import { Type } from "typebox";
import { describe, expect, it } from "vitest";
import { buildRequest, extractRetryDelay } from "../src/providers/google-gemini-cli.js";
import type { Context, Model, Tool } from "../src/types.js";
import { getOAuthProvider } from "../src/utils/oauth/index.js";

function model(overrides: Partial<Model<"google-gemini-cli">> = {}): Model<"google-gemini-cli"> {
	return {
		id: "gemini-3.1-pro-preview",
		name: "Gemini 3.1 Pro Preview (Cloud Code Assist)",
		api: "google-gemini-cli",
		provider: "google-gemini-cli",
		baseUrl: "https://cloudcode-pa.googleapis.com",
		reasoning: true,
		input: ["text", "image"],
		cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
		contextWindow: 1048576,
		maxTokens: 65535,
		...overrides,
	};
}

const context: Context = {
	systemPrompt: "You are a helpful assistant.",
	messages: [{ role: "user", content: "Hello", timestamp: Date.now() }],
};

const calculatorTool: Tool = {
	name: "math_operation",
	description: "Perform basic arithmetic operations",
	parameters: Type.Object({ a: Type.Number(), b: Type.Number() }),
};

describe("Cloud Code Assist request envelope", () => {
	it("wraps the Gemini request with project, model, and agent metadata", () => {
		const request = buildRequest(model(), context, "my-project");

		expect(request.project).toBe("my-project");
		expect(request.model).toBe("gemini-3.1-pro-preview");
		expect(request.userAgent).toBe("hoocode");
		expect(request.requestType).toBeUndefined();
		expect(request.request.systemInstruction?.parts).toEqual([{ text: "You are a helpful assistant." }]);
		expect(request.request.contents).toHaveLength(1);
	});

	it("marks Antigravity requests as agent requests and prepends its system instruction", () => {
		const request = buildRequest(
			model({ provider: "google-antigravity", id: "gemini-3.1-pro-high" }),
			context,
			"my-project",
			{},
			true,
		);

		expect(request.requestType).toBe("agent");
		expect(request.userAgent).toBe("antigravity");
		expect(request.requestId?.startsWith("agent-")).toBe(true);
		const parts = request.request.systemInstruction?.parts ?? [];
		expect(request.request.systemInstruction?.role).toBe("user");
		expect(parts[0]?.text).toContain("You are Antigravity");
		expect(parts.at(-1)?.text).toBe("You are a helpful assistant.");
	});

	it("sends Claude tool schemas as parameters and Gemini tool schemas as parametersJsonSchema", () => {
		const claude = buildRequest(
			model({ provider: "google-antigravity", id: "claude-sonnet-4-6" }),
			{ ...context, tools: [calculatorTool] },
			"my-project",
			{},
			true,
		);
		const gemini = buildRequest(model(), { ...context, tools: [calculatorTool] }, "my-project");

		const claudeDeclaration = claude.request.tools?.[0]?.functionDeclarations[0] ?? {};
		const geminiDeclaration = gemini.request.tools?.[0]?.functionDeclarations[0] ?? {};
		expect(claudeDeclaration).toHaveProperty("parameters");
		expect(geminiDeclaration).toHaveProperty("parametersJsonSchema");
	});

	it("maps a thinking level onto the generation config for Gemini 3 models", () => {
		const request = buildRequest(model(), context, "my-project", { thinking: { enabled: true, level: "HIGH" } });

		expect(request.request.generationConfig?.thinkingConfig).toMatchObject({
			includeThoughts: true,
			thinkingLevel: "HIGH",
		});
	});
});

describe("extractRetryDelay", () => {
	it("reads a quota reset duration from the error body", () => {
		expect(extractRetryDelay("Your quota will reset after 39s")).toBe(40000);
		expect(extractRetryDelay("Your quota will reset after 1h2m3s")).toBe(3724000);
	});

	it("prefers the Retry-After header", () => {
		const headers = new Headers({ "retry-after": "5" });
		expect(extractRetryDelay("Your quota will reset after 39s", headers)).toBe(6000);
	});

	it("returns undefined when no delay is advertised", () => {
		expect(extractRetryDelay("Internal error")).toBeUndefined();
	});
});

describe("Google OAuth providers", () => {
	it("registers the Gemini CLI provider and encodes token plus project into the API key", () => {
		const provider = getOAuthProvider("google-gemini-cli");
		expect(provider?.usesCallbackServer).toBe(true);
		expect(provider?.getApiKey({ access: "ya29.token", refresh: "r", expires: 0, projectId: "p" })).toBe(
			JSON.stringify({ token: "ya29.token", projectId: "p" }),
		);
	});

	it("registers the Antigravity provider", () => {
		const provider = getOAuthProvider("google-antigravity");
		expect(provider?.name).toContain("Antigravity");
		expect(provider?.getApiKey({ access: "ya29.token", refresh: "r", expires: 0, projectId: "p" })).toBe(
			JSON.stringify({ token: "ya29.token", projectId: "p" }),
		);
	});
});
