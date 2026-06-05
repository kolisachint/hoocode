/**
 * hoo-core — HooCode built-in core extension
 *
 * A. Permission Gate    — prompts before bash/write/edit; checks modes.{mode}.auto_allow
 *                         from the merged (global + project) config; persists "always"
 *                         choices back to the global config
 * B. MCP Server Loader  — discovers ~/.hoocode/mcp-servers and ./.hoocode/mcp-servers JSON
 *                         configs, connects via JSON-RPC 2.0, registers server tools
 * C. Mode                — resolves active mode (ask/plan/build/debug), loads the mode's
 *                         system prompt, filters active tools, and exposes /mode, /plan,
 *                         and /approve commands
 *
 * Config merge order (lowest → highest priority):
 *   1. ~/.hoocode/hoo-config.json    (global defaults)
 *   2. ./.hoocode/hoo-config.json   (project overrides — scalars win; arrays union)
 */

import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { readdir } from "node:fs/promises";
import { homedir } from "node:os";
import { join, relative } from "node:path";
import { createInterface } from "node:readline";
import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { getHooCodeDir } from "../../config.js";
import type {
	AgentToolResult,
	AgentToolUpdateCallback,
	AskQuestion,
	BeforeAgentStartEvent,
	BeforeAgentStartEventResult,
	ExtensionAPI,
	ExtensionCommandContext,
	ExtensionContext,
	SessionStartEvent,
	ToolCallEvent,
	ToolCallEventResult,
} from "../../core/extensions/types.js";
import { isToolCallEventType } from "../../core/extensions/types.js";
import { summarizeArgs } from "../../core/messages.js";
import { taskStore } from "../../core/task-store.js";

// ============================================================================
// Fallback defaults for mode prompts
// ============================================================================

const MODE_DEFAULTS: Record<string, string> = {
	ask: `You are in ASK mode — read-only Q&A.
Answer questions about the codebase. Trace logic, compare approaches, explain patterns.
You may read any file but NEVER write, edit, or execute commands.
If asked to make changes, refuse and suggest switching to /mode build.
Cite specific file paths and line numbers in your answers.`,

	plan: `You are in PLAN mode — exploration and planning.
Explore the codebase thoroughly. Understand the current structure.
Draft a complete plan with sections: Goal, Files to modify, New files, Tests, Verification.
Write the plan to {{PLAN_PATH}}.
When the plan is complete, tell the user to run /approve to execute it.`,

	build: `You are in BUILD mode — careful implementation.
Read files before editing them. Show diffs before non-trivial changes.
Ask for confirmation before destructive operations (delete, reformat).
Run tests after every logical unit of work.
Prefer the smallest change that achieves the goal.
Follow existing code patterns and conventions.`,

	debug: `You are in DEBUG mode — root cause analysis.
Gather evidence: read files, check logs, reproduce the issue.
Trace the call path from entry to failure point.
State the root cause in one sentence.
Describe the fix precisely but do NOT apply it.
To fix, switch to /mode build.`,
};

// ============================================================================
// Shared paths
// ============================================================================

const HOOCODE_DIR = getHooCodeDir();
const GLOBAL_CONFIG_PATH = join(HOOCODE_DIR, "hoo-config.json");

/**
 * Per-session plan file path. Keying on sessionId lets concurrent or resumed
 * plan sessions keep distinct plans instead of clobbering each other.
 */
function getPlanPath(cwd: string, sessionId: string): string {
	return join(cwd, ".hoocode", "plans", `${sessionId}.md`);
}

/** Legacy single-file plan location, retained as a read-only fallback for /approve. */
function getLegacyPlanPath(cwd: string): string {
	return join(cwd, ".hoocode", "plan.md");
}

// ============================================================================
// Config types
// ============================================================================

interface ModeConfig {
	/** Tool names that bypass the permission gate in this mode */
	auto_allow?: string[];
	/** Tool names available in this mode (if set, only these tools are active) */
	enabled_tools?: string[];
	/** Tool names explicitly blocked in this mode regardless of enabled_tools */
	denied_tools?: string[];
	/** Allowed write paths in this mode (glob patterns, only applies if write/edit is enabled) */
	allowed_write_paths?: string[];
	/** Regex patterns for allowed bash commands. If set, a command must match at least one to execute. */
	allowed_bash_commands?: string[];
	/** Regex patterns for denied bash commands. A command matching any pattern is blocked. */
	denied_bash_commands?: string[];
}

export interface HooConfig {
	/** Manually-pinned active mode (overrides default "build") */
	active_mode?: string;
	/** Per-mode configuration keyed by mode name */
	modes?: Record<string, ModeConfig>;
	/** Extra directories to search for `{name}/system.md` mode files (after project + user). */
	mode_paths?: string[];
}

// ============================================================================
// Config I/O and merging
// ============================================================================

