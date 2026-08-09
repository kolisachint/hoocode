/**
 * Per-tool `defer_loading` and the server-side tool-search tool.
 *
 * Deferral exists to make on-demand tool loading *cache-preserving*. Tools
 * render at position 0 of the prompt, so a tool the harness adds
 * mid-conversation invalidates the whole prefix — tools, system, and messages.
 * Declaring the tool up front as deferred and letting the server append its
 * schema on demand keeps the prefix stable instead.
 *
 * These assertions are on the wire format: what actually lands in the request
 * body. Whether the model then *calls* the search tool, and whether the cache
 * genuinely survives, are server-side behaviors this suite cannot observe.
 */

import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { Type } from "typebox";
import { describe, expect, it } from "vitest";
import { streamAnthropic } from "../src/providers/anthropic.js";
import type { CacheRetention, Context, Model, Tool } from "../src/types.js";

function createModel(baseUrl: string, compat?: Model<"anthropic-messages">["compat"]): Model<"anthropic-messages"> {
	return {
		id: "claude-opus-4-7",
		name: "Claude Opus 4.7",
		api: "anthropic-messages",
		provider: "test-anthropic",
		baseUrl,
		reasoning: true,
		input: ["text"],
		cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
		contextWindow: 200000,
		maxTokens: 32000,
		compat,
	};
}

function tool(name: string, deferLoading?: boolean): Tool {
	return {
		name,
		description: `Do ${name}`,
		parameters: Type.Object({ value: Type.String() }),
		...(deferLoading === undefined ? {} : { deferLoading }),
	};
}

async function readRequestBody(request: IncomingMessage): Promise<Record<string, unknown>> {
	const chunks: Buffer[] = [];
	for await (const chunk of request) {
		chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
	}
	return JSON.parse(Buffer.concat(chunks).toString("utf8")) as Record<string, unknown>;
}

function writeEmptySseResponse(response: ServerResponse): void {
	response.writeHead(200, { "content-type": "text/event-stream" });
	response.end();
}

async function captureTools(
	tools: Tool[],
	compat?: Model<"anthropic-messages">["compat"],
	cacheRetention: CacheRetention = "none",
): Promise<Array<Record<string, unknown>>> {
	let body: Record<string, unknown> | undefined;

	const server = createServer(async (request, response) => {
		body = await readRequestBody(request);
		writeEmptySseResponse(response);
	});
	await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
	const address = server.address() as AddressInfo;

	try {
		const context: Context = {
			messages: [{ role: "user", content: "Use a tool", timestamp: Date.now() }],
			tools,
		};
		const stream = streamAnthropic(createModel(`http://127.0.0.1:${address.port}`, compat), context, {
			apiKey: "test-key",
			cacheRetention,
		});
		for await (const event of stream) {
			if (event.type === "done" || event.type === "error") break;
		}
	} finally {
		await new Promise<void>((resolve, reject) => server.close((e) => (e ? reject(e) : resolve())));
	}

	if (!body) throw new Error("request was not captured");
	return (body.tools ?? []) as Array<Record<string, unknown>>;
}

const SEARCH_TOOL_TYPE = "tool_search_tool_bm25_20251119";

describe("Anthropic tool search / defer_loading", () => {
	it("changes nothing when no tool opts in", async () => {
		// The safety property that makes `deferLoading` acceptable on the shared,
		// provider-agnostic Tool type: a request that defers nothing is identical
		// to one sent before the field existed.
		const tools = await captureTools([tool("alpha"), tool("beta")]);
		expect(tools).toHaveLength(2);
		expect(tools.every((t) => t.defer_loading === undefined)).toBe(true);
		expect(tools.some((t) => t.type === SEARCH_TOOL_TYPE)).toBe(false);
	});

	it("marks only the opted-in tools and appends the search tool", async () => {
		const tools = await captureTools([tool("eager"), tool("heavy", true)]);

		expect(tools).toHaveLength(3);
		expect(tools[0]).toMatchObject({ name: "eager" });
		expect(tools[0].defer_loading).toBeUndefined();
		expect(tools[1]).toMatchObject({ name: "heavy", defer_loading: true });
		expect(tools[2]).toMatchObject({ type: SEARCH_TOOL_TYPE, name: "tool_search_tool_bm25" });
	});

	it("keeps a non-deferred tool in the request even when every caller tool defers", async () => {
		// The API rejects a request whose tools are all deferred; the appended
		// search tool is what makes this shape legal.
		const tools = await captureTools([tool("a", true), tool("b", true)]);
		expect(tools.filter((t) => t.defer_loading === true)).toHaveLength(2);
		expect(tools.some((t) => t.defer_loading === undefined)).toBe(true);
		expect(tools[tools.length - 1]).toMatchObject({ type: SEARCH_TOOL_TYPE });
	});

	it("degrades to eager schemas on an endpoint that cannot defer", async () => {
		// Ignoring the flag beats a 400 from a field the endpoint never heard of.
		const tools = await captureTools([tool("heavy", true)], { supportsToolSearch: false });
		expect(tools).toHaveLength(1);
		expect(tools[0].defer_loading).toBeUndefined();
		expect(tools.some((t) => t.type === SEARCH_TOOL_TYPE)).toBe(false);
	});

	it("puts the cache breakpoint on the last tool in the final array", async () => {
		// On the search tool once one is appended — a breakpoint left on the last
		// caller tool would strand the search tool outside the cached prefix and
		// re-process it every request.
		const withSearch = await captureTools([tool("heavy", true)], undefined, "short");
		expect(withSearch[withSearch.length - 1]).toMatchObject({
			type: SEARCH_TOOL_TYPE,
			cache_control: { type: "ephemeral" },
		});
		expect(withSearch[0].cache_control).toBeUndefined();

		const withoutSearch = await captureTools([tool("a"), tool("b")], undefined, "short");
		expect(withoutSearch[1]).toMatchObject({ name: "b", cache_control: { type: "ephemeral" } });
		expect(withoutSearch[0].cache_control).toBeUndefined();
	});
});
