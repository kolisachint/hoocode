/**
 * `--platform` — session-wide artifact platform targeting.
 *
 * Covers the full path: CLI token parsing → normalization/session state → the
 * two resolvers → plugin authoring targets (writePluginDraft) → workspace
 * scaffolds (/new-skill //new-agent //new-command), plus the per-adapter
 * workspace layouts (Claude `.claude/…`, Copilot `.github/…`, native `.agents/…`).
 *
 * The central invariant: **plugins are never produced for `agents`**. A plugin is
 * a distribution unit and the native layout belongs to no marketplace, so it
 * could never be published or installed. Workspace scaffolds are consumed in
 * place and keep `agents` as a first-class target.
 * See docs/plugin-system-architecture.md §1.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { parseArgs } from "../src/cli/args.js";
import { writePluginDraft } from "../src/core/extensions/plugins/authoring.js";
import { claudeFormat, copilotFormat, getFormatByPlatform } from "../src/core/extensions/plugins/formats/index.js";
import {
	DEFAULT_PLUGIN_PLATFORMS,
	getWorkspacePlatforms,
	normalizePlatformToken,
	parsePlatforms,
	resolvePluginPlatforms,
	setPlatforms,
} from "../src/core/extensions/plugins/formats/platform-targets.js";
import { parsePluginDir } from "../src/core/extensions/plugins/manifest.js";
import { setupScaffold } from "../src/extensions/core/scaffold.js";

afterEach(() => setPlatforms(undefined));

describe("--platform CLI parsing", () => {
	it("collects a single token", () => {
		expect(parseArgs(["--platform", "copilot"]).platform).toEqual(["copilot"]);
	});

	it("splits comma-separated lists and merges repeats", () => {
		const parsed = parseArgs(["--platform", "copilot, claude", "--platform", "agents"]);
		expect(parsed.platform).toEqual(["copilot", "claude", "agents"]);
	});

	it("leaves platform unset when the flag is absent", () => {
		expect(parseArgs(["hello"]).platform).toBeUndefined();
	});
});

describe("platform token normalization", () => {
	it("folds every documented alias", () => {
		expect(normalizePlatformToken("copilot")).toBe("github");
		expect(normalizePlatformToken("GH")).toBe("github");
		expect(normalizePlatformToken("github")).toBe("github");
		expect(normalizePlatformToken("Claude")).toBe("claude");
		expect(normalizePlatformToken("native")).toBe("agents");
		expect(normalizePlatformToken("agents")).toBe("agents");
		expect(normalizePlatformToken("vscode")).toBeUndefined();
	});

	it("parsePlatforms dedupes and reports invalid tokens", () => {
		const { platforms, invalid } = parsePlatforms(["copilot", "github", "bogus", "claude"]);
		expect(platforms).toEqual(["github", "claude"]);
		expect(invalid).toEqual(["bogus"]);
	});
});

describe("plugin platform resolution", () => {
	it("defaults to claude when nothing is configured", () => {
		expect(resolvePluginPlatforms()).toEqual([...DEFAULT_PLUGIN_PLATFORMS]);
		expect(resolvePluginPlatforms()).toEqual(["claude"]);
	});

	it("session targets replace the default; an explicit set still wins", () => {
		setPlatforms(["github"]);
		expect(resolvePluginPlatforms()).toEqual(["github"]);
		expect(resolvePluginPlatforms(["claude"])).toEqual(["claude"]);
	});

	it("throws on an explicit `agents` rather than silently dropping it", () => {
		expect(() => resolvePluginPlatforms(["agents"])).toThrow(/belongs to no marketplace/);
		expect(() => resolvePluginPlatforms(["claude", "agents"])).toThrow(/agents/);
	});

	it("filters a session-level `agents` instead of throwing, since it also drives scaffolds", () => {
		setPlatforms(["agents"]);
		expect(resolvePluginPlatforms()).toEqual(["claude"]);

		setPlatforms(["agents", "github"]);
		expect(resolvePluginPlatforms()).toEqual(["github"]);
	});

	it("clears back to the default", () => {
		setPlatforms(["github"]);
		setPlatforms(undefined);
		expect(resolvePluginPlatforms()).toEqual([...DEFAULT_PLUGIN_PLATFORMS]);
	});
});

describe("workspace platform resolution", () => {
	it("is undefined until configured, so scaffolds keep their hoocode-native location", () => {
		expect(getWorkspacePlatforms()).toBeUndefined();
	});

	it("keeps `agents` — a scaffolded artifact is consumed in place, not distributed", () => {
		setPlatforms(["agents"]);
		expect(getWorkspacePlatforms()).toEqual(["agents"]);
	});
});

describe("plugin authoring honors the session targets", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-platform-"));
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("--platform copilot writes only the Copilot manifest, and round-trips", () => {
		setPlatforms(["github"]);
		const result = writePluginDraft(cwd, {
			id: "gh-only",
			description: "copilot-targeted",
			skills: [{ name: "helper", description: "helps", body: "Help." }],
		});

		expect(fs.existsSync(path.join(result.dest, ".github", "plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(result.dest, ".claude-plugin"))).toBe(false);
		expect(fs.existsSync(path.join(result.dest, "skills", "helper", "SKILL.md"))).toBe(true);

		const parsed = parsePluginDir(result.dest);
		expect(parsed?.id).toBe("gh-only");
		expect(parsed?.supportPlatform).toEqual(["github"]);
	});

	it("--platform claude writes only the Claude manifest, and round-trips", () => {
		setPlatforms(["claude"]);
		const result = writePluginDraft(cwd, {
			id: "claude-only",
			commands: [{ name: "go", body: "Go." }],
		});

		expect(fs.existsSync(path.join(result.dest, ".claude-plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(result.dest, ".github"))).toBe(false);

		const parsed = parsePluginDir(result.dest);
		expect(parsed?.supportPlatform).toEqual(["claude"]);
	});

	it("defaults to claude — never the native layout — when no target is configured", () => {
		const result = writePluginDraft(cwd, { id: "defaulted", skills: [{ name: "s", body: "S." }] });
		expect(fs.existsSync(path.join(result.dest, ".claude-plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(result.dest, ".agents-plugin"))).toBe(false);
		expect(fs.existsSync(path.join(result.dest, ".github"))).toBe(false);
		expect(fs.existsSync(path.join(result.dest, "skills", "s", "SKILL.md"))).toBe(true);

		const parsed = parsePluginDir(result.dest);
		expect(parsed?.supportPlatform).toEqual(["claude"]);
	});

	it("never emits the native layout, even when the session asked for agents", () => {
		setPlatforms(["agents"]);
		const result = writePluginDraft(cwd, { id: "no-native", skills: [{ name: "s", body: "S." }] });
		expect(fs.existsSync(path.join(result.dest, ".agents-plugin"))).toBe(false);
		expect(fs.existsSync(path.join(result.dest, ".claude-plugin", "plugin.json"))).toBe(true);
	});

	it("filters a stale `agents` carried on the draft rather than refusing the write", () => {
		// A draft rebuilt from a plugin authored before production became
		// platform-specific still carries supportPlatform: ["agents"].
		const result = writePluginDraft(cwd, {
			id: "legacy-draft",
			supportPlatform: ["agents"],
			skills: [{ name: "s", body: "S." }],
		});
		expect(parsePluginDir(result.dest)?.supportPlatform).toEqual(["claude"]);
	});

	it("refuses an explicit `agents` target outright", () => {
		expect(() => writePluginDraft(cwd, { id: "nope", skills: [{ name: "s", body: "S." }] }, ["agents"])).toThrow(
			/belongs to no marketplace/,
		);
	});
});

describe("workspace layouts (per-adapter conventions)", () => {
	it("Copilot: .github/skills, .github/agents/*.agent.md (YAML-list tools), .github/prompts/*.prompt.md", () => {
		const ws = copilotFormat.workspace;
		const skill = ws.emitSkill({ name: "review-db", description: "reviews", body: "Review." });
		expect(skill.path).toBe(path.join(".github", "skills", "review-db", "SKILL.md"));
		expect(skill.content).toContain("name: review-db");

		const agent = ws.emitAgent({ name: "scout", description: "explores", tools: "read, grep", body: "Explore." });
		expect(agent.path).toBe(path.join(".github", "agents", "scout.agent.md"));
		expect(agent.content).toContain("tools: ['read', 'grep']");

		const cmd = ws.emitCommand({ name: "ship", description: "ships it", body: "Ship." });
		expect(cmd.path).toBe(path.join(".github", "prompts", "ship.prompt.md"));
		expect(cmd.content).toContain("description: ships it");
	});

	it("Claude: .claude/skills, .claude/agents/*.md (comma-string tools), .claude/commands/*.md", () => {
		const ws = claudeFormat.workspace;
		expect(ws.emitSkill({ name: "review-db", body: "R." }).path).toBe(
			path.join(".claude", "skills", "review-db", "SKILL.md"),
		);
		const agent = ws.emitAgent({ name: "scout", tools: "read, grep", body: "E." });
		expect(agent.path).toBe(path.join(".claude", "agents", "scout.md"));
		expect(agent.content).toContain("tools: read, grep");
		expect(ws.emitCommand({ name: "ship", body: "S." }).path).toBe(path.join(".claude", "commands", "ship.md"));
	});

	it("native: .agents/skills|agents|commands", () => {
		const ws = getFormatByPlatform("agents")?.workspace;
		expect(ws?.emitSkill({ name: "x", body: "X." }).path).toBe(path.join(".agents", "skills", "x", "SKILL.md"));
		expect(ws?.emitAgent({ name: "x", body: "X." }).path).toBe(path.join(".agents", "agents", "x.md"));
		expect(ws?.emitCommand({ name: "x", body: "X." }).path).toBe(path.join(".agents", "commands", "x.md"));
	});
});

describe("scaffold commands (/new-skill //new-agent //new-command)", () => {
	let cwd: string;
	let commands: Map<string, { handler: (args: string, ctx: unknown) => Promise<void> }>;
	let notifications: string[];
	let ctx: unknown;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-scaffold-"));
		commands = new Map();
		notifications = [];
		const pi = {
			registerCommand: (name: string, def: { handler: (args: string, ctx: unknown) => Promise<void> }) =>
				commands.set(name, def),
		} as never;
		setupScaffold(pi);
		ctx = { cwd, ui: { notify: (msg: string) => notifications.push(msg) } } as never;
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("keeps the legacy .hoocode/ layout when no targets are configured", async () => {
		await commands.get("new-skill")?.handler("my-skill", ctx);
		expect(fs.existsSync(path.join(cwd, ".hoocode", "skills", "my-skill", "SKILL.md"))).toBe(true);
	});

	it("--platform copilot scaffolds into the Copilot workspace directories", async () => {
		setPlatforms(["github"]);
		await commands.get("new-skill")?.handler("my-skill", ctx);
		await commands.get("new-agent")?.handler("my-agent", ctx);
		await commands.get("new-command")?.handler("my-cmd", ctx);

		expect(fs.existsSync(path.join(cwd, ".github", "skills", "my-skill", "SKILL.md"))).toBe(true);
		expect(fs.existsSync(path.join(cwd, ".github", "agents", "my-agent.agent.md"))).toBe(true);
		expect(fs.existsSync(path.join(cwd, ".github", "prompts", "my-cmd.prompt.md"))).toBe(true);
		expect(fs.existsSync(path.join(cwd, ".hoocode"))).toBe(false);

		// Copilot custom agents take tools as a YAML list.
		const agent = fs.readFileSync(path.join(cwd, ".github", "agents", "my-agent.agent.md"), "utf8");
		expect(agent).toContain("tools: ['read', 'bash']");
	});

	it("--platform claude,copilot scaffolds into both workspaces", async () => {
		setPlatforms(["claude", "github"]);
		await commands.get("new-skill")?.handler("dual", ctx);
		expect(fs.existsSync(path.join(cwd, ".claude", "skills", "dual", "SKILL.md"))).toBe(true);
		expect(fs.existsSync(path.join(cwd, ".github", "skills", "dual", "SKILL.md"))).toBe(true);
	});

	it("--platform agents scaffolds into .agents/ — unlike plugins, this target is valid here", async () => {
		setPlatforms(["agents"]);
		await commands.get("new-skill")?.handler("portable", ctx);
		expect(fs.existsSync(path.join(cwd, ".agents", "skills", "portable", "SKILL.md"))).toBe(true);
	});

	it("never clobbers an existing artifact", async () => {
		setPlatforms(["github"]);
		const file = path.join(cwd, ".github", "skills", "kept", "SKILL.md");
		fs.mkdirSync(path.dirname(file), { recursive: true });
		fs.writeFileSync(file, "original", "utf8");

		await commands.get("new-skill")?.handler("kept", ctx);
		expect(fs.readFileSync(file, "utf8")).toBe("original");
		expect(notifications.some((n) => n.includes("already exist"))).toBe(true);
	});
});