function readConfig(): HooConfig {
	try {
		return JSON.parse(readFileSync(GLOBAL_CONFIG_PATH, "utf8")) as HooConfig;
	} catch {
		return {};
	}
}

function writeConfig(config: HooConfig): void {
	if (!existsSync(HOOCODE_DIR)) mkdirSync(HOOCODE_DIR, { recursive: true });
	writeFileSync(GLOBAL_CONFIG_PATH, `${JSON.stringify(config, null, 2)}\n`, "utf8");
}

/**
 * Deep-merges a project-local config on top of the global config.
 *
 * Merge rules:
 * - active_mode: project wins if set
 * - modes[x].auto_allow: union of global + project arrays
 * - modes[x].allowed_write_paths: union of global + project arrays
 * - modes[x].enabled_tools: project wins if set, else falls back to global
 * - mode_paths: project list is prepended so project paths are searched first
 */
export function mergeConfigs(global: HooConfig, project: HooConfig): HooConfig {
	const merged: HooConfig = { ...global };

	if (project.active_mode !== undefined) merged.active_mode = project.active_mode;

	if (project.modes) {
		merged.modes = { ...(global.modes ?? {}) };
		for (const [mode, projectCfg] of Object.entries(project.modes)) {
			const globalCfg = global.modes?.[mode] ?? {};
			merged.modes[mode] = {
				...globalCfg,
				...projectCfg,
				// Union both auto_allow lists so project can extend, not just replace
				auto_allow: Array.from(new Set([...(globalCfg.auto_allow ?? []), ...(projectCfg.auto_allow ?? [])])),
				// Union allowed_write_paths so project can extend
				allowed_write_paths: Array.from(
					new Set([...(globalCfg.allowed_write_paths ?? []), ...(projectCfg.allowed_write_paths ?? [])]),
				),
				// enabled_tools: project wins if set, else falls back to global
				enabled_tools: projectCfg.enabled_tools ?? globalCfg.enabled_tools,
				// denied_tools: union so project can add more denied tools on top of global
				denied_tools: Array.from(new Set([...(globalCfg.denied_tools ?? []), ...(projectCfg.denied_tools ?? [])])),
				// allowed_bash_commands: project wins if set, else falls back to global
				allowed_bash_commands: projectCfg.allowed_bash_commands ?? globalCfg.allowed_bash_commands,
				// denied_bash_commands: union so project can add more denied patterns on top of global
				denied_bash_commands: Array.from(
					new Set([...(globalCfg.denied_bash_commands ?? []), ...(projectCfg.denied_bash_commands ?? [])]),
				),
			};
		}
	}

	if (project.mode_paths || global.mode_paths) {
		// Project paths first so they're searched before global paths
		merged.mode_paths = dedupePaths([...(project.mode_paths ?? []), ...(global.mode_paths ?? [])]);
	}

	return merged;
}

function dedupePaths(paths: string[]): string[] {
	const seen = new Set<string>();
	const out: string[] = [];
	for (const p of paths) {
		if (!seen.has(p)) {
			seen.add(p);
			out.push(p);
		}
	}
	return out;
}

function mergeSearchPaths(...sources: (string[] | undefined)[]): string[] {
	const merged: string[] = [];
	for (const source of sources) {
		if (!source) continue;
		merged.push(...source);
	}
	return dedupePaths(merged);
}

/**
 * Reads the global config and optionally overlays the project-local config at
 * `./.hoocode/hoo-config.json`. Project values win on all scalar fields; arrays are
 * unioned (see mergeConfigs for full rules).
 */
export function readMergedConfig(cwd: string): HooConfig {
	const global = readConfig();
	const projectPath = join(cwd, ".hoocode", "hoo-config.json");
	if (!existsSync(projectPath)) return global;
	try {
		const project = JSON.parse(readFileSync(projectPath, "utf8")) as HooConfig;
		return mergeConfigs(global, project);
	} catch {
		return global;
	}
}

// ============================================================================
// A. Permission Gate
// ============================================================================

const GATED_TOOLS = new Set(["bash", "write", "edit"]);

/**
 * Checks if a file path matches any of the allowed patterns.
 * Supports glob patterns with * and exact paths.
 */
function matchesAllowedPath(filePath: string, allowedPatterns: string[]): boolean {
	if (allowedPatterns.length === 0) return true;
	for (const pattern of allowedPatterns) {
		// Exact match
		if (pattern === filePath) return true;
		// Glob pattern matching for *
		if (pattern.includes("*")) {
			const regex = new RegExp(`^${pattern.replace(/\*/g, ".*")}$`);
			if (regex.test(filePath)) return true;
		}
	}
	return false;
}

/**
 * Tests a bash command string against a regex pattern string.
 * Returns false (no match) if the pattern is an invalid regex.
 */
function matchesBashPattern(pattern: string, command: string): boolean {
	try {
		return new RegExp(pattern).test(command);
	} catch {
		return false;
	}
}

