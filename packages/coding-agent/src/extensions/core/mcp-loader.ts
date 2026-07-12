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
import { type ExtensionMcpServerConfig, getExtensionMcpServers } from "../../core/extension-mcp-servers.js";
import type { ExtensionAPI, ExtensionContext, SessionStartEvent, ToolDefinition } from "../../core/extensions/types.js";
import { deferMcpSchemas, subagentSkipMcp } from "../../core/subagent-depth.js";
import { taskStore } from "../../core/task-store.js";
import {
	connectAllInOrder,
	type DeferredMcpToolEntry,
	formatDeferredCatalog,
	selectResolvable,
} from "./mcp-deferred.js";

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
	/**
	 * One-line steering text for the system prompt ("Use these tools for GitHub
	 * operations instead of bash git"). Eager mode applies it to each of the
	 * server's tools; deferred mode shows it on the server's catalog line.
	 */
	promptSnippet?: string;
	/**
	 * Guideline bullets appended to the system prompt while the server's tools
	 * (eager) or the ResolveMcpTools resolver (deferred) are registered.
	 * Duplicates are collapsed by the prompt builder.
	 */
	promptGuidelines?: string[];
}

/** Standard MCP config format used by Claude Desktop and other tools */
interface StandardMcpServerConfig {
	command: string;
	args?: string[];
	env?: Record<string, string>;
	background?: boolean;
	promptSnippet?: string;
	promptGuidelines?: string[];
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
			promptSnippet: serverConfig.promptSnippet,
			promptGuidelines: serverConfig.promptGuidelines,
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
		// Server-level steering: the config's snippet/guidelines ride on each of
		// the server's tools (the prompt builder collapses duplicate guidelines).
		promptSnippet: serverConfig.promptSnippet,
		promptGuidelines: serverConfig.promptGuidelines,
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

/**
 * Serializes MCP setup across session_start firings. A /reload re-entering
 * while a previous pass's connects are still in flight would race
 * connectMcpServer's terminate-then-replace against the earlier pass
 * registering the same server, so each setup waits for the previous one.
 */
let mcpSetupChain: Promise<void> = Promise.resolve();

export function setupMcpLoader(pi: ExtensionAPI): void {
	pi.on("session_start", (_event: SessionStartEvent, ctx: ExtensionContext) => {
		// A spawned subagent whose tool allowlist has no MCP tools is told by its
		// parent to skip server connection entirely (see SUBAGENT_SKIP_MCP_ENV).
		// Each connect is a ~15s-timeout handshake; doing it for a subagent that can
		// never call the tools is pure startup latency.
		if (subagentSkipMcp()) return;

		const run = mcpSetupChain.then(() => runMcpSetup(pi, ctx));
		mcpSetupChain = run.then(
			() => undefined,
			() => undefined,
		);
		return run;
	});
}

async function runMcpSetup(pi: ExtensionAPI, ctx: ExtensionContext): Promise<void> {
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
		for (const serverConfig of parseStandardMcpConfig({ mcpServers: entry.mcpServers }, `plugin:${entry.source}`)) {
			if (seenNames.has(serverConfig.name)) continue;
			seenNames.add(serverConfig.name);
			allServerConfigs.push(serverConfig);
		}
	}

	// Deferral (spec §2): inject MCP tool names only and materialize each schema
	// on demand via ResolveMcpTools. Default-on (the deferMcpSchemas setting) and
	// top-level only — a subagent that needs MCP has this env cleared, so it
	// eager-registers its allowlisted tools at dispatch (the dispatch ↔ schema
	// interaction) and they are immediately callable.
	resetMcpRegistrationState();
	const state = ensureMcpRegistrationState();

	// 3. Connect to all servers concurrently, then register (or defer) tools in
	// config order. Retain every config up front so a tool call can lazily
	// reconnect a dropped server even when its initial connect fails.
	for (const serverConfig of allServerConfigs) {
		mcpServerConfigs.set(serverConfig.name, serverConfig);
	}
	const outcomes = await connectAllInOrder(allServerConfigs, connectMcpServer);
	for (const { config: serverConfig, result } of outcomes) {
		if (result.status === "rejected") {
			ctx.ui.notify(`MCP: failed to connect "${serverConfig.name}": ${String(result.reason)}`, "error");
			continue;
		}
		const { tools } = result.value;
		registerServerTools(pi, state, serverConfig, tools);
		ctx.ui.notify(mcpConnectedMessage(serverConfig, tools.length, state.defer), "info");
	}

