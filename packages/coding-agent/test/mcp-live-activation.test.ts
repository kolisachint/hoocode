/**
 * Live MCP activation (the same-turn half of model-driven plugin install).
 *
 * Uses the fake stdio MCP server fixture to exercise the real connect path:
 * activateMcpServersLive in deferred and eager modes, first-wins skip for
 * duplicate server names, connect-failure isolation, and the plugin-level
 * activator built from a fixture plugin directory.
 */

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { clearExtensionMcpServers } from "../src/core/extension-mcp-servers.js";
import type { ExtensionAPI, ToolDefinition } from "../src/core/extensions/types.js";
import { mcpStats } from "../src/core/mcp-stats.js";
import { formatLiveActivationSummary } from "../src/core/plugin-activation.js";
import { DEFER_MCP_SCHEMAS_ENV } from "../src/core/subagent-depth.js";
import { activateMcpServersLive, resetMcpRegistrationState } from "../src/extensions/core/mcp-loader.js";
import { buildLivePluginActivator } from "../src/extensions/core/plugin-activator.js";

const FAKE_SERVER = join(__dirname, "fixtures", "fake-mcp-server.mjs");

function fakePi(): { pi: ExtensionAPI; tools: Map<string, ToolDefinition> } {
	const tools = new Map<string, ToolDefinition>();
	const pi = {
		registerTool: (tool: ToolDefinition) => {
			tools.set(tool.name, tool);
		},
	} as unknown as ExtensionAPI;
	return { pi, tools };
}

const noopNotify = () => {};

async function callTool(tool: ToolDefinition, params: unknown): Promise<string> {
	const result = await tool.execute("test-call", params as never, new AbortController().signal, () => {}, {} as never);
	const first = result.content[0];
	return first && first.type === "text" ? first.text : "";
}

// Unique server names per test: server configs are retained module-globally for
// lazy reconnect, so a reused name would be skipped as already active.
let serverSeq = 0;
function nextServerName(): string {
	return `fake${++serverSeq}_${process.pid}`;
}