function describeTool(event: ToolCallEvent): string {
	if (isToolCallEventType("bash", event)) {
		return `$ ${event.input.command.replace(/\s+/g, " ").slice(0, 100)}`;
	}
	if (isToolCallEventType("edit", event)) {
		const p = (event.input as { file_path?: string }).file_path ?? "(unknown)";
		return `edit ${p}`;
	}
	if (isToolCallEventType("write", event)) {
		const p = (event.input as { file_path?: string }).file_path ?? "(unknown)";
		return `write ${p}`;
	}
	return event.toolName;
}

export function setupPermissionGate(pi: ExtensionAPI): void {
	pi.on("tool_call", async (event: ToolCallEvent, ctx: ExtensionContext): Promise<ToolCallEventResult | undefined> => {
		// Use the merged config so project-local entries are respected
		const config = readMergedConfig(ctx.cwd);
		const mode = config.active_mode ?? "build";
		const modeCfg = config.modes?.[mode];

		// ── Hard enforcement (always applies, regardless of UI) ───────────────────

		// Explicitly denied tools are blocked unconditionally
		if (modeCfg?.denied_tools?.includes(event.toolName)) {
			return {
				block: true,
				reason: `Tool "${event.toolName}" is denied in mode "${mode}".`,
			};
		}

		// enabled_tools acts as a strict allowlist: only listed tools may execute
		if (
			modeCfg?.enabled_tools &&
			modeCfg.enabled_tools.length > 0 &&
			!modeCfg.enabled_tools.includes(event.toolName)
		) {
			return {
				block: true,
				reason:
					`Tool "${event.toolName}" is not enabled in mode "${mode}" ` +
					`(enabled: ${modeCfg.enabled_tools.join(", ")}).`,
			};
		}

		// Bash command-level filtering
		if (isToolCallEventType("bash", event)) {
			const command = (event.input as { command?: string }).command ?? "";

			// denied_bash_commands: block if any pattern matches
			if (modeCfg?.denied_bash_commands?.length) {
				for (const pattern of modeCfg.denied_bash_commands) {
					if (matchesBashPattern(pattern, command)) {
						return {
							block: true,
							reason: `Bash command matches a denied pattern in mode "${mode}": ${pattern}`,
						};
					}
				}
			}

			// allowed_bash_commands: block unless at least one pattern matches
			if (modeCfg?.allowed_bash_commands?.length) {
				const permitted = modeCfg.allowed_bash_commands.some((p) => matchesBashPattern(p, command));
				if (!permitted) {
					return {
						block: true,
						reason:
							`Bash command is not permitted in mode "${mode}". ` +
							`Allowed patterns: ${modeCfg.allowed_bash_commands.join(", ")}`,
					};
				}
			}
		}

		// ── UI-based permission prompting (interactive sessions only) ─────────────

		if (!GATED_TOOLS.has(event.toolName) || !ctx.hasUI) return;

		const autoAllow = modeCfg?.auto_allow ?? [];

		// Check allowed_write_paths for write/edit operations
		if ((event.toolName === "write" || event.toolName === "edit") && modeCfg?.allowed_write_paths) {
			const filePath = (event.input as { file_path?: string }).file_path ?? "";
			if (!matchesAllowedPath(filePath, modeCfg.allowed_write_paths)) {
				return {
					block: true,
					reason:
						`Mode "${mode}" only allows writes to: ${modeCfg.allowed_write_paths.join(", ")}. ` +
						`Attempted to ${event.toolName}: ${filePath}. ` +
						`Switch to "/mode build" to modify source files.`,
				};
			}
		}

		if (autoAllow.includes(event.toolName)) return;

		const choice = await ctx.ui.select(`Allow: ${describeTool(event)}`, [
			"Yes (once)",
			"No (block)",
			"Always (add to auto-allow for this mode)",
		]);

		if (!choice || choice.startsWith("No")) {
			return { block: true, reason: "Denied by permission gate" };
		}

		if (choice.startsWith("Always")) {
			// Write "always" choices to the global config only
			const latest = readConfig();
			const currentMode = latest.active_mode ?? "build";
			latest.modes ??= {};
			latest.modes[currentMode] ??= {};
			latest.modes[currentMode].auto_allow = Array.from(
				new Set([...(latest.modes[currentMode].auto_allow ?? []), event.toolName]),
			);
			writeConfig(latest);
			ctx.ui.notify(`"${event.toolName}" added to auto-allow for mode "${currentMode}"`, "info");
		}
	});
}

// ============================================================================
// B. MCP Server Loader
// ============================================================================

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