	// 4. In deferred mode, register the single resolver that materializes schemas on demand.
	if (state.defer && state.deferredCatalog.length > 0) {
		registerResolverTool(pi, state);
	}
}

/**
 * Registration state shared between the session-start setup pass and post-start
 * live activation (an InstallPlugin'd plugin's MCP servers). Holds the deferred
 * catalog so live-activated tools land in the same ResolveMcpTools surface the
 * setup pass built. Reset by each setup pass (startup and /reload rebuild the
 * tool registry from scratch).
 */
interface McpRegistrationState {
	/** Whether this session defers MCP tool schemas (captured once per pass). */
	defer: boolean;
	deferredCatalog: DeferredMcpToolEntry[];
	/** Raw definition + config per deferred tool so ResolveMcpTools can build the full ToolDefinition on request. */
	deferredByName: Map<string, { serverConfig: McpServerConfig; tool: McpToolDef }>;
	resolvedNames: Set<string>;
}

let mcpRegistrationState: McpRegistrationState | undefined;

function ensureMcpRegistrationState(): McpRegistrationState {
	mcpRegistrationState ??= {
		defer: deferMcpSchemas(),
		deferredCatalog: [],
		deferredByName: new Map(),
		resolvedNames: new Set(),
	};
	return mcpRegistrationState;
}

/** Drop registration state so the next pass starts fresh (each setup pass rebuilds the registry). */
export function resetMcpRegistrationState(): void {
	mcpRegistrationState = undefined;
}

function mcpConnectedMessage(serverConfig: McpServerConfig, toolCount: number, defer: boolean): string {
	const bgMode = serverConfig.background !== false ? "background" : "foreground";
	const suffix = defer ? ", schemas deferred" : "";
	return `MCP: connected "${serverConfig.name}" (${toolCount} tool${toolCount === 1 ? "" : "s"}, ${bgMode}${suffix})`;
}

/** Register (eager) or catalog (deferred) one connected server's tools. Returns the registered tool names. */
function registerServerTools(
	pi: ExtensionAPI,
	state: McpRegistrationState,
	serverConfig: McpServerConfig,
	tools: McpToolDef[],
): string[] {
	const names: string[] = [];
	for (const tool of tools) {
		const toolName = `mcp_${serverConfig.name}_${tool.name}`;
		names.push(toolName);
		if (state.defer) {
			if (state.deferredByName.has(toolName)) continue;
			state.deferredCatalog.push({ toolName, server: serverConfig.name, description: tool.description });
			state.deferredByName.set(toolName, { serverConfig, tool });
		} else {
			pi.registerTool(buildMcpToolDefinition(serverConfig, tool));
		}
	}
	return names;
}

/**
 * (Re-)register the ResolveMcpTools resolver from the current catalog.
 * registerTool replaces by name, so calling this again after live activation
 * refreshes the model-visible catalog embedded in the tool description.
 */
