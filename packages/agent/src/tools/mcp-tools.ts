/**
 * Headless MCP tool loader: parse a standard mcp.json file (the
 * `{ "mcpServers": { ... } }` format used by Claude Desktop, VS Code, and the
 * hoocode CLI), connect the declared servers — stdio (`command`), Streamable
 * HTTP (`{ "type": "http", "url": ... }`), or legacy SSE (`"type": "sse"`) —
 * and expose their tools as AgentTool instances usable by any Agent in any
 * process.
 *
 * Connections are tracked per loader call and reaped on process exit so
 * spawned servers never linger as orphans. Call closeMcpTools() to terminate
 * them earlier (tests, graceful shutdown).
 */

import { type ChildProcess, spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { createInterface } from "node:readline";
import { type TObject, Type } from "typebox";
import type { AgentTool, AgentToolResult } from "../types.js";
import { connectHttpMcpServer } from "./mcp-http-transport.js";

export interface McpToolsServerConfig {
	/** Unique server identifier used as prefix for tool names. */
	name: string;
	command: string;
	args?: string[];
	env?: Record<string, string>;
}

interface McpToolDef {
	name: string;
	description?: string;
	inputSchema?: {
		type?: string;
		properties?: Record<string, { type?: string; description?: string }>;
		required?: string[];
	};
}

interface McpConnection {
	rpc(method: string, params?: unknown, timeoutMs?: number): Promise<unknown>;
	notify(method: string, params?: unknown): void;
	terminate(): void;
}

/** Timeout for the connection handshake (initialize / tools/list). Tool calls
 *  themselves are left untimed since MCP tools can be long-running. */
const MCP_HANDSHAKE_TIMEOUT_MS = 15000;

const liveConnections = new Set<McpConnection>();
let exitCleanupInstalled = false;

/** Kill spawned MCP servers when the host process exits so they don't linger
 *  as orphans (their stdin merely goes idle, which doesn't terminate them). */
function installExitCleanup(): void {
	if (exitCleanupInstalled) return;
	exitCleanupInstalled = true;
	process.once("exit", () => {
		closeMcpTools();
	});
}

/** Terminate every MCP server spawned by loadMcpTools() in this process. */
export function closeMcpTools(): void {
	for (const conn of liveConnections) {
		try {
			conn.terminate();
		} catch {
			// best-effort cleanup
		}
	}
	liveConnections.clear();
}

function spawnMcpServer(config: McpToolsServerConfig): McpConnection {
	const proc: ChildProcess = spawn(config.command, config.args ?? [], {
		env: { ...process.env, ...(config.env ?? {}) },
		stdio: ["pipe", "pipe", "pipe"],
	});

	let nextId = 1;
	const pending = new Map<number, { resolve: (r: unknown) => void; reject: (e: Error) => void }>();

	const rl = createInterface({ input: proc.stdout! });
	rl.on("line", (line) => {
		if (!line.trim()) return;
		try {
			const msg = JSON.parse(line) as { id?: number; result?: unknown; error?: { message: string } };
			if (msg.id === undefined) return;
			const cb = pending.get(msg.id);
			if (!cb) return;
			pending.delete(msg.id);
			if (msg.error) cb.reject(new Error(msg.error.message));
			else cb.resolve(msg.result);
		} catch {
			// ignore non-JSON server startup output
		}
	});

	proc.on("exit", () => {
		for (const cb of pending.values()) cb.reject(new Error(`MCP server "${config.name}" exited unexpectedly`));
		pending.clear();
	});

	function rpc(method: string, params?: unknown, timeoutMs?: number): Promise<unknown> {
		const id = nextId++;
		return new Promise<unknown>((resolve, reject) => {
			let timer: NodeJS.Timeout | undefined;
			if (timeoutMs && timeoutMs > 0) {
				timer = setTimeout(() => {
					if (pending.delete(id)) {
						reject(new Error(`MCP server "${config.name}" timed out after ${timeoutMs}ms on ${method}`));
					}
				}, timeoutMs);
				timer.unref?.();
			}
			pending.set(id, {
				resolve: (r) => {
					if (timer) clearTimeout(timer);
					resolve(r);
				},
				reject: (e) => {
					if (timer) clearTimeout(timer);
					reject(e);
				},
			});
			proc.stdin!.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
		});
	}

	function notify(method: string, params?: unknown): void {
		proc.stdin!.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
	}

	return {
		rpc,
		notify,
		terminate: () => {
			rl.close();
			proc.kill();
		},
	};
}

async function handshake(conn: McpConnection): Promise<McpToolDef[]> {
	await conn.rpc(
		"initialize",
		{
			protocolVersion: "2024-11-05",
			capabilities: { tools: {} },
			clientInfo: { name: "hoocode-agent-core", version: "1.0.0" },
		},
		MCP_HANDSHAKE_TIMEOUT_MS,
	);
	// Per the MCP spec the client must acknowledge a successful initialize with
	// the initialized notification before issuing further requests; strict
	// servers gate tools/call on it.
	conn.notify("notifications/initialized");
	const toolsResult = (await conn.rpc("tools/list", {}, MCP_HANDSHAKE_TIMEOUT_MS)) as { tools?: McpToolDef[] };
	return toolsResult.tools ?? [];
}

async function connectMcpServer(config: McpToolsServerConfig): Promise<{ conn: McpConnection; tools: McpToolDef[] }> {
	const conn = spawnMcpServer(config);
	try {
		return { conn, tools: await handshake(conn) };
	} catch (error) {
		conn.terminate();
		throw error;
	}
}

function buildMcpSchema(tool: McpToolDef): TObject {
	const props = tool.inputSchema?.properties ?? {};
	const required = new Set(tool.inputSchema?.required ?? []);
	const shape: Record<string, ReturnType<typeof Type.String>> = {};

	for (const [key, prop] of Object.entries(props)) {
		let field: ReturnType<typeof Type.String>;
		switch (prop.type) {
			case "number":
			case "integer":
				field = Type.Number({ description: prop.description }) as unknown as ReturnType<typeof Type.String>;
				break;
			case "boolean":
				field = Type.Boolean({ description: prop.description }) as unknown as ReturnType<typeof Type.String>;
				break;
			default:
				field = Type.String({ description: prop.description });
		}
		shape[key] = required.has(key) ? field : (Type.Optional(field) as unknown as ReturnType<typeof Type.String>);
	}

	return Type.Object(shape);
}

function createMcpAgentTool(serverName: string, conn: McpConnection, tool: McpToolDef): AgentTool<any> {
	const schema = buildMcpSchema(tool);
	return {
		name: `mcp_${serverName}_${tool.name}`,
		label: `[MCP] ${serverName} › ${tool.name}`,
		description: tool.description ?? `MCP tool ${tool.name} from server ${serverName}`,
		parameters: schema,
		execute: async (_toolCallId, params, signal): Promise<AgentToolResult<undefined>> => {
			const abortPromise = new Promise<never>((_, reject) => {
				if (signal?.aborted) {
					reject(new Error("Aborted"));
					return;
				}
				signal?.addEventListener("abort", () => reject(new Error("Aborted")), { once: true });
			});
			const result = await Promise.race([
				conn.rpc("tools/call", { name: tool.name, arguments: params }),
				abortPromise,
			]);
			return {
				content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
				details: undefined,
			};
		},
	};
}

interface StandardMcpServerEntry {
	command?: string;
	args?: string[];
	env?: Record<string, string>;
	/** "stdio" (default with command), "http" (Streamable HTTP), or "sse" (legacy). */
	type?: string;
	/** Remote server URL for http/sse transports. */
	url?: string;
	/** Extra HTTP headers (e.g. Authorization) for remote transports. */
	headers?: Record<string, string>;
}

interface StandardMcpConfig {
	mcpServers?: Record<string, StandardMcpServerEntry>;
}

async function connectRemoteMcpServer(
	name: string,
	entry: StandardMcpServerEntry,
): Promise<{ conn: McpConnection; tools: McpToolDef[] }> {
	const conn = connectHttpMcpServer({
		name,
		url: entry.url!,
		headers: entry.headers,
		type: entry.type === "sse" ? "sse" : "http",
	});
	try {
		return { conn, tools: await handshake(conn) };
	} catch (error) {
		conn.terminate();
		throw error;
	}
}

/**
 * Parse a standard mcp.json file, connect every declared server — stdio
 * (`command`) or remote (`{ "type": "http" | "sse", "url": ... }`) — and
 * return their tools as AgentTool instances (named `mcp_<server>_<tool>`).
 *
 * An empty or server-less config resolves to []. A missing or malformed file,
 * or a server that fails its handshake, rejects — callers decide whether MCP
 * is optional. Connections are terminated automatically on process exit.
 */
export async function loadMcpTools(mcpJsonPath: string): Promise<AgentTool<any>[]> {
	const raw = await readFile(mcpJsonPath, "utf-8");
	const parsed = JSON.parse(raw) as StandardMcpConfig;
	const servers = Object.entries(parsed.mcpServers ?? {});
	if (servers.length === 0) return [];

	installExitCleanup();
	const tools: AgentTool<any>[] = [];
	for (const [name, serverConfig] of servers) {
		const isRemote =
			serverConfig &&
			(serverConfig.type === "http" ||
				serverConfig.type === "sse" ||
				(typeof serverConfig.command !== "string" && typeof serverConfig.url === "string"));
		let connected: { conn: McpConnection; tools: McpToolDef[] };
		if (isRemote) {
			if (typeof serverConfig.url !== "string") {
				throw new Error(`${mcpJsonPath}: mcpServers["${name}"] has type "${serverConfig.type}" but no "url"`);
			}
			connected = await connectRemoteMcpServer(name, serverConfig);
		} else {
			if (!serverConfig || typeof serverConfig.command !== "string") {
				throw new Error(
					`${mcpJsonPath}: mcpServers["${name}"] is missing a "command" (or a "url" for remote servers)`,
				);
			}
			connected = await connectMcpServer({
				name,
				command: serverConfig.command,
				args: serverConfig.args,
				env: serverConfig.env,
			});
		}
		liveConnections.add(connected.conn);
		for (const toolDef of connected.tools) {
			tools.push(createMcpAgentTool(name, connected.conn, toolDef));
		}
	}
	return tools;
}