export function setupMcpLoader(pi: ExtensionAPI): void {
	pi.on("session_start", async (_event: SessionStartEvent, ctx: ExtensionContext) => {
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

		// 3. Connect to all servers and register tools
		for (const serverConfig of allServerConfigs) {
			// Retain the config so a tool call can lazily reconnect a dropped server.
			mcpServerConfigs.set(serverConfig.name, serverConfig);
			try {
				const { tools } = await connectMcpServer(serverConfig);

				for (const tool of tools) {
					const toolName = `mcp_${serverConfig.name}_${tool.name}`;
					const schema = buildMcpSchema(tool);
					const capturedServer = serverConfig.name;
					const capturedTool = tool.name;
					// MCP tools default to background mode since they are external processes with potential high latency
					const isBackground = serverConfig.background !== false;

					pi.registerTool({
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
							// Foreground tools skip this (their result is awaited inline).
							const task = isBackground
								? taskStore.create(`${capturedServer} › ${capturedTool}`, { source: "mcp" })
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
										{
											type: "text",
											text: `MCP server "${capturedServer}" is not connected (reconnect attempt failed)`,
										},
									],
									details: undefined,
								};
							}

							try {
								const abortPromise = new Promise<never>((_, reject) => {
									signal.addEventListener("abort", () => reject(new Error("Aborted")));
								});

								const result = await Promise.race([
									activeConn.rpc("tools/call", {
										name: capturedTool,
										arguments: params,
									}),
									abortPromise,
								]);

								if (task) taskStore.update(task.id, { status: "done" });
								return {
									content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
									details: undefined,
								};
							} catch (error) {
								if (task) taskStore.update(task.id, { status: "failed" });
								throw error;
							}
						},
					});
				}

				const bgMode = serverConfig.background !== false ? "background" : "foreground";
				ctx.ui.notify(
					`MCP: connected "${serverConfig.name}" (${tools.length} tool${tools.length === 1 ? "" : "s"}, ${bgMode})`,
					"info",
				);
			} catch (err) {
				ctx.ui.notify(`MCP: failed to connect "${serverConfig.name}": ${String(err)}`, "error");
			}
		}
	});
}

// ============================================================================
// C. Mode System
// ============================================================================

const DEFAULT_MODE = "build";

function tryReadFile(path: string): string | undefined {
	if (!existsSync(path)) return undefined;
	try {
		const text = readFileSync(path, "utf8").trim();
		return text || undefined;
	} catch {
		return undefined;
	}
}

/**
 * Walks search dirs in precedence order and returns the first existing
 * `modes/{name}/system.md` content. Order: project → user → externalDirs.
 */
function resolveModeFile(name: string, cwd: string, externalDirs: string[]): string | undefined {
	const candidates: string[] = [
		join(cwd, ".hoocode", "modes", name, "system.md"),
		join(HOOCODE_DIR, "modes", name, "system.md"),
		...externalDirs.map((dir) => join(dir, name, "system.md")),
	];
	for (const candidate of candidates) {
		const content = tryReadFile(candidate);
		if (content !== undefined) return content;
	}
	return undefined;
}

/**
 * Returns the system prompt for the active mode.
 *
 * Search order (first hit wins):
 *   - `./.hoocode/modes/{mode}/system.md`
 *   - `~/.hoocode/modes/{mode}/system.md`
 *   - each of `externalDirs` in declared order (config + CLI + extension contributions)
 *   - built-in MODE_DEFAULTS for the four known modes
 */
export function buildSystemPrompt(mode: string, cwd: string, options?: { modePaths?: string[] }): string | undefined {
	const modePaths = options?.modePaths ?? [];
	return resolveModeFile(mode, cwd, modePaths) ?? MODE_DEFAULTS[mode];
}

// ============================================================================
// Plan file: section parsing and step-by-step execution message
// ============================================================================

export interface PlanSections {
	goal?: string;
	filesToModify?: string;
	newFiles?: string;
	tests?: string;
	verification?: string;
	/** Original full text, used as fallback if no sections parsed */
	raw: string;
}

/**
 * Parses `.hoocode/plan.md` into named sections.
 *
 * Recognises both ATX headings (`## Goal`) and bold labels (`**Goal**`).
 * Section names matched (case-insensitive): Goal, Files to modify, New files,
 * Tests, Verification.
 */
