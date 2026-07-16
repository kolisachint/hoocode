import { Type } from "typebox";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getModel } from "../src/models.js";
import { convertResponsesTools } from "../src/providers/openai-responses-shared.js";
import { streamSimple } from "../src/stream.js";
import type { Tool } from "../src/types.js";
import { toStrictJsonSchema } from "../src/utils/tool-constraints.js";

const mockState = vi.hoisted(() => ({
	lastParams: undefined as unknown,
}));

vi.mock("openai", () => {
	class FakeOpenAI {
		chat = {
			completions: {
				create: (params: unknown) => {
					mockState.lastParams = params;
					const stream = {
						async *[Symbol.asyncIterator]() {
							yield {
								choices: [{ delta: {}, finish_reason: "stop" }],
								usage: {
									prompt_tokens: 1,
									completion_tokens: 1,
									prompt_tokens_details: { cached_tokens: 0 },
									completion_tokens_details: { reasoning_tokens: 0 },
								},
							};
						},
					};
					const promise = Promise.resolve(stream) as Promise<typeof stream> & {
						withResponse: () => Promise<{
							data: typeof stream;
							response: { status: number; headers: Headers };
						}>;
					};
					promise.withResponse = async () => ({
						data: stream,
						response: { status: 200, headers: new Headers() },
					});
					return promise;
				},
			},
		};
	}

	return { default: FakeOpenAI };
});

const editSchema = Type.Object({
	path: Type.String({ description: "File path", minLength: 1 }),
	oldText: Type.String(),
	newText: Type.String(),
	replaceAll: Type.Optional(Type.Boolean({ default: false })),
});

const editTool: Tool = {
	name: "edit",
	description: "Replace exact text",
	parameters: editSchema,
};

type SentFunctionTool = {
	function?: {
		strict?: boolean;
		parameters?: {
			required?: string[];
			additionalProperties?: boolean;
			properties?: Record<string, Record<string, unknown>>;
		};
	};
};

async function streamWithTools(
	model: Parameters<typeof streamSimple>[0],
	options: Record<string, unknown>,
): Promise<SentFunctionTool | undefined> {
	await streamSimple(
		model,
		{
			messages: [{ role: "user", content: "edit the file", timestamp: Date.now() }],
			tools: [editTool],
		},
		{ apiKey: "test", ...options } as Parameters<typeof streamSimple>[2],
	).result();
	const params = mockState.lastParams as { tools?: SentFunctionTool[] };
	return params.tools?.[0];
}

describe("toStrictJsonSchema", () => {
	it("closes objects and promotes optionals to nullable required", () => {
		const strict = toStrictJsonSchema(editSchema);
		expect(strict.additionalProperties).toBe(false);
		expect(strict.required).toEqual(["path", "oldText", "newText", "replaceAll"]);
		const properties = strict.properties as Record<string, Record<string, unknown>>;
		// Optional property becomes nullable so "not provided" stays expressible.
		expect(properties.replaceAll.type).toEqual(["boolean", "null"]);
		// Required properties keep their plain type.
		expect(properties.path.type).toBe("string");
		// Unsupported validation keywords are stripped; descriptions survive.
		expect(properties.path.minLength).toBeUndefined();
		expect(properties.replaceAll.default).toBeUndefined();
		expect(properties.path.description).toBe("File path");
	});

	it("recurses into arrays and nested objects", () => {
		const nested = Type.Object({
			edits: Type.Array(
				Type.Object({
					oldText: Type.String(),
					newText: Type.String(),
					replaceAll: Type.Optional(Type.Boolean()),
				}),
			),
		});
		const strict = toStrictJsonSchema(nested);
		const properties = strict.properties as Record<string, Record<string, unknown>>;
		const items = properties.edits.items as Record<string, unknown>;
		expect(items.additionalProperties).toBe(false);
		expect(items.required).toEqual(["oldText", "newText", "replaceAll"]);
		const itemProperties = items.properties as Record<string, Record<string, unknown>>;
		expect(itemProperties.replaceAll.type).toEqual(["boolean", "null"]);
	});

	it("makes enum and anyOf properties nullable instead of widening type", () => {
		const schema = {
			type: "object",
			properties: {
				mode: { enum: ["a", "b"] },
				value: { anyOf: [{ type: "string" }, { type: "number" }] },
			},
		};
		const strict = toStrictJsonSchema(schema);
		const properties = strict.properties as Record<string, Record<string, unknown>>;
		expect(properties.mode.enum).toEqual(["a", "b", null]);
		expect(properties.value.anyOf).toEqual([{ type: "string" }, { type: "number" }, { type: "null" }]);
	});
});

