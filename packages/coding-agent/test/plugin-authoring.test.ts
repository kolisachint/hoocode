import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { classifyAllowlist } from "../src/core/extensions/plugins/authoring.js";
import { parsePluginDir } from "../src/core/extensions/plugins/manifest.js";
import {
	createProposeExecutablePluginToolDefinition,
	createProposePluginToolDefinition,
} from "../src/core/tools/propose-plugin.js";

/** ExtensionContext stub. `hasUI` + a scripted confirm answer drive the executable path. */
function makeCtx(cwd: string, opts: { hasUI?: boolean; confirm?: boolean } = {}) {
	const notifications: string[] = [];
	const ctx = {
		cwd,
		hasUI: opts.hasUI ?? false,
		ui: {
			notify: (msg: string) => notifications.push(msg),
			confirm: async () => opts.confirm ?? false,
		},
	} as never;
	return { ctx, notifications };
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
		// Default targets are Claude + Copilot.
		expect(fs.existsSync(path.join(dest, ".claude-plugin", "plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(dest, "skills", "assist", "SKILL.md"))).toBe(true);
		expect(fs.existsSync(path.join(dest, ".github", "copilot-plugin.json"))).toBe(true);
		expect(fs.existsSync(path.join(dest, ".github", "chatmodes", "scout.chatmode.md"))).toBe(true);

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

	it("rejects a mutating subagent allowlist (redirects to the executable path)", async () => {
		const { ctx } = makeCtx(cwd);
		const tool = createProposePluginToolDefinition();
		const res = await tool.execute(
			"1",
			{ id: "bad", subagents: [{ name: "worker", tools: "read, bash", body: "You act." }] },
			undefined,
			undefined,
			ctx,
		);
		expect((res.details as { authored: boolean }).authored).toBe(false);
		expect((res.content[0] as { text: string }).text).toContain("ProposeExecutablePlugin");
		expect(fs.existsSync(path.join(cwd, ".agents", "plugins", "bad"))).toBe(false);
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

describe("ProposeExecutablePlugin (risk-bearing path)", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-author-exec-"));
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("blocks a hook plugin until confirmation (no UI = fail closed)", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: false });
		const tool = createProposeExecutablePluginToolDefinition();
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
		const tool = createProposeExecutablePluginToolDefinition();
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
		const tool = createProposeExecutablePluginToolDefinition();
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

	it("still rejects plugin-system tools on a subagent, even with confirmation", async () => {
		const { ctx } = makeCtx(cwd, { hasUI: true, confirm: true });
		const tool = createProposeExecutablePluginToolDefinition();
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
});
