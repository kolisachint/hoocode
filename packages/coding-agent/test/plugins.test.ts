import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createEventBus } from "../src/core/event-bus.js";
import { clearExtensionMcpServers, getExtensionMcpServers } from "../src/core/extension-mcp-servers.js";
import {
	createExtensionRuntime,
	defaultPluginDirs,
	discoverAndLoadExtensions,
	loadPlugins,
} from "../src/core/extensions/loader.js";
import {
	emitForPlatforms,
	getFormat,
	getFormatByPlatform,
	PLUGIN_FORMATS,
	parsePluginWithFormats,
} from "../src/core/extensions/plugins/formats/index.js";
import type { PluginDraft } from "../src/core/extensions/plugins/formats/types.js";
import { buildPluginFactory, discoverPlugins, parsePluginDir } from "../src/core/extensions/plugins/index.js";

/** Write a set of emitted files (paths relative to `root`) to disk. */
function writeEmitted(root: string, files: { path: string; content: string }[]): void {
	for (const f of files) {
		const abs = path.join(root, f.path);
		fs.mkdirSync(path.dirname(abs), { recursive: true });
		fs.writeFileSync(abs, f.content);
	}
}

function writeJson(file: string, data: unknown): void {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, JSON.stringify(data, null, 2));
}

describe("plugin manifests", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-plugin-test-"));
	});

	afterEach(() => {
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it("parses a Claude-format plugin and resolves capability dirs", () => {
		const root = path.join(tempDir, "cc-plugin");
		writeJson(path.join(root, ".claude-plugin", "plugin.json"), {
			name: "cc-plugin",
			version: "1.2.3",
			description: "claude format",
		});
		fs.mkdirSync(path.join(root, "skills"));
		fs.mkdirSync(path.join(root, "commands"));
		writeJson(path.join(root, "hooks", "hooks.json"), {
			hooks: { PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: "true" }] }] },
		});

		const plugin = parsePluginDir(root);
		expect(plugin).not.toBeNull();
		expect(plugin?.format).toBe("claude");
		expect(plugin?.id).toBe("cc-plugin");
		expect(plugin?.version).toBe("1.2.3");
		expect(plugin?.skillsDir).toBe(path.join(root, "skills"));
		expect(plugin?.commandsDir).toBe(path.join(root, "commands"));
		expect(plugin?.agentsDir).toBeUndefined();
		expect(plugin?.hooks?.PreToolUse).toHaveLength(1);
	});

	it("prefers the native format when both manifests are present, and reads providers", () => {
		const root = path.join(tempDir, "dual-plugin");
		writeJson(path.join(root, ".claude-plugin", "plugin.json"), { name: "from-claude" });
		writeJson(path.join(root, ".agents-plugin", "plugin.json"), {
			name: "from-native",
			providers: [{ name: "myprov", config: { baseUrl: "https://x", api: "anthropic-messages" } }],
		});

		const plugin = parsePluginDir(root);
		expect(plugin?.format).toBe("agents");
		expect(plugin?.id).toBe("from-native");
		expect(plugin?.providers).toHaveLength(1);
		expect(plugin?.providers?.[0].name).toBe("myprov");
	});

	it("ignores providers declared in a Claude-format manifest", () => {
		const root = path.join(tempDir, "cc-with-providers");
		writeJson(path.join(root, ".claude-plugin", "plugin.json"), {
			name: "cc",
			providers: [{ name: "nope", config: {} }],
		});
		expect(parsePluginDir(root)?.providers).toBeUndefined();
	});

	it("returns null for a directory with no manifest", () => {
		const root = path.join(tempDir, "not-a-plugin");
		fs.mkdirSync(root);
		expect(parsePluginDir(root)).toBeNull();
	});

	it("discovers plugins and de-duplicates by id (first wins)", () => {
		writeJson(path.join(tempDir, "a", "p1", ".agents-plugin", "plugin.json"), { name: "shared" });
		writeJson(path.join(tempDir, "b", "p1", ".agents-plugin", "plugin.json"), { name: "shared" });
		const found = discoverPlugins([path.join(tempDir, "a"), path.join(tempDir, "b")]);
		expect(found).toHaveLength(1);
		expect(found[0].id).toBe("shared");
	});
});