describe("activateMcpServersLive", () => {
	const savedDefer = process.env[DEFER_MCP_SCHEMAS_ENV];

	beforeEach(() => {
		resetMcpRegistrationState();
	});

	afterEach(() => {
		if (savedDefer === undefined) delete process.env[DEFER_MCP_SCHEMAS_ENV];
		else process.env[DEFER_MCP_SCHEMAS_ENV] = savedDefer;
	});

	it("deferred mode: catalogs tools, registers the resolver, and resolving makes the tool callable", async () => {
		process.env[DEFER_MCP_SCHEMAS_ENV] = "1";
		const { pi, tools } = fakePi();
		const server = nextServerName();

		const activation = await activateMcpServersLive(
			pi,
			{ [server]: { command: "node", args: [FAKE_SERVER] } },
			noopNotify,
		);

		const toolName = `mcp_${server}_echo`;
		expect(activation.deferred).toBe(true);
		expect(activation.registeredTools).toEqual([toolName]);
		expect(activation.errors).toEqual([]);

		// Only the resolver is registered; the schema stays deferred.
		expect([...tools.keys()]).toEqual(["ResolveMcpTools"]);
		expect(tools.get("ResolveMcpTools")?.description).toContain(toolName);

		// Resolving materializes the real tool, and it round-trips to the server.
		const resolveText = await callTool(tools.get("ResolveMcpTools")!, { names: [toolName] });
		expect(resolveText).toContain(toolName);
		const echoTool = tools.get(toolName);
		expect(echoTool).toBeDefined();
		const echoText = await callTool(echoTool!, { message: "hello" });
		expect(echoText).toContain("echo: hello");
	});

	it("eager mode: registers the tool directly with no resolver", async () => {
		delete process.env[DEFER_MCP_SCHEMAS_ENV];
		const { pi, tools } = fakePi();
		const server = nextServerName();

		const activation = await activateMcpServersLive(
			pi,
			{ [server]: { command: "node", args: [FAKE_SERVER] } },
			noopNotify,
		);

		const toolName = `mcp_${server}_echo`;
		expect(activation.deferred).toBe(false);
		expect(activation.registeredTools).toEqual([toolName]);
		expect(tools.has("ResolveMcpTools")).toBe(false);
		const echoText = await callTool(tools.get(toolName)!, { message: "direct" });
		expect(echoText).toContain("echo: direct");
	});

	it("skips a server whose name is already active (first-wins) instead of clobbering it", async () => {
		process.env[DEFER_MCP_SCHEMAS_ENV] = "1";
		const { pi } = fakePi();
		const server = nextServerName();

		const first = await activateMcpServersLive(
			pi,
			{ [server]: { command: "node", args: [FAKE_SERVER] } },
			noopNotify,
		);
		expect(first.registeredTools).toHaveLength(1);

		const second = await activateMcpServersLive(
			pi,
			{ [server]: { command: "node", args: [FAKE_SERVER] } },
			noopNotify,
		);
		expect(second.skipped).toEqual([server]);
		expect(second.registeredTools).toEqual([]);
	});

	it("isolates a failing server: the healthy one still activates", async () => {
		process.env[DEFER_MCP_SCHEMAS_ENV] = "1";
		const { pi, tools } = fakePi();
		const good = nextServerName();
		const bad = nextServerName();

		const activation = await activateMcpServersLive(
			pi,
			{
				[bad]: { command: "node", args: ["-e", "process.exit(1)"] },
				[good]: { command: "node", args: [FAKE_SERVER] },
			},
			noopNotify,
		);

		expect(activation.errors).toHaveLength(1);
		expect(activation.errors[0]).toContain(bad);
		expect(activation.registeredTools).toEqual([`mcp_${good}_echo`]);
		expect(tools.get("ResolveMcpTools")?.description).toContain(`mcp_${good}_echo`);
	});

	it("eager mode: applies server-level promptSnippet and promptGuidelines to each tool", async () => {
		delete process.env[DEFER_MCP_SCHEMAS_ENV];
		const { pi, tools } = fakePi();
		const server = nextServerName();

		await activateMcpServersLive(
			pi,
			{
				[server]: {
					command: "node",
					args: [FAKE_SERVER],
					promptSnippet: "Prefer this server for echo work",
					promptGuidelines: ["Always echo politely"],
				},
			},
			noopNotify,
		);

		const tool = tools.get(`mcp_${server}_echo`);
		expect(tool?.promptSnippet).toBe("Prefer this server for echo work");
		expect(tool?.promptGuidelines).toEqual(["Always echo politely"]);
	});

	it("deferred mode: surfaces server snippet in the catalog and guidelines on the resolver", async () => {
		process.env[DEFER_MCP_SCHEMAS_ENV] = "1";
		const { pi, tools } = fakePi();
		const server = nextServerName();

		await activateMcpServersLive(
			pi,
			{
				[server]: {
					command: "node",
					args: [FAKE_SERVER],
					promptSnippet: "Prefer this server for echo work",
					promptGuidelines: ["Always echo politely"],
				},
			},
			noopNotify,
		);

		const resolver = tools.get("ResolveMcpTools");
		expect(resolver?.description).toContain(`${server}: Prefer this server for echo work`);
		expect(resolver?.promptGuidelines).toEqual(["Always echo politely"]);
	});

	it("flags a chronically failing server in the deferred catalog and records call outcomes", async () => {
		process.env[DEFER_MCP_SCHEMAS_ENV] = "1";
		const { pi, tools } = fakePi();
		const server = nextServerName();
		// Seed a bad observed record for this server before it connects.
		for (let i = 0; i < 10; i++) mcpStats.recordCall(server, i < 6 ? "transport_failure" : "ok");

		await activateMcpServersLive(pi, { [server]: { command: "node", args: [FAKE_SERVER] } }, noopNotify);
		expect(tools.get("ResolveMcpTools")?.description).toContain(`${server}: [unreliable:`);

		// A successful call through the registered tool lands in the same stats.
		await callTool(tools.get("ResolveMcpTools")!, { names: [`mcp_${server}_echo`] });
		await callTool(tools.get(`mcp_${server}_echo`)!, { message: "x" });
		expect(mcpStats.get(server)?.calls).toBe(11);
		expect(mcpStats.get(server)?.callTransportFailures).toBe(6);
	});

	it("appends to an existing deferred catalog and refreshes the resolver description", async () => {
		process.env[DEFER_MCP_SCHEMAS_ENV] = "1";
		const { pi, tools } = fakePi();
		const serverA = nextServerName();
		const serverB = nextServerName();

		await activateMcpServersLive(pi, { [serverA]: { command: "node", args: [FAKE_SERVER, "alpha"] } }, noopNotify);
		expect(tools.get("ResolveMcpTools")?.description).toContain(`mcp_${serverA}_alpha`);

		await activateMcpServersLive(pi, { [serverB]: { command: "node", args: [FAKE_SERVER, "beta"] } }, noopNotify);
		const description = tools.get("ResolveMcpTools")?.description ?? "";
		expect(description).toContain(`mcp_${serverA}_alpha`);
		expect(description).toContain(`mcp_${serverB}_beta`);
	});
});

