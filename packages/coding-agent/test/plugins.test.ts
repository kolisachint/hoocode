import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { discoverAndLoadExtensions } from "../src/core/extensions/loader.js";
import { buildPluginFactory, discoverPlugins, parsePluginDir } from "../src/core/extensions/plugins/index.js";

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

describe("plugin factory wiring", () => {
	it("registers resources, providers, mode paths and hooks via the ExtensionAPI", () => {
		const plugin = {
			id: "p",
			root: "/tmp/p",
			manifestPath: "/tmp/p/.agents-plugin/plugin.json",
			format: "agents" as const,
			skillsDir: "/tmp/p/skills",
			commandsDir: "/tmp/p/commands",
			agentsDir: "/tmp/p/agents",
			providers: [{ name: "prov", config: { baseUrl: "https://x" } }],
			hooks: { PreToolUse: [{ matcher: "*", hooks: [{ command: "true" }] }] },
		};

		const events: string[] = [];
		const providers: string[] = [];
		const modePaths: string[] = [];
		const pi = {
			on: (event: string) => events.push(event),
			registerProvider: (name: string) => providers.push(name),
			addModeSearchPath: (p: string) => modePaths.push(p),
		} as unknown as Parameters<ReturnType<typeof buildPluginFactory>>[0];

		const factory = buildPluginFactory(plugin);
		factory(pi);

		expect(factory.displayName).toBe("plugin:p");
		expect(events).toContain("resources_discover");
		expect(events).toContain("tool_call"); // PreToolUse hook
		expect(providers).toEqual(["prov"]);
		expect(modePaths).toEqual(["/tmp/p/agents"]);
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
