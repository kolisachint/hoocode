import { spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ENV_AGENT_DIR } from "../src/config.js";
import {
	defaultMarketplaceRecord,
	findAvailablePlugin,
	installAvailablePlugin,
	isPluginInstalled,
	listAvailablePlugins,
	listInstalledPlugins,
	readMarketplaceRecords,
	uninstallPlugin,
	WELL_KNOWN_MARKETPLACES,
} from "../src/core/extensions/plugins/install.js";
import { consumptionPluginsDir, marketplaceCacheDir } from "../src/core/extensions/plugins/locations.js";
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
function stubWellKnownMarketplaces(_cwd: string): void {
	for (const wk of WELL_KNOWN_MARKETPLACES) {
		fs.mkdirSync(marketplaceCacheDir(wk.url), { recursive: true });
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

/**
 * Redirect every global plugin location into a temp home for the duration of a
 * test. The consumption home and the marketplace cache are user-scoped now, so
 * without this a test run would write into the developer's real ~/.agents.
 */
function useTempHome(): { home: () => string } {
	let home = "";
	let prior: string | undefined;
	beforeEach(() => {
		home = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-home-"));
		prior = process.env[ENV_AGENT_DIR];
		process.env[ENV_AGENT_DIR] = path.join(home, ".hoocode");
	});
	afterEach(() => {
		if (prior === undefined) delete process.env[ENV_AGENT_DIR];
		else process.env[ENV_AGENT_DIR] = prior;
		fs.rmSync(home, { recursive: true, force: true });
	});
	return { home: () => home };
}

describe("plugin install engine", () => {
	let cwd: string;
	useTempHome();

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
		expect(fs.existsSync(path.join(consumptionPluginsDir(), "widget", ".agents-plugin", "plugin.json"))).toBe(true);
		expect(listInstalledPlugins(cwd).some((p) => p.id === "widget")).toBe(true);

		const removed = uninstallPlugin(cwd, "widget");
		expect(removed.removed).toBe(true);
		expect(fs.existsSync(path.join(consumptionPluginsDir(), "widget"))).toBe(false);
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
		expect(fs.existsSync(path.join(consumptionPluginsDir(), "widget", ".agents-plugin", "plugin.json"))).toBe(true);
		expect(listInstalledPlugins(cwd).some((p) => p.id === "widget")).toBe(true);
	});
});

describe("plugin lifecycle tools", () => {
	let cwd: string;
	useTempHome();

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-lifecycle-tools-"));
		seedLocalMarketplace(cwd);
		stubWellKnownMarketplaces(cwd);
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("exposes the capability-acquisition tool names (lifecycle + authoring + publish lane) for the guardrail", () => {
		// Exhaustive on purpose: a new plugin-system tool that is not in this set is
		// a tool an authored subagent could be granted, which is the whole
		// privilege-amplification hole the guardrail exists to close. Adding one
		// here should be a deliberate line in a diff, not a silent widening.
		expect(PLUGIN_SYSTEM_TOOL_NAMES).toEqual([
			"SearchPlugins",
			"ListPlugins",
			"SuggestPluginInstall",
			"InstallPlugin",
			"UninstallPlugin",
			"ProposePlugin",
			"UpdatePlugin",
			"RemovePluginCapability",
			"PackagePlugin",
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

	it("SearchPlugins adds capability matches without disturbing the substring ones", async () => {
		// Named nothing like "send mail", but described as doing it. Substring
		// matching cannot reach this; the capability index can.
		const market = path.join(cwd, "market2");
		writeJson(path.join(market, ".agents-plugin", "marketplace.json"), {
			name: "second",
			plugins: [{ name: "postbox", source: "./p", description: "Send messages by electronic mail." }],
		});
		writeJson(path.join(cwd, ".agents", "marketplaces.json"), {
			marketplaces: [
				{ location: path.join(cwd, "market"), dir: path.join(cwd, "market") },
				{ location: market, dir: market },
			],
		});

		const { ctx } = makeCtx(cwd);
		const res = await createSearchPluginsToolDefinition().execute(
			"1",
			{ query: "send mail" },
			undefined,
			undefined,
			ctx,
		);
		const text = (res.content[0] as { text: string }).text;

		// `count` stays the substring count, so nothing that depended on the old
		// behavior shifts; the semantic hits are additive and labelled as such.
		expect((res.details as { count: number }).count).toBe(0);
		expect(text).toContain("Related (matched by capability, not name)");
		expect(text).toContain("postbox");
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
