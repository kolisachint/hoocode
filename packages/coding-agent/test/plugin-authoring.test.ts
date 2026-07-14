import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { classifyAllowlist } from "../src/core/extensions/plugins/authoring.js";
import { parsePluginDir } from "../src/core/extensions/plugins/manifest.js";
import {
	createProposePluginToolDefinition,
	createUpdatePluginToolDefinition,
} from "../src/core/tools/propose-plugin.js";

/** ExtensionContext stub. `hasUI` + a scripted confirm answer drive the executable path. */
function makeCtx(cwd: string, opts: { hasUI?: boolean; confirm?: boolean } = {}) {
	const notifications: string[] = [];
	const activations: string[] = [];
	const ctx = {
		cwd,
		hasUI: opts.hasUI ?? false,
		ui: {
			notify: (msg: string) => notifications.push(msg),
			confirm: async () => opts.confirm ?? false,
		},
		activatePlugin: (dir: string) => {
			activations.push(dir);
			return {
				activated: true,
				pluginId: "stub",
				skills: [],
				commands: [],
				agents: [],
				pendingReloadForExecutables: false,
				message: `activated ${dir}`,
			};
		},
		requestReloadWhenIdle: () => {},
	} as never;
	return { ctx, notifications, activations };
}

describe("allowlist classification", () => {
	it("treats read-only grants as low risk", () => {
		expect(classifyAllowlist("read, grep, glob").risk).toBe("read-only");
		expect(classifyAllowlist("webfetch").risk).toBe("read-only");
		expect(classifyAllowlist(undefined).risk).toBe("read-only");
	});

	it("treats mutating/exec/network/* grants as mutating", () => {
		expect(classifyAllowlist("read, bash").risk).toBe("mutating");
		expect(classifyAllowlist("write").risk).toBe("mutating");
		expect(classifyAllowlist("*").risk).toBe("mutating");
		// Unrecognized / MCP-style tools are treated as mutating (fail-safe).
		expect(classifyAllowlist("read, mcp__github__create_pr").risk).toBe("mutating");
	});

	it("flags plugin-system tools regardless of risk", () => {
		const cls = classifyAllowlist("read, InstallPlugin");
		expect(cls.pluginTools).toEqual(["InstallPlugin"]);
	});
});

describe("ProposePlugin (scaffold path)", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-author-"));
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("authors a skill + read-only subagent and round-trips through parsePluginDir (both formats)", async () => {
		const { ctx } = makeCtx(cwd);
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{
				id: "myhelper",
				description: "helper plugin",
				skills: [{ name: "assist", description: "assists", body: "Assist the user." }],
				subagents: [{ name: "scout", description: "reads code", tools: "read, grep", body: "You explore." }],
			},
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(true);

		const dest = path.join(cwd, ".agents", "plugins", "myhelper");
		// Default targets are Claude + Copilot (Copilot manifest under .github/plugin/).
		expect(fs.existsSync(path.join(dest, ".claude-plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(dest, "skills", "assist", "SKILL.md"))).toBe(true);
		expect(fs.existsSync(path.join(dest, ".github", "plugin", "plugin.json"))).toBe(true);
		// Copilot shares the Claude-mirror capability tree — one tree, two manifests.
		expect(fs.existsSync(path.join(dest, "agents", "scout.md"))).toBe(true);

		// Claude wins precedence; both platforms recorded.
		const parsed = parsePluginDir(dest);
		expect(parsed?.id).toBe("myhelper");
		expect(parsed?.supportPlatform).toEqual(["claude", "github"]);
	});

	it("honors an explicit platforms selection", async () => {
		const { ctx } = makeCtx(cwd);
		const tool = createProposePluginToolDefinition();
		await tool.execute(
			"1",
			{ id: "claudeonly", platforms: ["claude"], commands: [{ name: "go", body: "Go." }] },
			undefined,
			undefined,
			ctx,
		);
		const dest = path.join(cwd, ".agents", "plugins", "claudeonly");
		expect(fs.existsSync(path.join(dest, ".claude-plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(dest, ".github"))).toBe(false);
	});

	it("routes a mutating subagent through the confirm gate (fails closed with no UI)", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: false });
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{ id: "bad", subagents: [{ name: "worker", tools: "read, bash", body: "You act." }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(false);
		expect((res.content[0] as { text: string }).text).toContain("human confirmation");
		expect(fs.existsSync(path.join(cwd, ".agents", "plugins", "bad"))).toBe(false);
	});

	it("authors a mixed skill + mutating subagent in one call once confirmed", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: true, confirm: true });
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{
				id: "mixed",
				skills: [{ name: "assist", body: "Assist." }],
				subagents: [{ name: "worker", tools: "read, bash", body: "You act." }],
			},
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean; confirmed: boolean }).authored).toBe(true);
		expect((res.details as { confirmed: boolean }).confirmed).toBe(true);
		const dest = path.join(cwd, ".agents", "plugins", "mixed");
		expect(fs.existsSync(path.join(dest, "skills", "assist", "SKILL.md"))).toBe(true);
		expect(fs.existsSync(path.join(dest, "agents", "worker.md"))).toBe(true);
	});

	it("rejects a subagent that carries plugin-system tools (privilege guardrail)", async () => {
		const { ctx } = makeCtx(cwd);
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{ id: "evil", subagents: [{ name: "boot", tools: "read, InstallPlugin", body: "You act." }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(false);
		expect((res.content[0] as { text: string }).text).toContain("capability-acquisition");
		expect(fs.existsSync(path.join(cwd, ".agents", "plugins", "evil"))).toBe(false);
	});
});