describe("plugin format registry", () => {
	it("registers agents, claude and copilot in precedence order", () => {
		expect(PLUGIN_FORMATS.map((f) => f.id)).toEqual(["agents", "claude", "copilot"]);
		expect(getFormat("copilot")?.platform).toBe("github");
		expect(getFormatByPlatform("github")?.id).toBe("copilot");
	});

	it("prefers the highest-precedence format when several coexist", () => {
		const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-fmt-"));
		try {
			const root = path.join(tmp, "multi");
			writeJson(path.join(root, ".github", "copilot-plugin.json"), { name: "from-copilot" });
			writeJson(path.join(root, ".claude-plugin", "plugin.json"), { name: "from-claude" });
			writeJson(path.join(root, ".agents-plugin", "plugin.json"), { name: "from-agents" });
			expect(parsePluginWithFormats(root)?.format).toBe("agents");
		} finally {
			fs.rmSync(tmp, { recursive: true, force: true });
		}
	});

	it("records supportPlatform for every format present (winner first)", () => {
		const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-fmt-sp-"));
		try {
			const root = path.join(tmp, "dual");
			writeJson(path.join(root, ".claude-plugin", "plugin.json"), { name: "dual" });
			writeJson(path.join(root, ".github", "copilot-plugin.json"), { name: "dual" });
			const plugin = parsePluginDir(root);
			expect(plugin?.format).toBe("claude"); // precedence winner
			expect(plugin?.supportPlatform).toEqual(["claude", "github"]); // both recorded
		} finally {
			fs.rmSync(tmp, { recursive: true, force: true });
		}
	});

	it("single-format plugins report just their own platform", () => {
		const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-fmt-solo-"));
		try {
			const root = path.join(tmp, "solo");
			writeJson(path.join(root, ".github", "copilot-plugin.json"), { name: "solo" });
			expect(parsePluginDir(root)?.supportPlatform).toEqual(["github"]);
		} finally {
			fs.rmSync(tmp, { recursive: true, force: true });
		}
	});

	it("emitForPlatforms renders only the requested platforms", () => {
		const draft: PluginDraft = { id: "sel", commands: [{ name: "go", body: "Go." }] };

		const claudeOnly = emitForPlatforms(draft, ["claude"]).map((f) => f.path);
		expect(claudeOnly.some((p) => p.includes(".claude-plugin"))).toBe(true);
		expect(claudeOnly).not.toContain("plugin.json");

		const both = emitForPlatforms(draft, ["claude", "github"]).map((f) => f.path);
		expect(both.some((p) => p.includes(".claude-plugin"))).toBe(true);
		expect(both).toContain("plugin.json");

		// Defaults to the draft's own supportPlatform when no platforms passed.
		// Copilot's capability tree mirrors the Claude layout, so only the root
		// plugin.json manifest is Copilot-specific.
		const fromDraft = emitForPlatforms({ ...draft, supportPlatform: ["github"] }).map((f) => f.path);
		expect(fromDraft).toContain("plugin.json");
		expect(fromDraft.some((p) => p.includes(".claude-plugin") || p.includes(".agents-plugin"))).toBe(false);
	});
});