function registerResolverTool(pi: ExtensionAPI, state: McpRegistrationState): void {
	const resolveParams = Type.Object(
		{
			names: Type.Array(Type.String(), {
				description: "MCP tool names to make callable (e.g. 'mcp_github_create_pr' or 'create_pr').",
			}),
		},
		{ additionalProperties: false },
	);
	// Server-level steering in deferred mode: the snippet annotates the server's
	// catalog line and the guidelines ride on the resolver itself, so the model
	// is steered toward a server's tools before any schema is resolved.
	const serverSnippets = new Map<string, string>();
	const serverGuidelines: string[] = [];
	const seenServers = new Set<string>();
	for (const { serverConfig } of state.deferredByName.values()) {
		if (seenServers.has(serverConfig.name)) continue;
		seenServers.add(serverConfig.name);
		if (serverConfig.promptSnippet) serverSnippets.set(serverConfig.name, serverConfig.promptSnippet);
		if (serverConfig.promptGuidelines) serverGuidelines.push(...serverConfig.promptGuidelines);
	}
	pi.registerTool({
		name: RESOLVE_MCP_TOOLS_NAME,
		label: RESOLVE_MCP_TOOLS_NAME,
		description:
			"MCP tools are connected but their schemas are loaded on demand to keep context small. Call this with the tool name(s) you need to make them callable, then call the tool(s). Available MCP tools:\n" +
			formatDeferredCatalog(state.deferredCatalog, serverSnippets),
		promptSnippet: "Resolve deferred MCP tool schemas by name before calling them.",
		promptGuidelines: serverGuidelines.length > 0 ? serverGuidelines : undefined,
		parameters: resolveParams,
		async execute(_toolCallId: string, params: Static<typeof resolveParams>) {
			const matched = selectResolvable(state.deferredCatalog, params.names ?? []);
			const newlyResolved: string[] = [];
			for (const entry of matched) {
				if (state.resolvedNames.has(entry.toolName)) continue;
				const raw = state.deferredByName.get(entry.toolName);
				if (!raw) continue;
				pi.registerTool(buildMcpToolDefinition(raw.serverConfig, raw.tool));
				state.resolvedNames.add(entry.toolName);
				newlyResolved.push(entry.toolName);
			}
			const text = matched.length
				? `Resolved ${newlyResolved.length} MCP tool(s): ${matched.map((m) => m.toolName).join(", ")}. They are now callable.`
				: `No MCP tools matched: ${(params.names ?? []).join(", ") || "(none)"}. See ResolveMcpTools for the catalog.`;
			return { content: [{ type: "text" as const, text }], details: undefined };
		},
	} as ToolDefinition);
}

/** Outcome of live-activating MCP servers after session start (see {@link activateMcpServersLive}). */
export interface LiveMcpActivation {
	/** Full `mcp_<server>_<tool>` names now callable (eager) or resolvable via ResolveMcpTools (deferred). */
	registeredTools: string[];
	/** Whether the tools went into the deferred catalog rather than being registered eagerly. */
	deferred: boolean;
	/** Per-server connect failures. */
	errors: string[];
	/** Servers skipped because a same-named server is already active (first-wins, like startup). */
	skipped: string[];
}

/**
 * Connect and register MCP servers mid-session — the live half of model-driven
 * plugin install (InstallPlugin). Honors the session's deferral mode: deferred
 * tools are appended to the shared catalog and the ResolveMcpTools description
 * is refreshed; eager tools register directly. A later /reload converges to the
 * same set by rebuilding everything from disk.
 */
export async function activateMcpServersLive(
	pi: ExtensionAPI,
	mcpServers: Record<string, ExtensionMcpServerConfig>,
	notify: (message: string, level: "info" | "warning" | "error") => void,
): Promise<LiveMcpActivation> {
	installMcpExitCleanup();
	const state = ensureMcpRegistrationState();
	const activation: LiveMcpActivation = { registeredTools: [], deferred: state.defer, errors: [], skipped: [] };

	const configs: McpServerConfig[] = [];
	for (const serverConfig of parseStandardMcpConfig({ mcpServers }, "live-activation")) {
		if (mcpServerConfigs.has(serverConfig.name)) {
			activation.skipped.push(serverConfig.name);
			continue;
		}
		mcpServerConfigs.set(serverConfig.name, serverConfig);
		configs.push(serverConfig);
	}

	const outcomes = await connectAllInOrder(configs, connectMcpServer);
	for (const { config: serverConfig, result } of outcomes) {
		if (result.status === "rejected") {
			activation.errors.push(`${serverConfig.name}: ${String(result.reason)}`);
			notify(`MCP: failed to connect "${serverConfig.name}": ${String(result.reason)}`, "error");
			continue;
		}
		const { tools } = result.value;
		activation.registeredTools.push(...registerServerTools(pi, state, serverConfig, tools));
		notify(mcpConnectedMessage(serverConfig, tools.length, state.defer), "info");
	}

	if (state.defer && activation.registeredTools.length > 0) {
		registerResolverTool(pi, state);
	}
	return activation;
}
