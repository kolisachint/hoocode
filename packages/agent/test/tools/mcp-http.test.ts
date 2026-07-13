import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { connectHttpMcpServer } from "../../src/tools/mcp-http-transport.js";
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
	params?: { name?: string; arguments?: { text?: string }; protocolVersion?: string };
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
			return {
				protocolVersion: "2025-03-26",
				capabilities: { tools: {} },
				serverInfo: { name: "stub", version: "1" },
			};
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

interface SeenRequest {
	method: string;
	headers: IncomingMessage["headers"];
}

/**
 * Minimal Streamable HTTP MCP server: JSON-RPC over POST; responses are plain
 * JSON or (when `sseResponses`) a one-shot SSE body. Issues an `Mcp-Session-Id`
 * on initialize, 405s the optional GET notification stream, and records the
 * headers of each request.
 */
async function startStreamableStub(opts: { sseResponses?: boolean } = {}) {
	const seenHeaders: SeenRequest[] = [];
	const server = createServer(async (req: IncomingMessage, res: ServerResponse) => {
		if (req.method === "GET") {
			res.writeHead(405).end();
			return;
		}
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
 * the POST endpoint; responses to POSTed requests arrive on the stream. When
 * `reject405OnMcpPost`, POSTs to the /mcp URL itself are answered 405 so a
 * streamable-HTTP attempt fails over to SSE.
 */
async function startLegacySseStub(opts: { reject405OnMcpPost?: boolean } = {}) {
	let stream: ServerResponse | undefined;
	const server = createServer(async (req: IncomingMessage, res: ServerResponse) => {
		const path = new URL(req.url ?? "/", "http://127.0.0.1").pathname;
		if (req.method === "GET" && path === "/mcp") {
			stream = res;
			res.writeHead(200, { "content-type": "text/event-stream" });
			res.write("event: endpoint\ndata: /messages\n\n");
			return;
		}
		if (req.method === "POST" && path === "/mcp" && opts.reject405OnMcpPost) {
			await readBody(req);
			res.writeHead(405).end();
			return;
		}
		if (req.method === "POST" && path === "/messages") {
			const msg = JSON.parse(await readBody(req)) as JsonRpcRequest;
			res.writeHead(202).end();
			if (msg.id !== undefined) {
				stream?.write(
					`event: message\ndata: ${JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: handleRpc(msg) })}\n\n`,
				);
			}
			return;
		}
		res.writeHead(404).end();
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

/**
 * Streamable HTTP MCP server guarded by OAuth: 401 (with RFC 9728 discovery
 * pointers) unless the request bears `Bearer <goodToken>`. Implements
 * protected-resource + authorization-server metadata, dynamic registration,
 * an instantly-approving /authorize, and a /token endpoint handling both
 * authorization_code + PKCE and refresh_token grants.
 */
async function startOAuthStub() {
	const state = {
		goodToken: "access-good",
		authCode: "auth-code-1",
		refreshedFrom: [] as string[],
		tokenRequests: [] as URLSearchParams[],
		registrations: 0,
	};
	let origin = "";
	const server = createServer(async (req: IncomingMessage, res: ServerResponse) => {
		const url = new URL(req.url ?? "/", origin);
		const respondJson = (status: number, body: unknown, headers: Record<string, string> = {}) => {
			res.writeHead(status, { "content-type": "application/json", ...headers });
			res.end(JSON.stringify(body));
		};

		if (url.pathname === "/.well-known/oauth-protected-resource") {
			respondJson(200, { resource: `${origin}/mcp`, authorization_servers: [origin] });
			return;
		}
		if (url.pathname === "/.well-known/oauth-authorization-server") {
			respondJson(200, {
				issuer: origin,
				authorization_endpoint: `${origin}/authorize`,
				token_endpoint: `${origin}/token`,
				registration_endpoint: `${origin}/register`,
				response_types_supported: ["code"],
				grant_types_supported: ["authorization_code", "refresh_token"],
				code_challenge_methods_supported: ["S256"],
				token_endpoint_auth_methods_supported: ["none"],
			});
			return;
		}
		if (url.pathname === "/register" && req.method === "POST") {
			const body = JSON.parse(await readBody(req)) as Record<string, unknown>;
			state.registrations++;
			respondJson(201, { ...body, client_id: "client-1" });
			return;
		}
		if (url.pathname === "/authorize" && req.method === "GET") {
			const redirectUri = url.searchParams.get("redirect_uri");
			const authState = url.searchParams.get("state");
			expect(url.searchParams.get("code_challenge_method")).toBe("S256");
			expect(url.searchParams.get("code_challenge")).toBeTruthy();
			const target = new URL(redirectUri!);
			target.searchParams.set("code", state.authCode);
			if (authState) target.searchParams.set("state", authState);
			res.writeHead(302, { location: target.toString() }).end();
			return;
		}
		if (url.pathname === "/token" && req.method === "POST") {
			const params = new URLSearchParams(await readBody(req));
			state.tokenRequests.push(params);
			if (params.get("grant_type") === "authorization_code") {
				expect(params.get("code")).toBe(state.authCode);
				expect(params.get("code_verifier")).toBeTruthy();
				respondJson(200, {
					access_token: state.goodToken,
					token_type: "bearer",
					expires_in: 3600,
					refresh_token: "refresh-1",
				});
			} else if (params.get("grant_type") === "refresh_token") {
				state.refreshedFrom.push(params.get("refresh_token") ?? "");
				respondJson(200, {
					access_token: state.goodToken,
					token_type: "bearer",
					expires_in: 3600,
					refresh_token: "refresh-2",
				});
			} else {
				respondJson(400, { error: "unsupported_grant_type" });
			}
			return;
		}
		if (url.pathname === "/mcp") {
			if (req.method === "GET") {
				res.writeHead(405).end();
				return;
			}
			if (req.method === "DELETE") {
				res.writeHead(200).end();
				return;
			}
			const body = await readBody(req);
			if (req.headers.authorization !== `Bearer ${state.goodToken}`) {
				res.writeHead(401, {
					"content-type": "application/json",
					"www-authenticate": `Bearer resource_metadata="${origin}/.well-known/oauth-protected-resource"`,
				});
				res.end(JSON.stringify({ error: "unauthorized" }));
				return;
			}
			const msg = JSON.parse(body) as JsonRpcRequest;
			if (msg.id === undefined) {
				res.writeHead(202).end();
				return;
			}
			res.writeHead(200, { "content-type": "application/json" });
			res.end(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result: handleRpc(msg) }));
			return;
		}
		res.writeHead(404).end();
	});
	const url = await listen(server);
	origin = url.replace(/\/mcp$/, "");
	return { url, state, close: () => server.close() };
}

/** A "browser" that immediately completes the authorization redirect. */
async function autoApproveBrowser(authorizationUrl: string): Promise<void> {
	const res = await fetch(authorizationUrl, { redirect: "manual" });
	const location = res.headers.get("location");
	expect(location).toBeTruthy();
	await fetch(location!);
}

/** Path of the persisted auth-state file for a server URL (mirrors mcp-oauth.ts). */
function authStatePath(storageDir: string, serverUrl: string): string {
	const hash = createHash("sha256").update(serverUrl).digest("hex").slice(0, 12);
	return join(storageDir, `${new URL(serverUrl).hostname}-${hash}.json`);
}

describe("remote MCP servers", () => {
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
			headers: { "x-api-key": "test-token" },
		});
		expect(tools).toHaveLength(1);
		expect(tools[0].name).toBe("mcp_remote_echo");

		const result = await tools[0].execute("call-1", { text: "hi remote" });
		const first = result.content[0];
		expect(first?.type === "text" ? first.text : "").toContain("echo: hi remote");

		// Every request carries the configured headers; requests after initialize
		// echo the server-issued session id.
		for (const seen of stub.seenHeaders) {
			expect(seen.headers["x-api-key"]).toBe("test-token");
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

	it("falls back from Streamable HTTP to SSE when the endpoint 405s", async () => {
		const stub = await startLegacySseStub({ reject405OnMcpPost: true });
		closers.push(stub.close);

		// type "http" attempts streamable first; the 405 flips it to legacy SSE.
		const tools = await loadFromConfig("fallback.json", { type: "http", url: stub.url });
		expect(tools).toHaveLength(1);

		const result = await tools[0].execute("call-1", { text: "fell back" });
		const first = result.content[0];
		expect(first?.type === "text" ? first.text : "").toContain("echo: fell back");
	});

	it("reconnects after terminate with a fresh session", async () => {
		const stub = await startStreamableStub();
		closers.push(stub.close);
		const config = { name: "remote", url: stub.url } as const;

		const first = connectHttpMcpServer(config);
		await first.rpc(
			"initialize",
			{ protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "t" } },
			5000,
		);
		first.notify("notifications/initialized");
		expect(await first.rpc("tools/list", {}, 5000)).toMatchObject({ tools: [ECHO_TOOL] });
		first.terminate();

		await expect(first.rpc("tools/list", {}, 5000)).rejects.toThrow(/terminated/);

		const second = connectHttpMcpServer(config);
		try {
			await second.rpc(
				"initialize",
				{ protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "t" } },
				5000,
			);
			second.notify("notifications/initialized");
			const result = (await second.rpc("tools/call", { name: "echo", arguments: { text: "again" } }, 5000)) as {
				content: Array<{ text: string }>;
			};
			expect(result.content[0].text).toContain("echo: again");
		} finally {
			second.terminate();
		}
	});

	it("rejects a remote entry without a url", async () => {
		await expect(loadFromConfig("bad.json", { type: "http" })).rejects.toThrow(/no "url"/);
	});

	it("completes the browser OAuth flow (auth code + PKCE) and persists tokens", async () => {
		const stub = await startOAuthStub();
		closers.push(stub.close);
		const storageDir = join(dir, "auth-interactive");
		let sawAuthUrl: string | undefined;

		const configPath = join(dir, "oauth.json");
		await writeFile(configPath, JSON.stringify({ mcpServers: { rovo: { type: "http", url: stub.url } } }));
		const tools = await loadMcpTools(configPath, {
			authStorageDir: storageDir,
			openBrowser: autoApproveBrowser,
			onAuthRequired: (authorizationUrl) => {
				sawAuthUrl = authorizationUrl;
			},
		});

		expect(tools).toHaveLength(1);
		const result = await tools[0].execute("call-1", { text: "authed" });
		const first = result.content[0];
		expect(first?.type === "text" ? first.text : "").toContain("echo: authed");

		expect(sawAuthUrl).toContain("/authorize");
		expect(stub.state.registrations).toBe(1);
		// Tokens persisted for future sessions.
		const persisted = JSON.parse(readFileSync(authStatePath(storageDir, stub.url), "utf8")) as {
			tokens?: { access_token: string; refresh_token: string };
			clientInformation?: { client_id: string };
		};
		expect(persisted.tokens?.access_token).toBe("access-good");
		expect(persisted.tokens?.refresh_token).toBe("refresh-1");
		expect(persisted.clientInformation?.client_id).toBe("client-1");
	});

	it("refreshes an expired token without opening a browser", async () => {
		const stub = await startOAuthStub();
		closers.push(stub.close);
		const storageDir = join(dir, "auth-refresh");

		// Pre-seed persisted state from a "previous session": registered client +
		// stale access token + valid refresh token.
		mkdirSync(storageDir, { recursive: true });
		writeFileSync(
			authStatePath(storageDir, stub.url),
			JSON.stringify({
				serverUrl: stub.url,
				clientInformation: { client_id: "client-1" },
				tokens: { access_token: "stale", token_type: "bearer", refresh_token: "refresh-1" },
			}),
		);

		const conn = connectHttpMcpServer(
			{ name: "rovo", url: stub.url, type: "http" },
			{
				authStorageDir: storageDir,
				openBrowser: () => {
					throw new Error("browser must not open for a refresh");
				},
			},
		);
		try {
			await conn.rpc(
				"initialize",
				{ protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "t" } },
				10000,
			);
			conn.notify("notifications/initialized");
			expect(await conn.rpc("tools/list", {}, 5000)).toMatchObject({ tools: [ECHO_TOOL] });
		} finally {
			conn.terminate();
		}

		expect(stub.state.refreshedFrom).toEqual(["refresh-1"]);
		const persisted = JSON.parse(readFileSync(authStatePath(storageDir, stub.url), "utf8")) as {
			tokens?: { access_token: string; refresh_token: string };
		};
		expect(persisted.tokens?.access_token).toBe("access-good");
		expect(persisted.tokens?.refresh_token).toBe("refresh-2");
	});
});
