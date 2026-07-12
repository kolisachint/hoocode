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
	readMarketplaceRecords,
	uninstallPlugin,
} from "../src/core/extensions/plugins/install.js";
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
	const ctx = {
		cwd,
		hasUI: false,
		ui: { notify: (msg: string) => notifications.push(msg) },
	} as never;
	return { ctx, notifications };
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
});

describe("plugin lifecycle tools", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-lifecycle-tools-"));
		seedLocalMarketplace(cwd);
	});

	afterEach(() => {
		fs.rmSync(cwd, { recursive: true, force: true });
	});

	it("exposes exactly the five capability-acquisition tool names", () => {
		expect(PLUGIN_SYSTEM_TOOL_NAMES).toEqual([
			"SearchPlugins",
			"ListPlugins",
			"SuggestPluginInstall",
			"InstallPlugin",
			"UninstallPlugin",
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