describe("copilot (.github) plugin format", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-copilot-"));
	});

	afterEach(() => {
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it("parses a real-world Copilot plugin (.github/plugin/plugin.json + Claude-mirror layout)", () => {
		// Mirrors microsoft/work-iq plugins/workiq: manifest under .github/plugin/,
		// a `skills` path override in the manifest, and a top-level .mcp.json.
		const root = path.join(tempDir, "workiq-like");
		writeJson(path.join(root, ".github", "plugin", "plugin.json"), {
			name: "workiq-like",
			version: "2.0.0",
			description: "real-world copilot layout",
			skills: "./skills/",
		});
		fs.mkdirSync(path.join(root, "skills", "workiq"), { recursive: true });
		fs.writeFileSync(path.join(root, "skills", "workiq", "SKILL.md"), "---\nname: workiq\n---\n\nUse WorkIQ.\n");
		writeJson(path.join(root, ".mcp.json"), { mcpServers: { svc: { command: "svc-bin" } } });

		const plugin = parsePluginDir(root);
		expect(plugin?.format).toBe("copilot");
		expect(plugin?.id).toBe("workiq-like");
		expect(plugin?.supportPlatform).toEqual(["github"]);
		expect(plugin?.skillsDir).toBe(path.join(root, "skills"));
		expect(plugin?.mcpServers).toMatchObject({ svc: { command: "svc-bin" } });
	});

	it("parses the canonical Copilot CLI layout (root plugin.json + root hooks.json)", () => {
		// Per the Copilot CLI plugin reference: plugin.json at the plugin root,
		// hooks config at root hooks.json, agents as agents/<name>.agent.md.
		const root = path.join(tempDir, "cli-canonical");
		writeJson(path.join(root, "plugin.json"), {
			name: "cli-canonical",
			version: "1.0.0",
			author: { name: "Octo" },
		});
		fs.mkdirSync(path.join(root, "agents"), { recursive: true });
		fs.writeFileSync(path.join(root, "agents", "helper.agent.md"), "---\nname: helper\n---\n\nHelp.\n");
		writeJson(path.join(root, "hooks.json"), {
			hooks: { PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: "true" }] }] },
		});

		const plugin = parsePluginDir(root);
		expect(plugin?.format).toBe("copilot");
		expect(plugin?.id).toBe("cli-canonical");
		expect(plugin?.author).toBe("Octo");
		expect(plugin?.agentsDir).toBe(path.join(root, "agents"));
		expect(plugin?.hooks?.PreToolUse).toHaveLength(1);
	});

	it("probes manifest locations in the Copilot CLI's documented order", () => {
		// .plugin/plugin.json beats root plugin.json beats .github/plugin/plugin.json.
		const root = path.join(tempDir, "probe");
		writeJson(path.join(root, ".github", "plugin", "plugin.json"), { name: "from-github-dir" });
		expect(parsePluginDir(root)?.id).toBe("from-github-dir");
		writeJson(path.join(root, "plugin.json"), { name: "from-root" });
		expect(parsePluginDir(root)?.id).toBe("from-root");
		writeJson(path.join(root, ".plugin", "plugin.json"), { name: "from-dot-plugin" });
		expect(parsePluginDir(root)?.id).toBe("from-dot-plugin");
	});

	it("honors manifest dir overrides in plugin.json-style formats", () => {
		const root = path.join(tempDir, "override");
		writeJson(path.join(root, ".claude-plugin", "plugin.json"), { name: "override", skills: "./my-skills" });
		fs.mkdirSync(path.join(root, "my-skills", "s"), { recursive: true });
		fs.writeFileSync(path.join(root, "my-skills", "s", "SKILL.md"), "---\nname: s\n---\n\nBody.\n");
		expect(parsePluginDir(root)?.skillsDir).toBe(path.join(root, "my-skills"));
	});

	it("parses a legacy-authored Copilot plugin (copilot-plugin.json + prompts/chatmodes)", () => {
		const root = path.join(tempDir, "cop");
		writeJson(path.join(root, ".github", "copilot-plugin.json"), {
			name: "cop",
			version: "2.0.0",
			description: "copilot format",
		});
		fs.mkdirSync(path.join(root, ".github", "prompts"), { recursive: true });
		fs.mkdirSync(path.join(root, ".github", "chatmodes"), { recursive: true });
		// VS Code / Copilot use `{ servers }`.
		writeJson(path.join(root, ".github", "mcp.json"), { servers: { demo: { command: "demo-server" } } });
		writeJson(path.join(root, ".github", "hooks", "hooks.json"), {
			hooks: { PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: "true" }] }] },
		});

		const plugin = parsePluginDir(root);
		expect(plugin?.format).toBe("copilot");
		expect(plugin?.id).toBe("cop");
		expect(plugin?.version).toBe("2.0.0");
		expect(plugin?.commandsDir).toBe(path.join(root, ".github", "prompts"));
		expect(plugin?.agentsDir).toBe(path.join(root, ".github", "chatmodes"));
		expect(plugin?.skillsDir).toBeUndefined();
		expect(plugin?.mcpServers).toMatchObject({ demo: { command: "demo-server" } });
		expect(plugin?.hooks?.PreToolUse).toHaveLength(1);
	});

	it("round-trips an emitted Copilot plugin back through the reader", () => {
		const draft: PluginDraft = {
			id: "authored",
			version: "0.1.0",
			description: "authored copilot plugin",
			commands: [{ name: "greet", description: "say hi", body: "Say hello to the user." }],
			agents: [{ name: "scout", description: "read-only scout", tools: "read, grep", body: "You explore." }],
			mcpServers: [{ name: "svc", command: "svc-bin", args: ["--port", "1"] }],
		};
		const root = path.join(tempDir, "rt");
		writeEmitted(root, getFormat("copilot")!.emit(draft));

		// Canonical Copilot CLI convention: manifest at root plugin.json, capability
		// tree mirrors the Claude layout.
		expect(fs.existsSync(path.join(root, "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(root, "commands", "greet.md"))).toBe(true);
		expect(fs.existsSync(path.join(root, "agents", "scout.md"))).toBe(true);

		const parsed = parsePluginDir(root);
		expect(parsed?.format).toBe("copilot");
		expect(parsed?.id).toBe("authored");
		expect(parsed?.commandsDir).toBe(path.join(root, "commands"));
		expect(parsed?.agentsDir).toBe(path.join(root, "agents"));
		expect(parsed?.mcpServers).toMatchObject({ svc: { command: "svc-bin", args: ["--port", "1"] } });
	});

	it("emits Claude and Copilot layouts from the same draft", () => {
		const draft: PluginDraft = {
			id: "dual",
			skills: [{ name: "helper", description: "helps", body: "Help." }],
			agents: [{ name: "agent1", tools: "read", body: "You act." }],
		};
		const claudeFiles = getFormat("claude")!
			.emit(draft)
			.map((f) => f.path);
		const copilotFiles = getFormat("copilot")!
			.emit(draft)
			.map((f) => f.path);

		expect(claudeFiles).toContain(path.join(".claude-plugin", "plugin.json"));
		expect(claudeFiles).toContain(path.join("skills", "helper", "SKILL.md"));
		expect(claudeFiles).toContain(path.join("agents", "agent1.md"));

		expect(copilotFiles).toContain("plugin.json");
		// Same capability tree as Claude — only the manifest location differs.
		expect(copilotFiles).toContain(path.join("skills", "helper", "SKILL.md"));
		expect(copilotFiles).toContain(path.join("agents", "agent1.md"));
	});
});

describe("plugin directory precedence (.agents first)", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-plugin-dirs-"));
	});

	afterEach(() => {
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it("lists project .agents/plugins ahead of .hoocode/plugins, project before global", () => {
		const cwd = path.join(tempDir, "proj");
		const agentDir = path.join(tempDir, "home", ".hoocode");
		const dirs = defaultPluginDirs(cwd, agentDir);
		expect(dirs).toEqual([
			path.join(cwd, ".agents", "plugins"),
			path.join(cwd, ".hoocode", "plugins"),
			path.join(tempDir, "home", ".agents", "plugins"),
			path.join(agentDir, "plugins"),
		]);
	});

	it("a plugin under .agents/plugins wins over a same-id one under .hoocode/plugins", () => {
		const cwd = path.join(tempDir, "proj");
		const agentDir = path.join(tempDir, "home", ".hoocode");
		writeJson(path.join(cwd, ".agents", "plugins", "dup", ".agents-plugin", "plugin.json"), {
			name: "dup",
			version: "agents",
		});
		writeJson(path.join(cwd, ".hoocode", "plugins", "dup", ".claude-plugin", "plugin.json"), {
			name: "dup",
			version: "hoocode",
		});
		const found = discoverPlugins(defaultPluginDirs(cwd, agentDir));
		expect(found).toHaveLength(1);
		expect(found[0].version).toBe("agents");
		expect(found[0].format).toBe("agents");
	});
});

