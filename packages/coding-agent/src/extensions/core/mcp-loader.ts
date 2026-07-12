/**
 * MCP server loader — discovers server configs (standard mcp.json locations,
 * hoocode's per-server JSON files, plugin registrations), connects via JSON-RPC
 * 2.0 over stdio, and registers each server tool as `mcp_<server>_<tool>`.
 *
 * Config sources (first-wins by server name):
 *   1. ~/.agents/mcp.json (user), ./.agents/mcp.json (project),
 *      ~/.config/claude/mcp.json (Claude Desktop)
 *   2. ~/.hoocode/mcp-servers/*.json and ./.hoocode/mcp-servers/*.json
 *   3. MCP servers registered by plugins/extensions during load
 */

import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { readdir } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import type { AgentToolResult, AgentToolUpdateCallback } from "@kolisachint/hoocode-agent-core";
import { summarizeArgs } from "@kolisachint/hoocode-agent-core";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { getHooCodeDir } from "../../config.js";
import { getExtensionMcpServers } from "../../core/extension-mcp-servers.js";
import type { ExtensionAPI, ExtensionContext, SessionStartEvent, ToolDefinition } from "../../core/extensions/types.js";
import { deferMcpSchemas, subagentSkipMcp } from "../../core/subagent-depth.js";
import { taskStore } from "../../core/task-store.js";
import { type DeferredMcpToolEntry, formatDeferredCatalog, selectResolvable } from "./mcp-deferred.js";

const HOOCODE_DIR = getHooCodeDir();

interface McpToolDef {
	name: string;
	description: string;
	inputSchema?: {
		type?: string;
		properties?: Record<string, { type?: string; description?: string }>;
		required?: string[];
	};
}

export interface McpServerConfig {
	/** Unique server identifier used as prefix for registered tool names */
	name: string;
	/** Executable to spawn */
	command: string;
	/** Optional arguments passed to the command */
	args?: string[];
	/** Optional extra environment variables for the server process */
	env?: Record<string, string>;
	/** Run MCP tools in background by default (default: true for MCP servers) */
	background?: boolean;
}

/** Standard MCP config format used by Claude Desktop and other tools */
interface StandardMcpServerConfig {
	command: string;
	args?: string[];
	env?: Record<string, string>;
	background?: boolean;
}

interface StandardMcpConfig {
	mcpServers?: Record<string, StandardMcpServerConfig>;
}

interface McpConnection {
	rpc(method: string, params?: unknown, timeoutMs?: number): Promise<unknown>;
	/** Send a JSON-RPC notification (no id, no response expected). */
	notify(method: string, params?: unknown): void;
	terminate(): void;
}

const mcpConnections = new Map<string, McpConnection>();
/**
 * Server configs retained by name so a tool call can transparently reconnect a
 * dropped server (process churn between turns, server exit, a racing teardown)
 * instead of permanently failing with "not connected".
 */
const mcpServerConfigs = new Map<string, McpServerConfig>();

/** Timeout for the connection handshake (initialize / tools/list). Tool calls
 *  themselves are left untimed since MCP tools can be long-running. */
const MCP_HANDSHAKE_TIMEOUT_MS = 15000;