describe("ProposePlugin (executable path — computed risk gate)", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-author-exec-"));
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("blocks a hook plugin until confirmation (no UI = fail closed)", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: false });
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{ id: "hooky", hooks: [{ event: "PreToolUse", matcher: "Bash", command: "echo hi" }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(false);
		expect(fs.existsSync(path.join(cwd, ".agents", "plugins", "hooky"))).toBe(false);
	});

	it("does not author when the human declines", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: true, confirm: false });
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{ id: "mcpy", mcpServers: [{ name: "svc", command: "svc-bin" }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean; confirmed: boolean }).confirmed).toBe(false);
		expect(fs.existsSync(path.join(cwd, ".agents", "plugins", "mcpy"))).toBe(false);
	});

	it("authors a hook + MCP plugin once confirmed, and round-trips", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: true, confirm: true });
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{
				id: "power",
				hooks: [{ event: "PreToolUse", matcher: "Bash", command: "echo hi" }],
				mcpServers: [{ name: "svc", command: "svc-bin", args: ["--port", "1"] }],
			},
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(true);

		const dest = path.join(cwd, ".agents", "plugins", "power");
		const parsed = parsePluginDir(dest);
		expect(parsed?.hooks?.PreToolUse).toHaveLength(1);
		expect(parsed?.mcpServers).toMatchObject({ svc: { command: "svc-bin" } });
	});

	it("does NOT prompt for a purely passive plugin (no confirm even with UI off)", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: false });
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{ id: "passive", commands: [{ name: "go", body: "Go." }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean; confirmed?: boolean }).authored).toBe(true);
		expect((res.details as { confirmed?: boolean }).confirmed).toBe(false);
	});

	it("still rejects plugin-system tools on a subagent, even with confirmation", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: true, confirm: true });
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{ id: "sneaky", subagents: [{ name: "w", tools: "bash, ProposePlugin", body: "You act." }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(false);
		expect(fs.existsSync(path.join(cwd, ".agents", "plugins", "sneaky"))).toBe(false);
	});

	it("rejects an empty draft (no capabilities at all)", async () => {
		const { ctx } = makeCtx(cwd);
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute("1", { id: "hollow", description: "nothing inside" }, undefined, undefined, ctx);
		expect((res.details as { authored: boolean }).authored).toBe(false);
		expect((res.content[0] as { text: string }).text).toContain("Nothing to author");
		expect(fs.existsSync(path.join(cwd, ".agents", "plugins", "hollow"))).toBe(false);
	});

	it("stamps the authored provenance marker", async () => {
		const { ctx } = makeCtx(cwd);
		const tool = createProposePluginToolDefinition();
		await tool.execute("1", { id: "marked", commands: [{ name: "go", body: "Go." }] }, undefined, undefined, ctx);
		expect(fs.existsSync(path.join(cwd, ".agents", "plugins", "marked", ".authored.json"))).toBe(true);
	});

	it("refuses to author over an existing plugin (points at UpdatePlugin)", async () => {
		const { ctx } = makeCtx(cwd);
		const tool = createProposePluginToolDefinition();
		await tool.execute("1", { id: "dup", commands: [{ name: "go", body: "Go." }] }, undefined, undefined, ctx);
		const res = await tool.execute(
			"2",
			{ id: "dup", commands: [{ name: "stop", body: "Stop." }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(false);
		expect((res.content[0] as { text: string }).text).toContain("UpdatePlugin");
	});
});

describe("UpdatePlugin (merge into an existing local plugin)", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-update-"));
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("rejects updating a plugin that does not exist", async () => {
		const { ctx } = makeCtx(cwd);
		const update = createUpdatePluginToolDefinition();
		const res = await update.execute(
			"1",
			{ id: "ghost", skills: [{ name: "s", body: "b" }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(false);
		expect((res.content[0] as { text: string }).text).toContain("ProposePlugin");
	});

	it("refuses a plugin without the authored marker (e.g. a marketplace install)", async () => {
		// Simulate a marketplace install: a valid plugin dir with no .authored.json.
		const dest = path.join(cwd, ".agents", "plugins", "thirdparty");
		fs.mkdirSync(path.join(dest, ".claude-plugin"), { recursive: true });
		fs.writeFileSync(
			path.join(dest, ".claude-plugin", "plugin.json"),
			`${JSON.stringify({ name: "thirdparty", description: "from a marketplace" })}\n`,
		);

		const { ctx } = makeCtx(cwd);
		const update = createUpdatePluginToolDefinition();
		const res = await update.execute(
			"1",
			{ id: "thirdparty", skills: [{ name: "extra", body: "Extra." }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(false);
		expect((res.content[0] as { text: string }).text).toContain("not authored");
		// The plugin was left untouched.
		expect(fs.existsSync(path.join(dest, "skills"))).toBe(false);
	});

	it("a platforms-only update adds a vendor layout without touching capabilities", async () => {
		const { ctx } = makeCtx(cwd);
		const propose = createProposePluginToolDefinition();
		await propose.execute(
			"1",
			{ id: "widen", platforms: ["claude"], commands: [{ name: "go", body: "Go." }] },
			undefined,
			undefined,
			ctx,
		);
		const dest = path.join(cwd, ".agents", "plugins", "widen");
		expect(fs.existsSync(path.join(dest, ".github"))).toBe(false);

		const update = createUpdatePluginToolDefinition();
		const res = await update.execute(
			"2",
			{ id: "widen", platforms: ["claude", "github"] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(true);
		// Copilot manifest added; original command untouched.
		expect(fs.existsSync(path.join(dest, ".github", "plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(dest, "commands", "go.md"))).toBe(true);
	});

	it("adds a skill to an existing plugin while preserving the original capabilities", async () => {
		const { ctx } = makeCtx(cwd);
		const propose = createProposePluginToolDefinition();
		await propose.execute(
			"1",
			{ id: "grow", skills: [{ name: "first", body: "First." }] },
			undefined,
			undefined,
			ctx,
		);

		const update = createUpdatePluginToolDefinition();
		const res = await update.execute(
			"2",
			{ id: "grow", skills: [{ name: "second", body: "Second." }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(true);

		const dest = path.join(cwd, ".agents", "plugins", "grow");
		// Both the original and the added skill are present.
		expect(fs.existsSync(path.join(dest, "skills", "first", "SKILL.md"))).toBe(true);
		expect(fs.existsSync(path.join(dest, "skills", "second", "SKILL.md"))).toBe(true);
	});

	it("unions an added hook with existing hooks (confirmed), preserving the original", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: true, confirm: true });
		const propose = createProposePluginToolDefinition();
		await propose.execute(
			"1",
			{ id: "hookgrow", hooks: [{ event: "PreToolUse", matcher: "Bash", command: "echo a" }] },
			undefined,
			undefined,
			ctx,
		);

		const update = createUpdatePluginToolDefinition();
		await update.execute(
			"2",
			{ id: "hookgrow", hooks: [{ event: "PostToolUse", command: "echo b" }] },
			undefined,
			undefined,
			ctx,
		);

		const parsed = parsePluginDir(path.join(cwd, ".agents", "plugins", "hookgrow"));
		expect(parsed?.hooks?.PreToolUse).toHaveLength(1);
		expect(parsed?.hooks?.PostToolUse).toHaveLength(1);
	});

	it("adding a passive skill to an executable plugin does not re-prompt (fails open with no UI)", async () => {
		// Seed an executable plugin with confirmation.
		const seed = makeCtx(cwd, { hasUI: true, confirm: true });
		const propose = createProposePluginToolDefinition();
		await propose.execute(
			"1",
			{ id: "exec", mcpServers: [{ name: "svc", command: "svc-bin" }] },
			undefined,
			undefined,
			seed.ctx,
		);

		// Now add a passive skill with NO UI — must still succeed (delta is passive).
		const noUi = makeCtx(cwd, { hasUI: false });
		const update = createUpdatePluginToolDefinition();
		const res = await update.execute(
			"2",
			{ id: "exec", skills: [{ name: "note", body: "Note." }] },
			undefined,
			undefined,
			noUi.ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(true);
		const parsed = parsePluginDir(path.join(cwd, ".agents", "plugins", "exec"));
		// The original MCP server survived the merge.
		expect(parsed?.mcpServers).toMatchObject({ svc: { command: "svc-bin" } });
	});
});
