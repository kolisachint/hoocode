import { spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
	defaultMarketplaceRecord,
	findAvailablePlugin,
	installAvailablePlugin,
	installedPluginsDir,
	isPluginInstalled,
	listAvailablePlugins,
	listInstalledPlugins,
	marketplaceCacheDir,
	readMarketplaceRecords,
	uninstallPlugin,
	WELL_KNOWN_MARKETPLACES,
} from "../src/core/extensions/plugins/install.js";
import { parseMarketplaceDir, resolvePluginSource } from "../src/core/extensions/plugins/marketplace.js";
import {
	createInstallPluginToolDefinition,
	createListPluginsToolDefinition,
	createSearchPluginsToolDefinition,
	createUninstallPluginToolDefinition,
	PLUGIN_SYSTEM_TOOL_NAMES,
} from "../src/core/tools/plugins.js";

function writeJson(file: string, data: unknown): void {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, JSON.stringify(data, null, 2));
}

/** Minimal ExtensionContext stub sufficient for the plugin tools. */
function makeCtx(cwd: string) {
	const notifications: string[] = [];
	const activations: string[] = [];
	const reloadRequests: string[] = [];
	const ctx = {
		cwd,
		hasUI: false,
		ui: { notify: (msg: string) => notifications.push(msg) },
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
		requestReloadWhenIdle: () => {
			reloadRequests.push("reload");
		},
	} as never;
	return { ctx, notifications, activations, reloadRequests };
}

/**
 * Keep tool tests hermetic: pre-create an EMPTY cache dir for every well-known
 * marketplace so SearchPlugins' lazy fetch no-ops (no network) and the empty
 * dir parses to no manifest (skipped from results).
 */
function stubWellKnownMarketplaces(cwd: string): void {
	for (const wk of WELL_KNOWN_MARKETPLACES) {
		fs.mkdirSync(marketplaceCacheDir(cwd, wk.url), { recursive: true });
	}
}

/** Seed a local marketplace with a single native-format plugin and register it. */
function seedLocalMarketplace(cwd: string): void {
	const market = path.join(cwd, "market");
	writeJson(path.join(market, ".agents-plugin", "marketplace.json"), {
		name: "local",
		plugins: [{ name: "widget", source: "./plugins/widget", description: "A widget." }],
	});
	writeJson(path.join(market, "plugins", "widget", ".agents-plugin", "plugin.json"), {
		name: "widget",
		version: "1.0.0",
	});
	fs.mkdirSync(path.join(market, "plugins", "widget", "skills", "w"), { recursive: true });
	fs.writeFileSync(
		path.join(market, "plugins", "widget", "skills", "w", "SKILL.md"),
		"---\nname: w\ndescription: does w\n---\n\nDo w.\n",
	);
	writeJson(path.join(cwd, ".agents", "marketplaces.json"), {
		marketplaces: [{ location: market, dir: market }],
	});
}

function execInDir(dir: string, command: string, args: string[]): void {
	const result = spawnSync(command, args, { cwd: dir, env: process.env });
	if (result.status !== 0) {
		throw new Error(`${command} ${args.join(" ")} failed: ${result.stderr?.toString() ?? ""}`);
	}
}

/** Seed a git-backed marketplace with a plugin in a subdirectory. */
function seedGitSubdirMarketplace(cwd: string): { marketDir: string; repoDir: string } {
	const repoDir = path.join(cwd, "repo");
	const marketDir = path.join(cwd, "market");

	fs.mkdirSync(path.join(repoDir, "plugins", "widget"), { recursive: true });
	writeJson(path.join(repoDir, "plugins", "widget", ".agents-plugin", "plugin.json"), {
		name: "widget",
		version: "1.0.0",
	});
	fs.mkdirSync(path.join(repoDir, "plugins", "widget", "skills", "w"), { recursive: true });
	fs.writeFileSync(
		path.join(repoDir, "plugins", "widget", "skills", "w", "SKILL.md"),
		"---\nname: w\ndescription: does w\n---\n\nDo w.\n",
	);

	execInDir(repoDir, "git", ["init", "--quiet"]);
	execInDir(repoDir, "git", ["config", "user.email", "test@example.com"]);
	execInDir(repoDir, "git", ["config", "user.name", "Test"]);
	execInDir(repoDir, "git", ["add", "."]);
	execInDir(repoDir, "git", ["commit", "--quiet", "-m", "initial"]);

	writeJson(path.join(marketDir, ".agents-plugin", "marketplace.json"), {
		name: "git-market",
		plugins: [
			{
				name: "widget",
				description: "A widget from a git subdirectory.",
				source: { source: "git-subdir", url: repoDir, path: "plugins/widget" },
			},
		],
	});
	writeJson(path.join(cwd, ".agents", "marketplaces.json"), {
		marketplaces: [{ location: marketDir, dir: marketDir }],
	});
	return { marketDir, repoDir };
}