function spawnMcpServer(config: McpServerConfig): McpConnection {
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
			const msg = JSON.parse(line) as {
				id?: number;
				result?: unknown;
				error?: { message: string };
			};
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
		mcpConnections.delete(config.name);
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

async function connectMcpServer(config: McpServerConfig): Promise<{ conn: McpConnection; tools: McpToolDef[] }> {
	mcpConnections.get(config.name)?.terminate();

	const conn = spawnMcpServer(config);
	mcpConnections.set(config.name, conn);

	await conn.rpc(
		"initialize",
		{
			protocolVersion: "2024-11-05",
			capabilities: { tools: {} },
			clientInfo: { name: "hoocode", version: "1.0.0" },
		},
		MCP_HANDSHAKE_TIMEOUT_MS,
	);

	// Per the MCP spec the client must acknowledge a successful initialize with the
	// initialized notification before issuing further requests; strict servers gate
	// tools/call on it.
	conn.notify("notifications/initialized");

	const toolsResult = (await conn.rpc("tools/list", {}, MCP_HANDSHAKE_TIMEOUT_MS)) as {
		tools?: McpToolDef[];
	};
	return { conn, tools: toolsResult.tools ?? [] };
}

/**
 * Return a live connection for a server, lazily reconnecting from the retained
 * config when the previous connection was torn down. Returns undefined only when
 * no config is known or a fresh connect attempt fails.
 */
async function getOrConnectMcp(name: string): Promise<McpConnection | undefined> {
	const existing = mcpConnections.get(name);
	if (existing) return existing;
	const config = mcpServerConfigs.get(name);
	if (!config) return undefined;
	try {
		const { conn } = await connectMcpServer(config);
		return conn;
	} catch {
		return undefined;
	}
}

let mcpExitCleanupInstalled = false;
/** Kill spawned MCP servers when the host process exits so they don't linger as
 *  orphans (their stdin merely goes idle, which doesn't terminate them). */
function installMcpExitCleanup(): void {
	if (mcpExitCleanupInstalled) return;
	mcpExitCleanupInstalled = true;
	process.once("exit", () => {
		for (const conn of mcpConnections.values()) {
			try {
				conn.terminate();
			} catch {
				// best-effort cleanup
			}
		}
		mcpConnections.clear();
	});
}

function buildMcpSchema(tool: McpToolDef): ReturnType<typeof Type.Object> {
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

/**
 * Parse standard MCP config format (used by Claude Desktop, VS Code, etc.)
 * into hoocode's McpServerConfig format.
 */
function parseStandardMcpConfig(config: StandardMcpConfig, _source: string): McpServerConfig[] {
	if (!config.mcpServers) return [];

	const servers: McpServerConfig[] = [];
	for (const [name, serverConfig] of Object.entries(config.mcpServers)) {
		servers.push({
			name,
			command: serverConfig.command,
			args: serverConfig.args,
			env: serverConfig.env,
			background: serverConfig.background,
		});
	}
	return servers;
}

/**
 * Load MCP servers from a standard mcp.json file.
 * Returns an array of McpServerConfig, or empty array if file doesn't exist or is invalid.
 */
function loadStandardMcpFile(filePath: string): McpServerConfig[] {
	if (!existsSync(filePath)) return [];

	try {
		const content = readFileSync(filePath, "utf8");
		const config = JSON.parse(content) as StandardMcpConfig;
		return parseStandardMcpConfig(config, filePath);
	} catch {
		return [];
	}
}

/**
 * Build the full {@link ToolDefinition} for one MCP tool — the complete JSON
 * schema plus the connect/execute machinery. Shared by the eager path (register
 * every tool up front) and the deferred path (materialize on resolve), so both
 * produce identical, callable tools.
 */
function buildMcpToolDefinition(serverConfig: McpServerConfig, tool: McpToolDef): ToolDefinition {
	const toolName = `mcp_${serverConfig.name}_${tool.name}`;
	const schema = buildMcpSchema(tool);
	const capturedServer = serverConfig.name;
	const capturedTool = tool.name;
	// MCP tools default to background mode since they are external processes with potential high latency
	const isBackground = serverConfig.background !== false;

	return {
		name: toolName,
		label: `[MCP] ${serverConfig.name} › ${tool.name}`,
		description: tool.description,
		parameters: schema,
		background: isBackground,
		// Render a clean, prefixed title in chat — `MCP [server › tool] <args>` —
		// parallel to the subagent `Task [type] <desc>` line. Without this the
		// ToolExecutionComponent falls back to the raw `mcp_<server>_<tool>` name.
		// The args summary reuses the same helper as the background start/finish
		// messages so the chat title stays in sync with them.
		renderCall(args, theme) {
			const summary = summarizeArgs((args ?? {}) as Record<string, unknown>);
			const text =
				theme.fg("toolTitle", theme.bold("MCP ")) +
				theme.fg("accent", `[${capturedServer} › ${capturedTool}]`) +
				(summary ? theme.fg("dim", ` ${summary}`) : "");
			return new Text(text, 0, 0);
		},
		async execute(
			_toolCallId: string,
			params: Static<typeof schema>,
			signal: AbortSignal,
			_onUpdate: AgentToolUpdateCallback,
		): Promise<AgentToolResult<undefined>> {
			// Background MCP tools get a task store entry so they appear in the task pane.
			// Foreground tools skip this (their result is awaited inline). The server
			// name rides in subagentMode and becomes the row's `[server]` origin tag;
			// the title carries just the tool.
			const task = isBackground
				? taskStore.create(capturedTool, { source: "mcp", subagentMode: capturedServer })
				: undefined;
			if (task) taskStore.update(task.id, { status: "in_progress" });

			// Lazily (re)connect: a dropped connection (server exit, process churn
			// between turns, a racing teardown) should transparently reconnect from
			// the retained config rather than permanently fail with "not connected".
			const activeConn = await getOrConnectMcp(capturedServer);
			if (!activeConn) {
				if (task) taskStore.update(task.id, { status: "failed" });
				return {
					content: [
						{ type: "text", text: `MCP server "${capturedServer}" is not connected (reconnect attempt failed)` },
					],
					details: undefined,
				};
			}

			try {
				const abortPromise = new Promise<never>((_, reject) => {
					signal.addEventListener("abort", () => reject(new Error("Aborted")));
				});

				const result = await Promise.race([
					activeConn.rpc("tools/call", { name: capturedTool, arguments: params }),
					abortPromise,
				]);

				if (task) taskStore.update(task.id, { status: "done" });
				return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }], details: undefined };
			} catch (error) {
				if (task) taskStore.update(task.id, { status: "failed" });
				throw error;
			}
		},
	} as ToolDefinition;
}

