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
import { ENV_AGENT_DIR } from "../src/config.js";
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
import { productionPluginDir } from "../src/core/extensions/plugins/locations.js";
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
	// Authored plugins land in a global production home, so the whole home root is
	// redirected into a temp dir — a test must never write into a real ~/.claude.
	let home: string;
	let priorAgentDir: string | undefined;

	const destFor = (id: string, platform: "claude" | "github" = "claude") => productionPluginDir(platform, id);

	beforeEach(() => {
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-home-"));
		priorAgentDir = process.env[ENV_AGENT_DIR];
		process.env[ENV_AGENT_DIR] = path.join(home, ".hoocode");
	});

	afterEach(() => {
		if (priorAgentDir === undefined) delete process.env[ENV_AGENT_DIR];
		else process.env[ENV_AGENT_DIR] = priorAgentDir;
		fs.rmSync(home, { recursive: true, force: true });
	});

	it("--platform copilot writes only the Copilot manifest, and round-trips", () => {
		setPlatforms(["github"]);
		const result = writePluginDraft({
			id: "gh-only",
			description: "copilot-targeted",
			skills: [{ name: "helper", description: "helps", body: "Help." }],
		});

		expect(result.dest).toBe(destFor("gh-only", "github"));
		expect(fs.existsSync(path.join(result.dest, "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(result.dest, ".claude-plugin"))).toBe(false);
		expect(fs.existsSync(path.join(result.dest, "skills", "helper", "SKILL.md"))).toBe(true);

		const parsed = parsePluginDir(result.dest);
		expect(parsed?.id).toBe("gh-only");
		expect(parsed?.supportPlatform).toEqual(["github"]);
	});

	it("--platform claude writes into the skills-directory drop-in Claude Code reads", () => {
		setPlatforms(["claude"]);
		const result = writePluginDraft({ id: "claude-only", commands: [{ name: "go", body: "Go." }] });

		// The documented drop-in: ~/.claude/skills/<id>/, discovered in place.
		expect(result.dest).toBe(path.join(home, ".claude", "skills", "claude-only"));
		expect(fs.existsSync(path.join(result.dest, ".claude-plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(result.dest, ".github"))).toBe(false);

		expect(parsePluginDir(result.dest)?.supportPlatform).toEqual(["claude"]);
	});

	it("defaults to claude — never the native layout — when no target is configured", () => {
		const result = writePluginDraft({ id: "defaulted", skills: [{ name: "s", body: "S." }] });
		expect(fs.existsSync(path.join(result.dest, ".claude-plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(result.dest, ".agents-plugin"))).toBe(false);
		expect(parsePluginDir(result.dest)?.supportPlatform).toEqual(["claude"]);
	});

	it("never emits the native layout, even when the session asked for agents", () => {
		setPlatforms(["agents"]);
		const result = writePluginDraft({ id: "no-native", skills: [{ name: "s", body: "S." }] });
		expect(fs.existsSync(path.join(result.dest, ".agents-plugin"))).toBe(false);
		expect(fs.existsSync(path.join(result.dest, ".claude-plugin", "plugin.json"))).toBe(true);
	});

	it("filters a stale `agents` carried on the draft rather than refusing the write", () => {
		const result = writePluginDraft({
			id: "legacy-draft",
			supportPlatform: ["agents"],
			skills: [{ name: "s", body: "S." }],
		});
		expect(parsePluginDir(result.dest)?.supportPlatform).toEqual(["claude"]);
	});

	it("refuses an explicit `agents` target outright", () => {
		expect(() => writePluginDraft({ id: "nope", skills: [{ name: "s", body: "S." }] }, ["agents"])).toThrow(
			/belongs to no marketplace/,
		);
	});

	it("promote: false stops at an ephemeral draft, leaving the production home untouched", () => {
		const result = writePluginDraft({ id: "held", skills: [{ name: "s", body: "S." }] }, undefined, {
			promote: false,
		});
		expect(result.promoted).toBe(false);
		expect(result.dest).not.toBe(destFor("held"));
		// The draft parses, so the gates have something real to run against...
		expect(parsePluginDir(result.dest)?.id).toBe("held");
		// ...but nothing is live yet.
		expect(fs.existsSync(destFor("held"))).toBe(false);
	});

	it("promotes over an existing plugin rather than merging into it", () => {
		writePluginDraft({ id: "replaced", skills: [{ name: "old", body: "Old." }] });
		writePluginDraft({ id: "replaced", skills: [{ name: "new", body: "New." }] });
		const dest = destFor("replaced");
		expect(fs.existsSync(path.join(dest, "skills", "new", "SKILL.md"))).toBe(true);
		expect(fs.existsSync(path.join(dest, "skills", "old", "SKILL.md"))).toBe(false);
	});

	it("keeps the two platforms' artifacts apart — same id is not a collision", () => {
		setPlatforms(["claude"]);
		writePluginDraft({ id: "dual", skills: [{ name: "s", body: "S." }] });
		setPlatforms(["github"]);
		writePluginDraft({ id: "dual", skills: [{ name: "s", body: "S." }] });

		expect(fs.existsSync(path.join(destFor("dual", "claude"), ".claude-plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(destFor("dual", "github"), "plugin.json"))).toBe(true);
	});
});

describe("workspace layouts (per-adapter conventions)", () => {
	it("Copilot: .github/skills, .github/agents/*.agent.md (YAML-list tools), .github/prompts/*.prompt.md", () => {
		const ws = copilotFormat.workspace;
		const skill = ws.emitSkill({ name: "review-db", description: "reviews", body: "Review." });
		expect(skill.path).toBe(path.join(".github", "skills", "review-db", "SKILL.md"));
		expect(skill.content).toContain("name: review-db");

		const agent = ws.emitAgent({ name: "scout", description: "explores", tools: "read, search", body: "Explore." });
		expect(agent.path).toBe(path.join(".github", "agents", "scout.agent.md"));
		expect(agent.content).toContain("tools: ['read', 'search']");

		const cmd = ws.emitCommand({ name: "ship", description: "ships it", body: "Ship." });
		expect(cmd.path).toBe(path.join(".github", "prompts", "ship.prompt.md"));
		expect(cmd.content).toContain("description: ships it");
	});

	it("Claude: .claude/skills, .claude/agents/*.md (comma-string tools), .claude/commands/*.md", () => {
		const ws = claudeFormat.workspace;
		expect(ws.emitSkill({ name: "review-db", body: "R." }).path).toBe(
			path.join(".claude", "skills", "review-db", "SKILL.md"),
		);
		const agent = ws.emitAgent({ name: "scout", tools: "read, search", body: "E." });
		expect(agent.path).toBe(path.join(".claude", "agents", "scout.md"));
		expect(agent.content).toContain("tools: read, search");
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