describe("plugin install engine", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-lifecycle-"));
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("always includes the bundled default marketplace", () => {
		const records = readMarketplaceRecords(cwd);
		expect(records.some((r) => r.dir === defaultMarketplaceRecord().dir)).toBe(true);
		// The default marketplace ships a hello-world plugin.
		expect(listAvailablePlugins(cwd).some((p) => p.name === "hello-world")).toBe(true);
	});

	it("lists user marketplace plugins alongside the default", () => {
		seedLocalMarketplace(cwd);
		const available = listAvailablePlugins(cwd);
		expect(available.some((p) => p.name === "hello-world")).toBe(true);
		const widget = available.find((p) => p.name === "widget");
		expect(widget).toBeDefined();
		expect(widget?.sourceKind).toBe("local");
		expect(widget?.supportPlatform).toEqual(["agents"]);
	});

	it("installs a local plugin and can uninstall it (reversible)", async () => {
		seedLocalMarketplace(cwd);
		expect(isPluginInstalled(cwd, "widget", cwd)).toBe(false);

		const outcome = await installAvailablePlugin(cwd, "widget");
		expect(outcome.installed).toBe(true);
		expect(fs.existsSync(path.join(installedPluginsDir(cwd), "widget", ".agents-plugin", "plugin.json"))).toBe(true);
		expect(listInstalledPlugins(cwd, cwd).some((p) => p.id === "widget")).toBe(true);

		const removed = uninstallPlugin(cwd, "widget");
		expect(removed.removed).toBe(true);
		expect(fs.existsSync(path.join(installedPluginsDir(cwd), "widget"))).toBe(false);
	});

	it("reports a helpful message when installing an unknown plugin", async () => {
		const outcome = await installAvailablePlugin(cwd, "does-not-exist");
		expect(outcome.installed).toBe(false);
		expect(outcome.message).toContain("not found");
	});

	it("finds available plugins by exact name", () => {
		seedLocalMarketplace(cwd);
		expect(findAvailablePlugin(cwd, "widget")?.name).toBe("widget");
		expect(findAvailablePlugin(cwd, "nope")).toBeUndefined();
	});

	it("resolves structured url and git-subdir sources", () => {
		const url = resolvePluginSource({ source: "url", url: "https://example.com/repo.git" }, cwd);
		expect(url).toEqual({ kind: "git", url: "https://example.com/repo.git" });

		const subdir = resolvePluginSource(
			{ source: "git-subdir", url: "https://example.com/repo.git", path: "plugins/foo" },
			cwd,
		);
		expect(subdir).toEqual({
			kind: "git-subdir",
			url: "https://example.com/repo.git",
			path: "plugins/foo",
		});
	});

	it("parses marketplace entries with structured sources and skips invalid ones", () => {
		const market = path.join(cwd, "market");
		writeJson(path.join(market, ".agents-plugin", "marketplace.json"), {
			name: "mixed",
			plugins: [
				{ name: "good-url", source: { source: "url", url: "https://example.com/repo.git" } },
				{ name: "good-subdir", source: { source: "git-subdir", url: "https://example.com/repo.git", path: "p" } },
				{ name: "bad-source", source: { source: "unknown", url: "https://example.com/repo.git" } },
				{ name: "bad-shape", source: 123 },
			],
		});
		const parsed = parseMarketplaceDir(market);
		expect(parsed).not.toBeNull();
		expect(parsed?.plugins.map((p) => p.name)).toEqual(["good-url", "good-subdir"]);
	});

	it("installs a plugin from a git-subdir source", async () => {
		seedGitSubdirMarketplace(cwd);
		expect(isPluginInstalled(cwd, "widget", cwd)).toBe(false);

		const outcome = await installAvailablePlugin(cwd, "widget");
		expect(outcome.installed).toBe(true);
		expect(fs.existsSync(path.join(installedPluginsDir(cwd), "widget", ".agents-plugin", "plugin.json"))).toBe(true);
		expect(listInstalledPlugins(cwd, cwd).some((p) => p.id === "widget")).toBe(true);
	});
});

describe("plugin lifecycle tools", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-lifecycle-tools-"));
		seedLocalMarketplace(cwd);
		stubWellKnownMarketplaces(cwd);
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("exposes the capability-acquisition tool names (lifecycle + authoring) for the guardrail", () => {
		expect(PLUGIN_SYSTEM_TOOL_NAMES).toEqual([
			"SearchPlugins",
			"ListPlugins",
			"SuggestPluginInstall",
			"InstallPlugin",
			"UninstallPlugin",
			"ProposePlugin",
			"UpdatePlugin",
		]);
	});

	it("SearchPlugins filters by query and platform", async () => {
		const { ctx } = makeCtx(cwd);
		const tool = createSearchPluginsToolDefinition();
		const all = await tool.execute("1", {}, undefined, undefined, ctx);
		expect((all.details as { count: number }).count).toBeGreaterThanOrEqual(2);

		const q = await tool.execute("2", { query: "widget" }, undefined, undefined, ctx);
		expect((q.details as { count: number }).count).toBe(1);

		const gh = await tool.execute("3", { platform: "github" }, undefined, undefined, ctx);
		// hello-world (default marketplace) supports github; widget (agents) does not.
		expect((gh.details as { count: number }).count).toBe(1);
	});

	it("InstallPlugin installs and announces, ListPlugins reflects it, UninstallPlugin reverses it", async () => {
		const { ctx, notifications } = makeCtx(cwd);
		const install = createInstallPluginToolDefinition();
		const res = await install.execute("1", { name: "widget", reason: "need a widget" }, undefined, undefined, ctx);
		expect((res.details as { installed: boolean }).installed).toBe(true);
		// Transparency: it announced the intent and the outcome.
		expect(notifications.some((n) => n.includes("need a widget"))).toBe(true);

		const list = createListPluginsToolDefinition();
		const listed = await list.execute("2", {}, undefined, undefined, ctx);
		expect(listed.content[0]).toMatchObject({ type: "text" });
		expect((listed.content[0] as { text: string }).text).toContain("widget");

		const uninstall = createUninstallPluginToolDefinition();
		const removed = await uninstall.execute("3", { name: "widget" }, undefined, undefined, ctx);
		expect((removed.details as { removed: boolean }).removed).toBe(true);
	});
});
