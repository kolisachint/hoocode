import * as http from "node:http";
import { afterEach, describe, expect, it } from "vitest";
import type { AssistantMessageEvent, Context, Model } from "../src/index.js";
import { streamOpenAICompletions } from "../src/providers/openai-completions.js";

interface RequestBody {
	messages: Array<{ role: string; content?: unknown }>;
}

function lastUserText(body: RequestBody): string {
	const userMsgs = body.messages.filter((m) => m.role === "user");
	const last = userMsgs[userMsgs.length - 1];
	if (typeof last.content === "string") return last.content;
	if (Array.isArray(last.content)) {
		return last.content
			.filter((p): p is { type: "text"; text: string } => (p as { type?: string }).type === "text")
			.map((p) => p.text)
			.join("");
	}
	return "";
}

function buildModel(baseUrl: string, promptSuffix?: string): Model<"openai-completions"> {
	return {
		id: "suffix-model",
		name: "Suffix Model",
		api: "openai-completions",
		provider: "suffix-provider",
		baseUrl,
		reasoning: false,
		input: ["text"],
		cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
		contextWindow: 128000,
		maxTokens: 4096,
		compat: promptSuffix ? { promptSuffix } : undefined,
	};
}

const context: Context = {
	messages: [{ role: "user", content: "summarize this", timestamp: 1 }],
};

async function drain(stream: AsyncIterable<AssistantMessageEvent>): Promise<void> {
	for await (const _ of stream) {
		// consume
	}
}

function makeServer(sink: RequestBody[]): Promise<{ url: string; close: () => Promise<void> }> {
	return new Promise((resolve) => {
		const server = http.createServer(async (req, res) => {
			let body = "";
			for await (const chunk of req) body += chunk.toString();
			sink.push(JSON.parse(body) as RequestBody);
			res.writeHead(200, { "content-type": "text/event-stream" });
			res.write(
				`data: ${JSON.stringify({
					id: "x",
					object: "chat.completion.chunk",
					created: 0,
					model: "suffix-model",
					choices: [{ index: 0, delta: { role: "assistant", content: "ok" }, finish_reason: null }],
				})}\n\n`,
			);
			res.write(
				`data: ${JSON.stringify({
					id: "x",
					object: "chat.completion.chunk",
					created: 0,
					model: "suffix-model",
					choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
					usage: { prompt_tokens: 1, completion_tokens: 1 },
				})}\n\n`,
			);
			res.write("data: [DONE]\n\n");
			res.end();
		});
		server.listen(0, "127.0.0.1", () => {
			const addr = server.address();
			const port = typeof addr === "object" && addr ? addr.port : 0;
			resolve({
				url: `http://127.0.0.1:${port}`,
				close: () => new Promise((r) => server.close(() => r())),
			});
		});
	});
}

describe("openai-completions promptSuffix", () => {
	afterEach(() => {
		delete process.env.OPENAI_API_KEY;
	});

	it("appends the suffix to the last user message", async () => {
		const sink: RequestBody[] = [];
		const server = await makeServer(sink);
		try {
			await drain(streamOpenAICompletions(buildModel(server.url, "/no_think"), context, { apiKey: "test" }));
			expect(sink).toHaveLength(1);
			expect(lastUserText(sink[0])).toBe("summarize this /no_think");
		} finally {
			await server.close();
		}
	});

	it("does not modify the prompt when no suffix is configured", async () => {
		const sink: RequestBody[] = [];
		const server = await makeServer(sink);
		try {
			await drain(streamOpenAICompletions(buildModel(server.url), context, { apiKey: "test" }));
			expect(sink).toHaveLength(1);
			expect(lastUserText(sink[0])).toBe("summarize this");
		} finally {
			await server.close();
		}
	});
});