describe("openai-completions constrained tool calls", () => {
	beforeEach(() => {
		mockState.lastParams = undefined;
	});

	it("sends strict function calling with a closed schema when constrained", async () => {
		// api.openai.com auto-detects toolCallConstraint: "strict".
		const { compat: _compat, ...baseModel } = getModel("openai", "gpt-4o-mini")!;
		const model = { ...baseModel, api: "openai-completions" } as const;
		const tool = await streamWithTools(model, { constrainToolCalls: true });

		expect(tool?.function?.strict).toBe(true);
		expect(tool?.function?.parameters?.additionalProperties).toBe(false);
		expect(tool?.function?.parameters?.required).toEqual(["path", "oldText", "newText", "replaceAll"]);
		expect(tool?.function?.parameters?.properties?.replaceAll.type).toEqual(["boolean", "null"]);
	});

	it("keeps tool calls unconstrained when the compat capability is none", async () => {
		const { compat: _compat, ...baseModel } = getModel("openai", "gpt-4o-mini")!;
		// Simulates a runtime that cannot constrain decoding, opted out via compat.
		const model = { ...baseModel, api: "openai-completions", compat: { toolCallConstraint: "none" } } as const;
		const tool = await streamWithTools(model, { constrainToolCalls: true });

		expect(tool?.function?.strict).toBe(false);
		expect(tool?.function?.parameters?.additionalProperties).toBeUndefined();
	});

	it("keeps existing behavior when the flag is unset", async () => {
		const { compat: _compat, ...baseModel } = getModel("openai", "gpt-4o-mini")!;
		const model = { ...baseModel, api: "openai-completions" } as const;
		const tool = await streamWithTools(model, {});

		expect(tool?.function?.strict).toBe(false);
		expect(tool?.function?.parameters?.additionalProperties).toBeUndefined();
	});

	it("honors an explicit compat opt-in for local runtimes", async () => {
		const { compat: _compat, ...baseModel } = getModel("openai", "gpt-4o-mini")!;
		// A local vLLM-style endpoint that supports strict schemas.
		const model = {
			...baseModel,
			api: "openai-completions",
			baseUrl: "http://localhost:8000/v1",
			compat: { toolCallConstraint: "strict" },
		} as const;
		const tool = await streamWithTools(model, { constrainToolCalls: true });

		expect(tool?.function?.strict).toBe(true);
		expect(tool?.function?.parameters?.additionalProperties).toBe(false);
	});
});

describe("openai-responses constrained tool calls", () => {
	it("sends strict: true with a closed schema when constrained", () => {
		const [tool] = convertResponsesTools([editTool], { constrainToolCalls: true });
		if (tool.type !== "function") {
			throw new Error("expected a function tool");
		}
		expect(tool.strict).toBe(true);
		const parameters = tool.parameters as {
			additionalProperties?: boolean;
			required?: string[];
			properties?: Record<string, Record<string, unknown>>;
		};
		expect(parameters.additionalProperties).toBe(false);
		expect(parameters.required).toEqual(["path", "oldText", "newText", "replaceAll"]);
		expect(parameters.properties?.replaceAll.type).toEqual(["boolean", "null"]);
	});

	it("keeps the passthrough schema and strict default without the flag", () => {
		const [tool] = convertResponsesTools([editTool]);
		if (tool.type !== "function") {
			throw new Error("expected a function tool");
		}
		expect(tool.strict).toBe(false);
		const parameters = tool.parameters as { additionalProperties?: boolean };
		expect(parameters.additionalProperties).toBeUndefined();
	});
});