describe("buildLivePluginActivator", () => {
	const savedDefer = process.env[DEFER_MCP_SCHEMAS_ENV];
	let pluginRoot: string;

	beforeEach(() => {
		resetMcpRegistrationState();
		clearExtensionMcpServers();
		pluginRoot = mkdtempSync(join(tmpdir(), "hoo-live-activation-"));
	});

	afterEach(() => {
		rmSync(pluginRoot, { recursive: true, force: true });
		clearExtensionMcpServers();
		if (savedDefer === undefined) delete process.env[DEFER_MCP_SCHEMAS_ENV];
		else process.env[DEFER_MCP_SCHEMAS_ENV] = savedDefer;
	});

	it("activates a fixture plugin's MCP server and reports reload-bound capabilities", async () => {
		process.env[DEFER_MCP_SCHEMAS_ENV] = "1";
		const server = nextServerName();
		mkdirSync(join(pluginRoot, ".agents-plugin"), { recursive: true });
		mkdirSync(join(pluginRoot, "skills"), { recursive: true });
		mkdirSync(join(pluginRoot, "commands"), { recursive: true });
		writeFileSync(
			join(pluginRoot, ".agents-plugin", "plugin.json"),
			JSON.stringify({
				name: "fixture-plugin",
				mcpServers: { [server]: { command: "node", args: [FAKE_SERVER] } },
			}),
		);

		const { pi, tools } = fakePi();
		const activator = buildLivePluginActivator(pi, noopNotify);
		const result = await activator(pluginRoot);

		expect("error" in result).toBe(false);
		if ("error" in result) return;
		expect(result.pluginId).toBe("fixture-plugin");
		expect(result.mcpTools).toEqual([`mcp_${server}_echo`]);
		expect(result.mcpDeferred).toBe(true);
		expect(result.needsReload).toEqual(["skills", "commands"]);
		expect(tools.get("ResolveMcpTools")?.description).toContain(`mcp_${server}_echo`);
	});

	it("returns an error for a directory without a plugin manifest", async () => {
		const { pi } = fakePi();
		const activator = buildLivePluginActivator(pi, noopNotify);
		const result = await activator(pluginRoot);
		expect("error" in result && result.error).toContain("no recognizable plugin manifest");
	});
});

describe("formatLiveActivationSummary", () => {
	it("formats the error case", () => {
		expect(formatLiveActivationSummary({ error: "boom" })).toContain("Live activation failed: boom");
	});

	it("formats deferred tools with the ResolveMcpTools pointer", () => {
		const text = formatLiveActivationSummary({
			pluginId: "p",
			mcpTools: ["mcp_x_a"],
			mcpDeferred: true,
			mcpErrors: [],
			mcpSkipped: [],
			needsReload: ["skills", "hooks"],
		});
		expect(text).toContain("ResolveMcpTools");
		expect(text).toContain("mcp_x_a");
		expect(text).toContain("skills, hooks");
		expect(text).toContain("next reload");
	});

	it("formats eager tools as directly callable", () => {
		const text = formatLiveActivationSummary({
			pluginId: "p",
			mcpTools: ["mcp_x_a"],
			mcpDeferred: false,
			mcpErrors: [],
			mcpSkipped: [],
			needsReload: [],
		});
		expect(text).toContain("now callable");
		expect(text).not.toContain("ResolveMcpTools");
	});

	it("reports when nothing could be live-activated", () => {
		const text = formatLiveActivationSummary({
			pluginId: "p",
			mcpTools: [],
			mcpDeferred: false,
			mcpErrors: [],
			mcpSkipped: [],
			needsReload: [],
		});
		expect(text).toContain("next reload");
	});
});