export function parsePlanSections(planContent: string): PlanSections {
	const result: PlanSections = { raw: planContent };

	// Match `## Heading text` or `**Heading text**` followed by content until
	// the next heading of the same style.
	const sectionPattern =
		/^(?:#{1,3}\s+(.+?)|(?:\*\*(.+?)\*\*))\s*\n([\s\S]*?)(?=(?:^#{1,3}\s+|\*\*[^*\n]+\*\*\s*\n)|$)/gm;

	for (const match of planContent.matchAll(sectionPattern)) {
		const heading = (match[1] ?? match[2] ?? "").toLowerCase().trim();
		const content = match[3].trim();
		if (!content) continue;

		if (/^goal/.test(heading)) {
			result.goal = content;
		} else if (/files?\s+to\s+modif|^modif/.test(heading)) {
			result.filesToModify = content;
		} else if (/new\s+files?/.test(heading)) {
			result.newFiles = content;
		} else if (/^tests?/.test(heading)) {
			result.tests = content;
		} else if (/^verif/.test(heading)) {
			result.verification = content;
		}
	}

	return result;
}

/**
 * Builds the user message sent to the agent when `/approve` is run.
 *
 * If the plan has recognisable sections, each is presented as a numbered step
 * so the agent works through them sequentially. Otherwise the raw plan is used.
 *
 * Execution order:
 *   1. Modify existing files
 *   2. Create new files
 *   3. Update / add tests
 *   4. Run verification commands
 */
export function buildApproveMessage(sections: PlanSections): string {
	const steps: string[] = [];

	if (sections.goal) {
		steps.push(`**Goal:** ${sections.goal}`);
	}
	if (sections.filesToModify) {
		steps.push(`**Step 1 — Modify existing files:**\n${sections.filesToModify}`);
	}
	if (sections.newFiles) {
		steps.push(`**Step 2 — Create new files:**\n${sections.newFiles}`);
	}
	if (sections.tests) {
		steps.push(`**Step 3 — Update tests:**\n${sections.tests}`);
	}
	if (sections.verification) {
		steps.push(`**Step 4 — Verify:**\n${sections.verification}`);
	}

	if (steps.length === 0) {
		return `Execute the following plan:\n\n${sections.raw}`;
	}

	return `Execute this plan step by step. Complete each step fully before moving to the next.\n\n${steps.join("\n\n")}`;
}

// ============================================================================
// C. setupMode
// ============================================================================

export function setupMode(pi: ExtensionAPI): void {
	let cachedMode = DEFAULT_MODE;
	let cachedSystemPrompt: string | undefined;
	let cachedPlanPath: string | undefined;

	// ── session_start ─────────────────────────────────────────────────────────
	// Config resolution order:
	//   1. Read global config  (~/.hoocode/hoo-config.json)
	//   2. Read project config (./.hoocode/hoo-config.json) if present
	//   3. Merge — project scalars win; arrays are unioned
	//   4. Re-resolve active_mode from the merged result

	pi.on("session_start", (_event: SessionStartEvent, ctx: ExtensionContext) => {
		// Steps 1–3: merge global + project configs
		const config = readMergedConfig(ctx.cwd);

		// Step 4: resolve mode from the merged config
		cachedMode = config.active_mode ?? DEFAULT_MODE;
		// External search dirs come from two channels:
		//  - HooConfig.mode_paths (config-declared)
		//  - pi.addModeSearchPath (CLI flags + extension contributions)
		const modePaths = mergeSearchPaths(config.mode_paths, pi.getModeSearchPaths());
		const rawSystemPrompt = buildSystemPrompt(cachedMode, ctx.cwd, { modePaths });

		// Per-session plan path so concurrent sessions don't overwrite each other.
		// The `{{PLAN_PATH}}` token in plan-mode templates is substituted here.
		cachedPlanPath = getPlanPath(ctx.cwd, ctx.sessionManager.getSessionId());
		const relPlanPath = relative(ctx.cwd, cachedPlanPath) || cachedPlanPath;
		cachedSystemPrompt = rawSystemPrompt?.replace(/\{\{PLAN_PATH\}\}/g, relPlanPath);

		// Update footer with active mode
		if (ctx.hasUI) {
			ctx.ui.setMode(cachedMode);
		}

		// Apply tool filter from mode enabled_tools
		const modeCfg = config.modes?.[cachedMode];
		if (modeCfg?.enabled_tools && modeCfg.enabled_tools.length > 0) {
			pi.setActiveTools(modeCfg.enabled_tools);
		}
	});

	// ── before_agent_start ────────────────────────────────────────────────────

	pi.on("before_agent_start", (event: BeforeAgentStartEvent): BeforeAgentStartEventResult | undefined => {
		if (!cachedSystemPrompt) return;
		return {
			systemPrompt: `${event.systemPrompt}\n\n<!-- hoo-core: mode=${cachedMode} -->\n${cachedSystemPrompt}`,
		};
	});

	// ── /mode command ─────────────────────────────────────────────────────────

	const KNOWN_MODES = ["ask", "plan", "build", "debug"];

	pi.registerCommand("mode", {
		description: "Switch active mode. Usage: /mode <ask|plan|build|debug>",
		getArgumentCompletions: (prefix: string) =>
			KNOWN_MODES.filter((m) => m.startsWith(prefix)).map((m) => ({ value: m, label: m })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const name = args.trim();
			if (!name) {
				ctx.ui.notify(`Active mode: ${cachedMode}`, "info");
				return;
			}
			const config = readConfig();
			config.active_mode = name === DEFAULT_MODE ? undefined : name;
			writeConfig(config);
			ctx.ui.notify(`Mode set to "${name}" — reloading…`, "info");
			await ctx.reload();
		},
	});

	// ── /plan command (shorthand for /mode plan) ──────────────────────────────

	pi.registerCommand("plan", {
		description: "Switch to plan mode. Shorthand for /mode plan.",
		getArgumentCompletions: () => [],
		handler: async (_args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const config = readConfig();
			config.active_mode = "plan";
			writeConfig(config);
			ctx.ui.notify(`Mode set to "plan" — reloading…`, "info");
			await ctx.reload();
		},
	});

	// ── /approve command ──────────────────────────────────────────────────────
	// Reads .hoocode/plan.md, parses it into named sections (Goal, Files to
	// modify, New files, Tests, Verification), switches to build mode, then
	// injects a step-by-step execution message into the new session.

	pi.registerCommand("approve", {
		description: "Approve the current plan and switch to build mode to execute it.",
		getArgumentCompletions: () => [],
		handler: async (_args: string, ctx: ExtensionCommandContext): Promise<void> => {
			if (cachedMode !== "plan") {
				ctx.ui.notify(`/approve is only available in plan mode (current mode: "${cachedMode}")`, "warning");
				return;
			}

			// Prefer the per-session plan file, fall back to the legacy single file.
			const sessionPlanPath = cachedPlanPath ?? getPlanPath(ctx.cwd, ctx.sessionManager.getSessionId());
			const candidatePaths = [sessionPlanPath, getLegacyPlanPath(ctx.cwd)];
			let approveMessage: string | undefined;

			for (const planPath of candidatePaths) {
				if (!existsSync(planPath)) continue;
				try {
					const raw = readFileSync(planPath, "utf8").trim();
					if (raw) {
						const sections = parsePlanSections(raw);
						approveMessage = buildApproveMessage(sections);
						break;
					}
				} catch {
					ctx.ui.notify(`Could not read ${relative(ctx.cwd, planPath) || planPath}`, "error");
					return;
				}
			}

			// Switch global config to build mode
			const config = readConfig();
			config.active_mode = "build";
			writeConfig(config);

			if (approveMessage) {
				// Open a new build-mode session and deliver the parsed plan as the
				// first user message so the agent starts executing immediately
				await ctx.newSession({
					withSession: async (replacedCtx) => {
						await replacedCtx.sendUserMessage(approveMessage!, { deliverAs: "followUp" });
					},
				});
			} else {
				const relPlan = relative(ctx.cwd, sessionPlanPath) || sessionPlanPath;
				ctx.ui.notify(`Switched to build mode. No ${relPlan} found — describe what to build.`, "info");
				await ctx.reload();
			}
		},
	});

	// ── /cost command ─────────────────────────────────────────────────────────
	// Walks every assistant message in the current session and sums tokens + cost,
	// then prints a session total followed by a per-model breakdown.
	// Per-tool attribution is intentionally not shown — tokens aren't tracked
	// per-tool, and any heuristic would be misleading.

	pi.registerCommand("cost", {
		description: "Show session token and cost totals, broken down by model.",
		getArgumentCompletions: () => [],
		handler: async (_args: string, ctx: ExtensionCommandContext): Promise<void> => {
			type Totals = { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number };
			const empty = (): Totals => ({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 });
			const total = empty();
			const perModel = new Map<string, Totals>();
			let assistantTurns = 0;

			for (const entry of ctx.sessionManager.getEntries()) {
				if (entry.type !== "message" || entry.message.role !== "assistant") continue;
				const u = entry.message.usage;
				if (!u) continue;
				assistantTurns++;
				total.input += u.input;
				total.output += u.output;
				total.cacheRead += u.cacheRead;
				total.cacheWrite += u.cacheWrite;
				total.cost += u.cost.total;

				const key = `${entry.message.provider}/${entry.message.model}`;
				const t = perModel.get(key) ?? empty();
				t.input += u.input;
				t.output += u.output;
				t.cacheRead += u.cacheRead;
				t.cacheWrite += u.cacheWrite;
				t.cost += u.cost.total;
				perModel.set(key, t);
			}

			if (assistantTurns === 0) {
				ctx.ui.notify("No assistant turns yet — nothing to cost.", "info");
				return;
			}

			const fmt = (n: number) => n.toLocaleString();
			const fmtCost = (n: number) => `$${n.toFixed(4)}`;
			const lines: string[] = [];
			lines.push(`Session totals (${assistantTurns} assistant turn${assistantTurns === 1 ? "" : "s"})`);
			lines.push(`  Input         ${fmt(total.input)}`);
			lines.push(`  Output        ${fmt(total.output)}`);
			lines.push(`  Cache read    ${fmt(total.cacheRead)}`);
			lines.push(`  Cache write   ${fmt(total.cacheWrite)}`);
			lines.push(`  Cost          ${fmtCost(total.cost)}`);

			if (perModel.size > 1) {
				lines.push("");
				lines.push("By model:");
				const sorted = [...perModel.entries()].sort((a, b) => b[1].cost - a[1].cost);
				for (const [key, t] of sorted) {
					lines.push(`  ${key}: ${fmt(t.input)} in / ${fmt(t.output)} out  ${fmtCost(t.cost)}`);
				}
			}

			ctx.ui.notify(lines.join("\n"), "info");
		},
	});
}

// ============================================================================
// Scaffold commands — /new-skill, /new-agent, and /new-command
// ============================================================================

/** Validates a resource name: lowercase a-z, 0-9, hyphens, no leading/trailing/double hyphens. */
function validateResourceName(name: string): string | null {
	if (!name) return "name is required";
	if (!/^[a-z0-9-]+$/.test(name)) return "name must be lowercase a-z, 0-9, and hyphens only";
	if (name.startsWith("-") || name.endsWith("-")) return "name must not start or end with a hyphen";
	if (name.includes("--")) return "name must not contain consecutive hyphens";
	return null;
}

function setupScaffold(pi: ExtensionAPI): void {
	// ── /new-skill <name> ─────────────────────────────────────────────────────
	// Creates .hoocode/skills/<name>/SKILL.md with a valid Agent Skills frontmatter
	// template so the file is ready to edit and will be picked up on next reload.

	pi.registerCommand("new-skill", {
		description: "Scaffold a new skill. Usage: /new-skill <name>",
		getArgumentCompletions: () => [],
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const name = args.trim();
			const error = validateResourceName(name);
			if (error) {
				ctx.ui.notify(`/new-skill: ${error}. Usage: /new-skill <name>`, "warning");
				return;
			}

			const skillDir = join(ctx.cwd, ".hoocode", "skills", name);
			const skillFile = join(skillDir, "SKILL.md");

			if (existsSync(skillFile)) {
				ctx.ui.notify(`/new-skill: ${skillFile} already exists`, "warning");
				return;
			}

			mkdirSync(skillDir, { recursive: true });
			writeFileSync(
				skillFile,
				[
					"---",
					`name: ${name}`,
					"description: |",
					"  TODO: describe when to use this skill — one clear sentence per bullet.",
					"  The model reads this to decide whether to load the skill.",
					"allowed-tools: read, bash",
					"---",
					"",
					`# ${name}`,
					"",
					"TODO: write the skill instructions here.",
					"",
					"When relative paths appear below, they are resolved from this file's directory.",
					"",
				].join("\n"),
				"utf8",
			);

			ctx.ui.notify(
				`Skill created: ${join(".hoocode", "skills", name, "SKILL.md")}\nEdit the file, then run /reload to activate it.`,
				"info",
			);
		},
	});

	// ── /new-agent <name> ─────────────────────────────────────────────────────
	// Creates .hoocode/agents/<name>.md following the Claude Code subagent standard
	// (name, description, tools comma-string, model alias, optional background).

	pi.registerCommand("new-agent", {
		description: "Scaffold a new subagent. Usage: /new-agent <name>",
		getArgumentCompletions: () => [],
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const name = args.trim();
			const error = validateResourceName(name);
			if (error) {
				ctx.ui.notify(`/new-agent: ${error}. Usage: /new-agent <name>`, "warning");
				return;
			}

			const agentsDir = join(ctx.cwd, ".hoocode", "agents");
			const agentFile = join(agentsDir, `${name}.md`);

			if (existsSync(agentFile)) {
				ctx.ui.notify(`/new-agent: ${agentFile} already exists`, "warning");
				return;
			}

			mkdirSync(agentsDir, { recursive: true });
			writeFileSync(
				agentFile,
				[
					"---",
					`name: ${name}`,
					"description: |",
					"  Use this subagent ONLY when:",
					"  - TODO: describe the task(s) to delegate here",
					"",
					"  DO NOT use for:",
					"  - TODO: describe what this agent should NOT handle",
					"tools: read, bash",
					"model: sonnet",
					"---",
					`You are a ${name} subagent running inside hoocode.`,
					"You run in an isolated context and cannot see the parent conversation.",
					"",
					"TODO: write the system prompt here.",
					"",
					"Your final message must contain ONLY your answer — it is the only output",
					"the caller receives. Do not include intermediate reasoning or tool logs.",
					"",
				].join("\n"),
				"utf8",
			);

			ctx.ui.notify(
				`Agent created: ${join(".hoocode", "agents", `${name}.md`)}\nEdit the file, then run /reload to activate it.`,
				"info",
			);
		},
	});

	// ── /new-command <name> ───────────────────────────────────────────────────
	// Creates .hoocode/commands/<name>.md with a slash-command prompt-template
	// frontmatter (name, description, argument-hint) so it is ready to edit and
	// picked up on next reload. Body supports $1, $@, $ARGUMENTS placeholders.

	pi.registerCommand("new-command", {
		description: "Scaffold a new slash command. Usage: /new-command <name>",
		getArgumentCompletions: () => [],
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const name = args.trim();
			const error = validateResourceName(name);
			if (error) {
				ctx.ui.notify(`/new-command: ${error}. Usage: /new-command <name>`, "warning");
				return;
			}

			const commandsDir = join(ctx.cwd, ".hoocode", "commands");
			const commandFile = join(commandsDir, `${name}.md`);

			if (existsSync(commandFile)) {
				ctx.ui.notify(`/new-command: ${commandFile} already exists`, "warning");
				return;
			}

			mkdirSync(commandsDir, { recursive: true });
			writeFileSync(
				commandFile,
				[
					"---",
					`name: ${name}`,
					"description: |",
					`  TODO: describe what /${name} does and when to use it.`,
					`  Usage: /${name} <args>`,
					"argument-hint: <args>",
					"---",
					`Run the /${name} command with arguments: **$ARGUMENTS**.`,
					"",
					"TODO: write the instructions here. Placeholders you can use:",
					"- $1, $2, ... for positional arguments",
					"- $@ or $ARGUMENTS for all arguments",
					`- $${"{"}@:N} / $${"{"}@:N:L} for bash-style slices`,
					"",
				].join("\n"),
				"utf8",
			);

			ctx.ui.notify(
				`Command created: ${join(".hoocode", "commands", `${name}.md`)}\nEdit the file, then run /reload to activate it.`,
				"info",
			);
		},
	});
}