describe("plugin factory wiring", () => {
	it("registers resources, providers and hooks via the ExtensionAPI", () => {
		const plugin = {
			id: "p",
			root: "/tmp/p",
			manifestPath: "/tmp/p/.agents-plugin/plugin.json",
			format: "agents" as const,
			skillsDir: "/tmp/p/skills",
			commandsDir: "/tmp/p/commands",
			agentsDir: "/tmp/p/agents",
			supportPlatform: ["agents" as const],
			providers: [{ name: "prov", config: { baseUrl: "https://x" } }],
			hooks: { PreToolUse: [{ matcher: "*", hooks: [{ command: "true" }] }] },
		};

		const events: string[] = [];
		const providers: string[] = [];
		const discovered: Record<string, unknown>[] = [];
		const pi = {
			on: (event: string, handler: () => Record<string, unknown>) => {
				events.push(event);
				if (event === "resources_discover") discovered.push(handler());
			},
			registerProvider: (name: string) => providers.push(name),
		} as unknown as Parameters<ReturnType<typeof buildPluginFactory>>[0];

		const factory = buildPluginFactory(plugin);
		factory(pi);

		expect(factory.displayName).toBe("plugin:p");
		expect(events).toContain("resources_discover");
		expect(events).toContain("tool_call"); // PreToolUse hook
		expect(providers).toEqual(["prov"]);
		// Commands → slash-command surface, agents → subagent surface.
		expect(discovered[0]).toMatchObject({
			skillPaths: ["/tmp/p/skills"],
			slashCommandPaths: ["/tmp/p/commands"],
			agentPaths: ["/tmp/p/agents"],
		});
	});
});

