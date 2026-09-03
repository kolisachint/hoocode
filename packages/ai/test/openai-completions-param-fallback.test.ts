import { beforeEach, describe, expect, it, vi } from "vitest";
import { getModel } from "../src/models.js";
import { streamOpenAICompletions } from "../src/providers/openai-completions.js";
import { resetRejectedParams } from "../src/providers/param-fallback.js";
import type { Model } from "../src/types.js";

type CapturedPayload = Record<string, unknown>;

/**
 * A 4xx the way the OpenAI SDK surfaces one: the proxy's complaint on
 * `message`, and a `status` that says it was the request that was refused.
 */
class FakeAPIError extends Error {
	constructor(
		readonly status: number,
		message: string,
	) {
		super(message);
	}
}

const mockState = vi.hoisted(() => ({
	payloads: [] as CapturedPayload[],
	/** Params this fake endpoint refuses, named one per response like a strict validator. */
	forbidden: [] as string[],
}));

vi.mock("openai", () => {
	class FakeOpenAI {
		chat = {
			completions: {
				create: (params: CapturedPayload) => {
					mockState.payloads.push(params);
					const offending = mockState.forbidden.find((param) => params[param] !== undefined);
					if (offending) {
						return {
							withResponse: async () => {
								throw new FakeAPIError(422, `${offending}: Extra inputs are not permitted.`);
							},
						};
					}
					const stream = {
						async *[Symbol.asyncIterator]() {
							yield {
								choices: [{ delta: { content: "ok" }, finish_reason: "stop" }],
								usage: { prompt_tokens: 1, completion_tokens: 1 },
							};
						},
					};
					return {
						withResponse: async () => ({
							data: stream,
							response: { status: 200, headers: new Headers() },
						}),
					};
				},
			},
		};

		constructor(_options: unknown) {}
	}

	return { default: FakeOpenAI };
});

describe("openai-completions unsupported-param fallback", () => {
	beforeEach(() => {
		mockState.payloads = [];
		mockState.forbidden = [];
		resetRejectedParams();
		delete process.env.HOOCODE_CACHE_RETENTION;
	});

	/** A model on a gateway of our own, so base-URL detection stays out of it. */
	function gatewayModel(baseUrl = "https://cortex.example.com/v1"): Model<"openai-completions"> {
		const { compat: _compat, ...base } = getModel("openai", "gpt-4o-mini");
		return {
			...(base as Omit<Model<"openai-completions">, "api">),
			api: "openai-completions",
			provider: "openai",
			baseUrl,
		};
	}

	async function run(model: Model<"openai-completions">, sessionId = "session-1") {
		return streamOpenAICompletions(
			model,
			{ systemPrompt: "sys", messages: [{ role: "user", content: "hi", timestamp: Date.now() }] },
			{ apiKey: "test-key", sessionId },
		).result();
	}

	it("retries without the param a strict gateway refuses, and succeeds", async () => {
		mockState.forbidden = ["prompt_cache_retention"];

		const result = await run(gatewayModel());

		expect(result.stopReason).toBe("stop");
		expect(mockState.payloads).toHaveLength(2);
		expect(mockState.payloads[0]?.prompt_cache_retention).toBe("24h");
		expect(mockState.payloads[1]?.prompt_cache_retention).toBeUndefined();
		// Only the named param goes; the rest of the request is untouched.
		expect(mockState.payloads[1]?.prompt_cache_key).toBe("session-1");
		expect(mockState.payloads[1]?.messages).toEqual(mockState.payloads[0]?.messages);
	});

	it("drops one param per pass when the gateway refuses several", async () => {
		mockState.forbidden = ["prompt_cache_retention", "store"];

		const result = await run(gatewayModel());

		expect(result.stopReason).toBe("stop");
		expect(mockState.payloads).toHaveLength(3);
		expect(mockState.payloads[2]?.prompt_cache_retention).toBeUndefined();
		expect(mockState.payloads[2]?.store).toBeUndefined();
	});

	it("remembers the refusal, so later requests cost no extra round trip", async () => {
		mockState.forbidden = ["prompt_cache_retention"];
		const model = gatewayModel();

		await run(model);
		mockState.payloads = [];
		await run(model);

		expect(mockState.payloads).toHaveLength(1);
		expect(mockState.payloads[0]?.prompt_cache_retention).toBeUndefined();
	});

	// The point of keying by base URL: one strict gateway must not turn off
	// caching for every other endpoint in the process.
	it("keeps what it learned to the endpoint that taught it", async () => {
		mockState.forbidden = ["prompt_cache_retention"];
		await run(gatewayModel());

		mockState.forbidden = [];
		mockState.payloads = [];
		await run(gatewayModel("https://other.example.com/v1"));

		expect(mockState.payloads[0]?.prompt_cache_retention).toBe("24h");
	});

	it("does not drop params on an error that names none of them", async () => {
		const model = gatewayModel();
		mockState.forbidden = ["messages"];

		const result = await run(model);

		expect(result.stopReason).toBe("error");
		expect(mockState.payloads).toHaveLength(1);
	});
});