// ============================================================================
// D. Options pane — ask_options tool
// ============================================================================

// The model calls this tool when it needs the user to make a decision before
// continuing. Each question is shown in an inline options pane where the user
// moves with up/down, advances with right, and may type a custom answer.
const askOptionsSchema = Type.Object({
	questions: Type.Array(
		Type.Object({
			question: Type.String({ description: "The question to ask the user." }),
			detail: Type.Optional(Type.String({ description: "Optional clarifying sub-text shown under the question." })),
			options: Type.Array(
				Type.Object({
					label: Type.String({ description: "The option text; returned verbatim when chosen." }),
					description: Type.Optional(
						Type.String({ description: "Optional short description shown next to the option." }),
					),
					recommended: Type.Optional(
						Type.Boolean({
							description: "When true, the option is marked '(recommended)' to help the user choose.",
						}),
					),
				}),
				{ description: "The options the user can choose from." },
			),
			allow_custom: Type.Optional(
				Type.Boolean({
					description: "When true, the user can type a free-form answer instead of choosing an option.",
				}),
			),
		}),
		{ description: "One or more decisions to ask the user, in order." },
	),
});

export function setupAskOptions(pi: ExtensionAPI): void {
	// Capture the latest context so the tool can reach the interactive UI.
	let activeCtx: ExtensionContext | undefined;
	pi.on("session_start", (_event: SessionStartEvent, ctx: ExtensionContext) => {
		activeCtx = ctx;
	});

	pi.registerTool({
		name: "ask_options",
		label: "Ask the user",
		description:
			"Ask the user to make one or more decisions before continuing. Each question is presented " +
			"in an interactive options pane where the user selects an option (or types a custom answer). " +
			"Use this when you genuinely need input to proceed and cannot reasonably decide yourself. " +
			"Returns the user's answer for each question; if the user skips, no answers are returned.",
		parameters: askOptionsSchema,
		async execute(
			_toolCallId: string,
			params: Static<typeof askOptionsSchema>,
			signal: AbortSignal,
			_onUpdate: AgentToolUpdateCallback,
		): Promise<AgentToolResult<undefined>> {
			if (!activeCtx || !activeCtx.hasUI) {
				return {
					content: [
						{
							type: "text",
							text: "Cannot ask the user: no interactive UI is available in this session. Proceed using your best judgement.",
						},
					],
					details: undefined,
				};
			}

			if (!params.questions.length) {
				return {
					content: [{ type: "text", text: "No questions were provided." }],
					details: undefined,
				};
			}

			const questions: AskQuestion[] = params.questions.map((q) => ({
				question: q.question,
				detail: q.detail,
				options: q.options.map((o) => ({ label: o.label, description: o.description, recommended: o.recommended })),
				allowCustom: q.allow_custom,
			}));

			const answers = await activeCtx.ui.askOptions(questions, { signal });

			if (!answers) {
				return {
					content: [
						{
							type: "text",
							text: "The user skipped the question(s) without answering. Ask how they would like to proceed.",
						},
					],
					details: undefined,
				};
			}

			const text = questions.map((q, i) => `${q.question}\n  \u2192 ${answers[i] ?? "(no answer)"}`).join("\n\n");
			return {
				content: [{ type: "text", text }],
				details: undefined,
			};
		},
	});
}

// ============================================================================
// Extension entry point
// ============================================================================

function hooCore(pi: ExtensionAPI): void {
	setupPermissionGate(pi);
	setupMcpLoader(pi);
	setupMode(pi);
	setupScaffold(pi);
	setupAskOptions(pi);
}

hooCore.displayName = "hoo-core";
export default hooCore;
