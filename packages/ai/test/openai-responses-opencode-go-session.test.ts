import { beforeEach, describe, expect, it, vi } from "vitest";
import { getModel } from "../src/models.js";
import { streamOpenAIResponses } from "../src/providers/openai-responses.js";

type ClientOptions = {
	apiKey: string;
	baseURL: string;
	dangerouslyAllowBrowser: boolean;
	defaultHeaders?: Record<string, string>;
};

const mockState = vi.hoisted(() => ({
	clientOptions: undefined as ClientOptions | undefined,
}));

vi.mock("openai", () => {
	class FakeOpenAI {
		responses = {
			create: () => {
				const responseStream = {
					async *[Symbol.asyncIterator]() {},
				};
				const promise = Promise.resolve(responseStream) as Promise<typeof responseStream> & {
					withResponse: () => Promise<{
						data: typeof responseStream;
						response: { status: number; headers: Headers };
					}>;
				};
				promise.withResponse = async () => ({
					data: responseStream,
					response: { status: 200, headers: new Headers() },
				});
				return promise;
			},
		};

		constructor(options: ClientOptions) {
			mockState.clientOptions = options;
		}
	}

	return { default: FakeOpenAI };
});

describe("OpenCode Go Responses session headers", () => {
	beforeEach(() => {
		mockState.clientOptions = undefined;
	});

	it("sends the required session and client-identification headers", async () => {
		const model = getModel("opencode-go", "grok-4.7");
		const stream = streamOpenAIResponses(
			model,
			{ messages: [{ role: "user", content: "hello", timestamp: Date.now() }] },
			{ apiKey: "test-key", sessionId: "go-session-456", cacheRetention: "none" },
		);
		await stream.result();

		expect(mockState.clientOptions?.defaultHeaders?.["x-opencode-session"]).toBe("go-session-456");
		expect(mockState.clientOptions?.defaultHeaders?.["user-agent"]).toBe("hoocode");
	});
});