describe("plugin discovery integration", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-plugin-int-"));
	});

	afterEach(() => {
		fs.rmSync(tempDir, { recursive: true, force: true });
	});

	it("loads a plugin from the global plugins/ folder as an extension", async () => {
		writeJson(path.join(tempDir, "plugins", "hello", ".claude-plugin", "plugin.json"), {
			name: "hello",
			version: "0.1.0",
		});

		// agentDir = tempDir → global plugins dir is tempDir/plugins
		const result = await discoverAndLoadExtensions([], tempDir, tempDir);

		expect(result.errors).toHaveLength(0);
		const plugin = result.extensions.find((e) => e.displayName === "plugin:hello");
		expect(plugin).toBeDefined();
	});
});

describe("plugin MCP servers", () => {
	let tempDir: string;

	beforeEach(() => {
		tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-plugin-mcp-"));
		clearExtensionMcpServers();
	});

	afterEach(() => {
		fs.rmSync(tempDir, { recursive: true, force: true });
		clearExtensionMcpServers();
	});

	it("registers plugin mcpServers into the registry with root substitution", async () => {
		const root = path.join(tempDir, "plugins", "mcp-plugin");
		writeJson(path.join(root, ".agents-plugin", "plugin.json"), {
			name: "mcp-plugin",
			mcpServers: {
				// biome-ignore lint/suspicious/noTemplateCurlyInString: literal placeholders substituted at load time
				demo: { command: "${CLAUDE_PLUGIN_ROOT}/bin/server", args: ["--root", "${AGENTS_PLUGIN_ROOT}"] },
			},
		});

		const result = await loadPlugins(
			[path.join(tempDir, "plugins")],
			tempDir,
			createEventBus(),
			createExtensionRuntime(),
		);

		expect(result.errors).toHaveLength(0);
		const entries = getExtensionMcpServers();
		expect(entries).toHaveLength(1);
		expect(entries[0].source).toBe("mcp-plugin");
		expect(entries[0].mcpServers.demo.command).toBe(path.join(root, "bin", "server"));
		expect(entries[0].mcpServers.demo.args).toEqual(["--root", root]);
	});

	it("clears prior registrations on each load (no accumulation across reloads)", async () => {
		writeJson(path.join(tempDir, "plugins", "p", ".agents-plugin", "plugin.json"), {
			name: "p",
			mcpServers: { demo: { command: "server" } },
		});

		await loadPlugins([path.join(tempDir, "plugins")], tempDir, createEventBus(), createExtensionRuntime());
		await loadPlugins([path.join(tempDir, "plugins")], tempDir, createEventBus(), createExtensionRuntime());

		expect(getExtensionMcpServers()).toHaveLength(1);
	});

	it("reads mcpServers from a .mcp.json file when not inline", async () => {
		const root = path.join(tempDir, "plugins", "filey");
		writeJson(path.join(root, ".claude-plugin", "plugin.json"), { name: "filey" });
		writeJson(path.join(root, ".mcp.json"), { mcpServers: { fs: { command: "fs-server" } } });

		const plugin = parsePluginDir(root);
		expect(plugin?.mcpServers).toMatchObject({ fs: { command: "fs-server" } });
	});

	it("resolves a Claude-form string `mcpServers` path into the registry", async () => {
		// Claude Code manifests may set `"mcpServers": "./.mcp.json"` (a path, not an
		// inline object). The path must be resolved and its servers registered.
		const root = path.join(tempDir, "plugins", "atlassian");
		writeJson(path.join(root, ".claude-plugin", "plugin.json"), {
			name: "atlassian",
			mcpServers: "./.mcp.json",
		});
		writeJson(path.join(root, ".mcp.json"), { mcpServers: { atlassian: { command: "mcp-atlassian" } } });

		// Parses correctly via parsePluginDir.
		const plugin = parsePluginDir(root);
		expect(plugin?.mcpServers).toMatchObject({ atlassian: { command: "mcp-atlassian" } });

		// And flows through loadPlugins into the extension MCP registry.
		const result = await loadPlugins(
			[path.join(tempDir, "plugins")],
			tempDir,
			createEventBus(),
			createExtensionRuntime(),
		);
		expect(result.errors).toHaveLength(0);
		const entries = getExtensionMcpServers();
		expect(entries).toHaveLength(1);
		expect(entries[0].source).toBe("atlassian");
		expect(entries[0].mcpServers.atlassian).toMatchObject({ command: "mcp-atlassian" });
	});

	it("resolves a string `mcpServers` path whose file uses the `servers` key", async () => {
		// The referenced server file may use the Code/Copilot `{ servers }` key
		// rather than `{ mcpServers }`; readMcpFile accepts both.
		const root = path.join(tempDir, "plugins", "atlassian");
		writeJson(path.join(root, ".claude-plugin", "plugin.json"), {
			name: "atlassian",
			mcpServers: "./.mcp.json",
		});
		writeJson(path.join(root, ".mcp.json"), { servers: { atlassian: { command: "mcp-atlassian" } } });

		const plugin = parsePluginDir(root);
		expect(plugin?.mcpServers).toMatchObject({ atlassian: { command: "mcp-atlassian" } });
	});

	it("registers remote http/sse servers instead of skipping them", async () => {
		writeJson(path.join(tempDir, "plugins", "remote", ".claude-plugin", "plugin.json"), { name: "remote" });
		writeJson(path.join(tempDir, "plugins", "remote", ".mcp.json"), {
			mcpServers: {
				workiq: { type: "http", url: "https://mcp.example.com/mcp", headers: { authorization: "Bearer t" } },
				legacy: { type: "sse", url: "https://sse.example.com/mcp" },
				typeless: { url: "https://plain.example.com/mcp" },
			},
		});

		const result = await loadPlugins(
			[path.join(tempDir, "plugins")],
			tempDir,
			createEventBus(),
			createExtensionRuntime(),
		);

		expect(result.errors).toHaveLength(0);
		const entries = getExtensionMcpServers();
		expect(entries).toHaveLength(1);
		expect(entries[0].mcpServers.workiq).toEqual({
			type: "http",
			url: "https://mcp.example.com/mcp",
			headers: { authorization: "Bearer t" },
			background: undefined,
		});
		expect(entries[0].mcpServers.legacy).toMatchObject({ type: "sse", url: "https://sse.example.com/mcp" });
		// No explicit type defaults to Streamable HTTP.
		expect(entries[0].mcpServers.typeless).toMatchObject({ type: "http", url: "https://plain.example.com/mcp" });
	});

	it("still skips entries with neither command nor url", async () => {
		writeJson(path.join(tempDir, "plugins", "broken", ".agents-plugin", "plugin.json"), {
			name: "broken",
			mcpServers: { bad: { args: ["--nope"] } },
		});

		await loadPlugins([path.join(tempDir, "plugins")], tempDir, createEventBus(), createExtensionRuntime());

		const entries = getExtensionMcpServers();
		expect(entries.flatMap((e) => Object.keys(e.mcpServers))).toEqual([]);
	});
});
