import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { closeMcpTools, loadMcpTools } from "../../src/tools/mcp-tools.js";

const ECHO_TOOL = {
	name: "echo",
	description: "Echo the given text back",
	inputSchema: {
		type: "object",
		properties: { text: { type: "string", description: "Text to echo" } },
		required: ["text"],
	},
};

interface JsonRpcRequest {
	id?: number;
	method: string;
	params?: { name?: string; arguments?: { text?: string } };
}

function readBody(req: IncomingMessage): Promise<string> {
	return new Promise((resolve, reject) => {
		let body = "";
		req.on("data", (chunk) => {
			body += chunk;
		});
		req.on("end", () => resolve(body));
		req.on("error", reject);
	});
}

function handleRpc(msg: JsonRpcRequest): unknown {
	switch (msg.method) {
		case "initialize":
			return { protocolVersion: "2025-03-26", capabilities: { tools: {} }, serverInfo: { name: "stub" } };
		case "tools/list":
			return { tools: [ECHO_TOOL] };
		case "tools/call":
			return { content: [{ type: "text", text: `echo: ${msg.params?.arguments?.text ?? ""}` }] };
		default:
			return {};
	}
}

function listen(server: Server): Promise<string> {
	return new Promise((resolve) => {
		server.listen(0, "127.0.0.1", () => {
			resolve(`http://127.0.0.1:${(server.address() as AddressInfo).port}/mcp`);
		});
	});
}

/**
 * Minimal Streamable HTTP MCP server: every message is POSTed here; responses
 * are plain JSON or (when `sseResponses`) a one-shot SSE body. Issues an
 * `Mcp-Session-Id` on initialize and records the headers of each request.
 */
async function startStreamableStub(opts: { sseResponses?: boolean } = {}) {
	const seenHeaders: Array<{ method: string; headers: IncomingMessage["headers"] }> = [];
	const server = createServer(async (req: IncomingMessage, res: ServerResponse) => {
		if (req.method === "DELETE") {
			res.writeHead(200).end();
			return;
		}
		const msg = JSON.parse(await readBody(req)) as JsonRpcRequest;
		seenHeaders.push({ method: msg.method, headers: req.headers });
		if (msg.id === undefined) {
			res.writeHead(202).end();
			return;
		}
		const response = { jsonrpc: "2.0", id: msg.id, result: handleRpc(msg) };
		const headers: Record<string, string> = {};
		if (msg.method === "initialize") headers["mcp-session-id"] = "sess-123";
		if (opts.sseResponses) {
			res.writeHead(200, { ...headers, "content-type": "text/event-stream" });
			res.end(`event: message\ndata: ${JSON.stringify(response)}\n\n`);
		} else {
			res.writeHead(200, { ...headers, "content-type": "application/json" });
			res.end(JSON.stringify(response));
		}
	});
	const url = await listen(server);
	return { url, seenHeaders, close: () => server.close() };
}

/**
 * Minimal legacy HTTP+SSE MCP server: GET opens the event stream and announces
 * the POST endpoint; responses to POSTed requests arrive on the stream.
 */
async function startLegacySseStub() {
	let stream: ServerResponse | undefined;
	const server = createServer(async (req: IncomingMessage, res: ServerResponse) => {
		if (req.method === "GET") {
			stream = res;
			res.writeHead(200, { "content-type": "text/event-stream" });
			res.write("event: endpoint\ndata: /messages\n\n");
			return;
		}
		const msg = JSON.parse(await readBody(req)) as JsonRpcRequest;
		res.writeHead(202).end();
		if (msg.id !== undefined) {
			stream?.write(
				`event: message\ndata: ${JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: handleRpc(msg) })}\n\n`,
			);
		}
	});
	const url = await listen(server);
	return {
		url,
		close: () => {
			stream?.end();
			server.close();
		},
	};
}

describe("loadMcpTools over HTTP", () => {
	let dir: string;
	const closers: Array<() => void> = [];

	beforeAll(async () => {
		dir = await mkdtemp(join(tmpdir(), "mcp-http-"));
	});

	afterEach(() => {
		closeMcpTools();
		for (const close of closers.splice(0)) close();
	});

	afterAll(async () => {
		await rm(dir, { recursive: true, force: true });
	});

	async function loadFromConfig(fileName: string, serverEntry: unknown) {
		const configPath = join(dir, fileName);
		await writeFile(configPath, JSON.stringify({ mcpServers: { remote: serverEntry } }));
		return loadMcpTools(configPath);
	}

	it("connects a Streamable HTTP server, echoes the session id, and sends custom headers", async () => {
		const stub = await startStreamableStub();
		closers.push(stub.close);

		const tools = await loadFromConfig("http.json", {
			type: "http",
			url: stub.url,
			headers: { authorization: "Bearer test-token" },
		});
		expect(tools).toHaveLength(1);
		expect(tools[0].name).toBe("mcp_remote_echo");

		const result = await tools[0].execute("call-1", { text: "hi remote" });
		const first = result.content[0];
		expect(first?.type === "text" ? first.text : "").toContain("echo: hi remote");

		// Every request carries the configured headers; requests after initialize
		// echo the server-issued session id.
		for (const seen of stub.seenHeaders) {
			expect(seen.headers.authorization).toBe("Bearer test-token");
		}
		const toolsCall = stub.seenHeaders.find((h) => h.method === "tools/call");
		expect(toolsCall?.headers["mcp-session-id"]).toBe("sess-123");
	});

	it("parses SSE-formatted responses from a Streamable HTTP server", async () => {
		const stub = await startStreamableStub({ sseResponses: true });
		closers.push(stub.close);

		const tools = await loadFromConfig("http-sse-resp.json", { type: "http", url: stub.url });
		expect(tools).toHaveLength(1);

		const result = await tools[0].execute("call-1", { text: "streamed" });
		const first = result.content[0];
		expect(first?.type === "text" ? first.text : "").toContain("echo: streamed");
	});

	it("defaults to Streamable HTTP when only a url is given", async () => {
		const stub = await startStreamableStub();
		closers.push(stub.close);

		const tools = await loadFromConfig("url-only.json", { url: stub.url });
		expect(tools).toHaveLength(1);
		expect(tools[0].name).toBe("mcp_remote_echo");
	});

	it("connects a legacy SSE server via the endpoint event", async () => {
		const stub = await startLegacySseStub();
		closers.push(stub.close);

		const tools = await loadFromConfig("sse.json", { type: "sse", url: stub.url });
		expect(tools).toHaveLength(1);
		expect(tools[0].name).toBe("mcp_remote_echo");

		const result = await tools[0].execute("call-1", { text: "over sse" });
		const first = result.content[0];
		expect(first?.type === "text" ? first.text : "").toContain("echo: over sse");
	});

	it("rejects a remote entry without a url", async () => {
		await expect(loadFromConfig("bad.json", { type: "http" })).rejects.toThrow(/no "url"/);
	});
});
