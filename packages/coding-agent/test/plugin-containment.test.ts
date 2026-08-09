/**
 * Plugin content containment (architecture step 3).
 *
 * Three rules that together stop a plugin's contents leaking into surfaces they
 * do not belong to:
 *  - a plugin directory is not a skill tree (the plain scan stops at it);
 *  - plugin skills are namespaced `<plugin>:<skill>`;
 *  - a project-scoped plugin contributes passive capabilities only.
 *
 * Deliberately kept out of plugins.test.ts: that suite imports the extension
 * loader, which pulls in the workspace packages and cannot be resolved without a
 * build. These rules need none of it.
 * See docs/plugin-system-architecture.md §5.7–§5.9.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { clearExtensionMcpServers, getExtensionMcpServers } from "../src/core/extension-mcp-servers.js";
import { isProjectSuppliedPlugin } from "../src/core/extensions/loader.js";
import { emitForPlatforms, isPluginRoot } from "../src/core/extensions/plugins/formats/index.js";
import { buildPluginFactory, parsePluginDir, withheldCapabilities } from "../src/core/extensions/plugins/index.js";
import { loadSkills, loadSkillsFromDir } from "../src/core/skills.js";

function writeJson(file: string, data: unknown): void {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, JSON.stringify(data, null, 2), "utf8");
}

describe("plugin content containment (step 3)", () => {
	let dir: string;

	beforeEach(() => {
		dir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-contain-"));
	});

	afterEach(() => {
		fs.rmSync(dir, { recursive: true, force: true });
	});

	function writeSkill(file: string, name: string): void {
		fs.mkdirSync(path.dirname(file), { recursive: true });
		fs.writeFileSync(file, `---\nname: ${name}\ndescription: does ${name} things\n---\n\nBody.\n`, "utf8");
	}

	it("isPluginRoot recognizes every format, and ignores a plain skill dir", () => {
		const plain = path.join(dir, "plain");
		writeSkill(path.join(plain, "SKILL.md"), "plain");
		expect(isPluginRoot(plain)).toBe(false);

		const asPlugin = path.join(dir, "as-plugin");
		writeJson(path.join(asPlugin, ".claude-plugin", "plugin.json"), { name: "as-plugin" });
		expect(isPluginRoot(asPlugin)).toBe(true);
	});

	it("the plain-skill scan does not descend into a plugin root", () => {
		// A skills directory holding both a plain skill and a skills-dir plugin,
		// which is exactly what ~/.claude/skills/ looks like in practice.
		writeSkill(path.join(dir, "loose", "SKILL.md"), "loose");
		const plugin = path.join(dir, "my-tool");
		writeJson(path.join(plugin, ".claude-plugin", "plugin.json"), { name: "my-tool" });
		writeSkill(path.join(plugin, "skills", "inner", "SKILL.md"), "inner");

		const { skills } = loadSkillsFromDir({ dir, source: "test" });
		const names = skills.map((s) => s.name).sort();
		expect(names).toEqual(["loose"]);
		// `inner` belongs to the plugin and is contributed through the plugin
		// loader instead — it must not also appear as a loose top-level skill.
		expect(names).not.toContain("inner");
	});

	it("namespaces plugin skills so two plugins shipping the same name both survive", () => {
		const a = path.join(dir, "a", "skills");
		const b = path.join(dir, "b", "skills");
		writeSkill(path.join(a, "review", "SKILL.md"), "review");
		writeSkill(path.join(b, "review", "SKILL.md"), "review");

		const { skills } = loadSkills({
			cwd: dir,
			agentDir: path.join(dir, "agentdir"),
			skillPaths: [a, b],
			includeDefaults: false,
			includeClaude: false,
			namespaces: new Map([
				[a, "alpha"],
				[b, "beta"],
			]),
		});
		expect(skills.map((s) => s.name).sort()).toEqual(["alpha:review", "beta:review"]);
	});

	it("leaves non-plugin skill paths unnamespaced", () => {
		const plain = path.join(dir, "plain");
		writeSkill(path.join(plain, "solo", "SKILL.md"), "solo");
		const { skills } = loadSkills({
			cwd: dir,
			agentDir: path.join(dir, "agentdir"),
			skillPaths: [plain],
			includeDefaults: false,
			includeClaude: false,
		});
		expect(skills.map((s) => s.name)).toEqual(["solo"]);
	});

	it("scopes the passive gate to .claude/skills, not to every project path", () => {
		// `<cwd>/.agents/plugins` and `<cwd>/.hoocode/plugins` are hoocode's own
		// former install homes: they hold plugins the user installed deliberately,
		// so gating them would break existing setups. Only the vendor convention
		// for repo-committed, collaborator-shared plugins is withheld.
		const cwd = path.join(dir, "repo");
		expect(isProjectSuppliedPlugin(path.join(cwd, ".claude", "skills", "shared"), cwd)).toBe(true);
		expect(isProjectSuppliedPlugin(path.join(cwd, ".agents", "plugins", "installed"), cwd)).toBe(false);
		expect(isProjectSuppliedPlugin(path.join(cwd, ".hoocode", "plugins", "installed"), cwd)).toBe(false);
		expect(isProjectSuppliedPlugin(path.join(dir, "elsewhere", "global"), cwd)).toBe(false);
	});

	it("withholds hooks and mcp servers from a project-scoped plugin", () => {
		const root = path.join(dir, "proj");
		writeJson(path.join(root, ".agents-plugin", "plugin.json"), {
			name: "proj",
			mcpServers: { thing: { command: "run-thing" } },
		});
		writeJson(path.join(root, "hooks", "hooks.json"), {
			hooks: { PreToolUse: [{ matcher: "*", hooks: [{ type: "command", command: "echo hi" }] }] },
		});
		const plugin = parsePluginDir(root);
		expect(plugin).not.toBeNull();
		if (!plugin) return;

		expect(withheldCapabilities(plugin).sort()).toEqual(["hooks", "mcp servers"]);

		// The hooks bridge wires itself through pi.on(...), and MCP servers go to
		// the module-level extension registry — so record both, and compare the
		// gated run against the ungated one. Asserting only "nothing registered"
		// would pass even if the gate did nothing.
		const run = (passiveOnly: boolean) => {
			clearExtensionMcpServers();
			const events: string[] = [];
			const pi = {
				on: (event: string) => events.push(event),
				registerProvider: () => {},
			} as never;
			buildPluginFactory(plugin, { passiveOnly })(pi);
			return { events, mcp: getExtensionMcpServers().map((e) => e.source) };
		};

		const ungated = run(false);
		expect(ungated.events).toContain("tool_call");
		expect(ungated.mcp).toContain("proj");

		const gated = run(true);
		expect(gated.events).not.toContain("tool_call");
		expect(gated.mcp).toEqual([]);
		clearExtensionMcpServers();
	});
});

describe("adapter catch-up (step 4)", () => {
	let dir: string;

	beforeEach(() => {
		dir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-adapter-"));
	});

	afterEach(() => {
		fs.rmSync(dir, { recursive: true, force: true });
	});

	it("treats a manifest-less directory with default components as a plugin", () => {
		const root = path.join(dir, "inferred");
		fs.mkdirSync(path.join(root, "skills", "s"), { recursive: true });
		fs.writeFileSync(path.join(root, "skills", "s", "SKILL.md"), "---\nname: s\ndescription: d\n---\n\nB.\n");

		const plugin = parsePluginDir(root);
		expect(plugin?.id).toBe("inferred");
		expect(plugin?.skillsDir).toBe(path.join(root, "skills"));
		// …but not when a manifest is what's being tested for (a skills directory).
		expect(parsePluginDir(root, { requireManifest: true })).toBeNull();
	});

	it("loads a single SKILL.md at the plugin root as that plugin's one skill", () => {
		const root = path.join(dir, "one-skill");
		writeJson(path.join(root, ".claude-plugin", "plugin.json"), { name: "one-skill" });
		fs.writeFileSync(path.join(root, "SKILL.md"), "---\nname: solo\ndescription: d\n---\n\nB.\n");
		expect(parsePluginDir(root)?.skillsDir).toBe(root);
	});

	it("an explicit manifest beats an inferred match from another format", () => {
		// A Copilot plugin also looks manifest-less-Claude-ish (it has skills/), and
		// claude sorts first by precedence. Without preferring declared manifests it
		// would be misreported as a Claude plugin.
		const root = path.join(dir, "copilot-only");
		writeJson(path.join(root, "plugin.json"), { name: "copilot-only" });
		fs.mkdirSync(path.join(root, "skills", "s"), { recursive: true });
		fs.writeFileSync(path.join(root, "skills", "s", "SKILL.md"), "---\nname: s\ndescription: d\n---\n\nB.\n");

		const plugin = parsePluginDir(root);
		expect(plugin?.format).toBe("copilot");
		expect(plugin?.supportPlatform).toEqual(["github"]);
	});

	it("preserves manifest keys it does not model, and reports unsupported surfaces", () => {
		const root = path.join(dir, "rich");
		writeJson(path.join(root, "plugin.json"), {
			name: "rich",
			lspServers: "./lsp.json",
			extensions: { paths: ["./ext"], exclusive: true },
		});
		fs.mkdirSync(path.join(root, "bin"), { recursive: true });
		writeJson(path.join(root, "monitors", "monitors.json"), []);

		const plugin = parsePluginDir(root);
		expect(plugin?.unknownFields).toMatchObject({ lspServers: "./lsp.json" });
		expect(plugin?.unsupportedSurfaces).toEqual(expect.arrayContaining(["bin/", "monitors/"]));
	});

	it("carries unknown manifest keys back out through the emitter", () => {
		const files = emitForPlatforms(
			{ id: "rich", supportPlatform: ["github"], unknownFields: { lspServers: "./lsp.json" } },
			["github"],
		);
		const manifest = files.find((f) => f.path === "plugin.json");
		expect(manifest).toBeDefined();
		expect(JSON.parse(manifest!.content)).toMatchObject({ name: "rich", lspServers: "./lsp.json" });
	});
});