const RESOLVE_MCP_TOOLS_NAME = "ResolveMcpTools";

export function setupMcpLoader(pi: ExtensionAPI): void {
	pi.on("session_start", async (_event: SessionStartEvent, ctx: ExtensionContext) => {
		// A spawned subagent whose tool allowlist has no MCP tools is told by its
		// parent to skip server connection entirely (see SUBAGENT_SKIP_MCP_ENV).
		// Each connect is a ~15s-timeout handshake; doing it for a subagent that can
		// never call the tools is pure startup latency.
		if (subagentSkipMcp()) return;

		installMcpExitCleanup();
		const allServerConfigs: McpServerConfig[] = [];
		const seenNames = new Set<string>();

		// 1. Load from standard mcp.json locations
		// User-level: ~/.agents/mcp.json
		const userAgentsConfig = loadStandardMcpFile(join(homedir(), ".agents", "mcp.json"));
		for (const config of userAgentsConfig) {
			if (!seenNames.has(config.name)) {
				seenNames.add(config.name);
				allServerConfigs.push(config);
			}
		}

		// Project-level: ./.agents/mcp.json
		const projectAgentsConfig = loadStandardMcpFile(join(ctx.cwd, ".agents", "mcp.json"));
		for (const config of projectAgentsConfig) {
			if (!seenNames.has(config.name)) {
				seenNames.add(config.name);
				allServerConfigs.push(config);
			}
		}

		// Claude Desktop: ~/.config/claude/mcp.json
		const claudeDesktopConfig = loadStandardMcpFile(join(homedir(), ".config", "claude", "mcp.json"));
		for (const config of claudeDesktopConfig) {
			if (!seenNames.has(config.name)) {
				seenNames.add(config.name);
				allServerConfigs.push(config);
			}
		}

		// 2. Load from hoocode's per-server format (existing behavior)
		const searchDirs = [join(HOOCODE_DIR, "mcp-servers"), join(ctx.cwd, ".hoocode", "mcp-servers")];

		for (const dir of searchDirs) {
			if (!existsSync(dir)) continue;

			let files: string[];
			try {
				files = (await readdir(dir)).filter((f) => f.endsWith(".json"));
			} catch {
				continue;
			}

			for (const file of files) {
				const cfgPath = join(dir, file);
				let serverConfig: McpServerConfig;

				try {
					serverConfig = JSON.parse(readFileSync(cfgPath, "utf8")) as McpServerConfig;
					if (!serverConfig.name || !serverConfig.command) {
						ctx.ui.notify(`MCP: config "${file}" is missing required "name" or "command"`, "warning");
						continue;
					}
				} catch (err) {
					ctx.ui.notify(`MCP: failed to parse "${file}": ${String(err)}`, "error");
					continue;
				}

				// Skip if already loaded from standard config
				if (seenNames.has(serverConfig.name)) continue;
				seenNames.add(serverConfig.name);
				allServerConfigs.push(serverConfig);
			}
		}

		// 2b. Load from plugins/extensions that registered MCP servers during load.
		for (const entry of getExtensionMcpServers()) {
			for (const serverConfig of parseStandardMcpConfig(
				{ mcpServers: entry.mcpServers },
				`plugin:${entry.source}`,
			)) {
				if (seenNames.has(serverConfig.name)) continue;
				seenNames.add(serverConfig.name);
				allServerConfigs.push(serverConfig);
			}
		}

		// Deferral (spec §2): inject MCP tool names only and materialize each schema
		// on demand via ResolveMcpTools. Opt-in and top-level only — a subagent that
		// needs MCP has this env cleared, so it eager-registers its allowlisted tools
		// at dispatch (the dispatch ↔ schema interaction) and they are immediately callable.
		const defer = deferMcpSchemas();
		const deferredCatalog: DeferredMcpToolEntry[] = [];
		// Retain each deferred tool's raw definition + config so ResolveMcpTools can
		// build the full ToolDefinition on request.
		const deferredByName = new Map<string, { serverConfig: McpServerConfig; tool: McpToolDef }>();
		const resolvedNames = new Set<string>();

		// 3. Connect to all servers and register (or defer) tools
		for (const serverConfig of allServerConfigs) {
			// Retain the config so a tool call can lazily reconnect a dropped server.
			mcpServerConfigs.set(serverConfig.name, serverConfig);
			try {
				const { tools } = await connectMcpServer(serverConfig);

				for (const tool of tools) {
					const toolName = `mcp_${serverConfig.name}_${tool.name}`;
					if (defer) {
						deferredCatalog.push({ toolName, server: serverConfig.name, description: tool.description });
						deferredByName.set(toolName, { serverConfig, tool });
					} else {
						pi.registerTool(buildMcpToolDefinition(serverConfig, tool));
					}
				}

				const bgMode = serverConfig.background !== false ? "background" : "foreground";
				const suffix = defer ? ", schemas deferred" : "";
				ctx.ui.notify(
					`MCP: connected "${serverConfig.name}" (${tools.length} tool${tools.length === 1 ? "" : "s"}, ${bgMode}${suffix})`,
					"info",
				);
			} catch (err) {
				ctx.ui.notify(`MCP: failed to connect "${serverConfig.name}": ${String(err)}`, "error");
			}
		}

		// 4. In deferred mode, register the single resolver that materializes schemas on demand.
		if (defer && deferredCatalog.length > 0) {
			const resolveParams = Type.Object(
				{
					names: Type.Array(Type.String(), {
						description: "MCP tool names to make callable (e.g. 'mcp_github_create_pr' or 'create_pr').",
					}),
				},
				{ additionalProperties: false },
			);
			pi.registerTool({
				name: RESOLVE_MCP_TOOLS_NAME,
				label: RESOLVE_MCP_TOOLS_NAME,
				description:
					"MCP tools are connected but their schemas are loaded on demand to keep context small. Call this with the tool name(s) you need to make them callable, then call the tool(s). Available MCP tools:\n" +
					formatDeferredCatalog(deferredCatalog),
				promptSnippet: "Resolve deferred MCP tool schemas by name before calling them.",
				parameters: resolveParams,
				async execute(_toolCallId: string, params: Static<typeof resolveParams>) {
					const matched = selectResolvable(deferredCatalog, params.names ?? []);
					const newlyResolved: string[] = [];
					for (const entry of matched) {
						if (resolvedNames.has(entry.toolName)) continue;
						const raw = deferredByName.get(entry.toolName);
						if (!raw) continue;
						pi.registerTool(buildMcpToolDefinition(raw.serverConfig, raw.tool));
						resolvedNames.add(entry.toolName);
						newlyResolved.push(entry.toolName);
					}
					const text = matched.length
						? `Resolved ${newlyResolved.length} MCP tool(s): ${matched.map((m) => m.toolName).join(", ")}. They are now callable.`
						: `No MCP tools matched: ${(params.names ?? []).join(", ") || "(none)"}. See ResolveMcpTools for the catalog.`;
					return { content: [{ type: "text" as const, text }], details: undefined };
				},
			} as ToolDefinition);
		}
	});
}
