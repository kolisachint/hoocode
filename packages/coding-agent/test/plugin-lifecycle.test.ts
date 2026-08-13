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
import {
	consumptionPluginsDir,
	marketplaceCacheDir,
	marketplaceCacheMetaPath,
} from "../src/core/extensions/plugins/locations.js";
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
 *
 * The freshness stamp is the half that makes it actually hermetic. A cache dir
 * with no recorded fetch is *stale*, not fresh, so without this every test in
 * this file paid a real network refresh per well-known marketplace — a cost that
 * grew with the list rather than staying at zero.
 */
function stubWellKnownMarketplaces(_cwd: string): void {
	const stamps: Record<string, string> = {};
	for (const wk of WELL_KNOWN_MARKETPLACES) {
		fs.mkdirSync(marketplaceCacheDir(wk.url), { recursive: true });
		stamps[wk.url] = new Date().toISOString();
	}
	const meta = marketplaceCacheMetaPath();
	fs.mkdirSync(path.dirname(meta), { recursive: true });
	fs.writeFileSync(meta, JSON.stringify(stamps, null, 2));
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
 * Seed a git-backed marketplace whose entry pins a sha *and* names a ref that
 * has since moved past it — the shape 83 entries of the official Claude
 * marketplace ship. Each commit writes its own name into `VERSION`, so which
 * one was checked out is visible on disk.
 */
function seedMovedTagMarketplace(cwd: string): { pinned: string; marketDir: string } {
	const repoDir = path.join(cwd, "repo");
	const marketDir = path.join(cwd, "market");

	fs.mkdirSync(repoDir, { recursive: true });
	writeJson(path.join(repoDir, ".agents-plugin", "plugin.json"), { name: "widget", version: "1.0.0" });
	fs.writeFileSync(path.join(repoDir, "VERSION"), "pinned\n");
	execInDir(repoDir, "git", ["init", "--quiet"]);
	execInDir(repoDir, "git", ["config", "user.email", "test@example.com"]);
	execInDir(repoDir, "git", ["config", "user.name", "Test"]);
	execInDir(repoDir, "git", ["add", "."]);
	execInDir(repoDir, "git", ["commit", "--quiet", "-m", "pinned"]);
	const pinnedSha = spawnSync("git", ["rev-parse", "HEAD"], { cwd: repoDir }).stdout.toString().trim();

	// The tag now points at a later commit than the catalog pinned.
	fs.writeFileSync(path.join(repoDir, "VERSION"), "moved\n");
	execInDir(repoDir, "git", ["add", "."]);
	execInDir(repoDir, "git", ["commit", "--quiet", "-m", "moved"]);
	execInDir(repoDir, "git", ["tag", "v1.0.0"]);

	writeJson(path.join(marketDir, ".agents-plugin", "marketplace.json"), {
		name: "pinned-market",
		plugins: [{ name: "widget", source: { source: "url", url: repoDir, ref: "v1.0.0", sha: pinnedSha } }],
	});
	writeJson(path.join(cwd, ".agents", "marketplaces.json"), {
		marketplaces: [{ location: marketDir, dir: marketDir }],
	});
	return { pinned: "pinned", marketDir };
}

/** Seed a marketplace whose entry name differs from the plugin's own manifest name. */
function seedNameMismatchMarketplace(cwd: string): void {
	const market = path.join(cwd, "market");
	writeJson(path.join(market, ".agents-plugin", "marketplace.json"), {
		name: "local",
		plugins: [{ name: "entry-name", source: "./plugins/thing" }],
	});
	writeJson(path.join(market, "plugins", "thing", ".claude-plugin", "plugin.json"), { name: "manifest-id" });
	fs.mkdirSync(path.join(market, "plugins", "thing", "skills", "s"), { recursive: true });
	fs.writeFileSync(
		path.join(market, "plugins", "thing", "skills", "s", "SKILL.md"),
		"---\nname: s\ndescription: does s\n---\n\nDo s.\n",
	);
	writeJson(path.join(cwd, ".agents", "marketplaces.json"), {
		marketplaces: [{ location: market, dir: market }],
	});
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

	it("checks out the pinned sha even when the entry also names a ref", async () => {
		// The failure this pins is silent: a tag that moved after the catalog
		// pinned it installs different code than the index vouches for, and
		// nothing in the outcome says so. Both vendors specify sha over ref.
		const { pinned } = seedMovedTagMarketplace(cwd);

		const outcome = await installAvailablePlugin(cwd, "widget");
		expect(outcome.installed).toBe(true);
		const marker = path.join(consumptionPluginsDir(), "widget", "VERSION");
		expect(fs.readFileSync(marker, "utf8").trim()).toBe(pinned);
	});

	it("keeps a plugin removable when its manifest id differs from the entry name", async () => {
		// Vendors allow the two names to differ. Installs are named for the entry
		// while discovery reports the manifest id, so resolving only one of them
		// left the plugin stranded: ListPlugins showed an id uninstall rejected.
		seedNameMismatchMarketplace(cwd);
		const outcome = await installAvailablePlugin(cwd, "entry-name");
		expect(outcome.installed).toBe(true);
		expect(outcome.id).toBe("manifest-id");
		expect(outcome.message).toContain("manifest-id");

		expect(listInstalledPlugins(cwd).map((p) => p.id)).toContain("manifest-id");
		// Installed under one name, listed under the other — both must resolve, or
		// InstallPlugin re-clones what is already there.
		expect(isPluginInstalled(cwd, "entry-name")).toBe(true);
		expect(isPluginInstalled(cwd, "manifest-id")).toBe(true);

		const removed = uninstallPlugin(cwd, "manifest-id");
		expect(removed.removed).toBe(true);
		expect(fs.existsSync(path.join(consumptionPluginsDir(), "entry-name"))).toBe(false);
	});

	it("never uninstalls a plugin the repository ships", () => {
		// `<cwd>/.claude/skills` is discovered but is committed repo content, not
		// something hoocode installed. Resolving uninstall by manifest id must not
		// turn UninstallPlugin into a way to delete files out of the working tree.
		const repoOwned = path.join(cwd, ".claude", "skills", "repo-owned");
		writeJson(path.join(repoOwned, ".claude-plugin", "plugin.json"), { name: "repo-owned" });

		const removed = uninstallPlugin(cwd, "repo-owned");
		expect(removed.removed).toBe(false);
		expect(fs.existsSync(repoOwned)).toBe(true);
	});

	it("installs into the user home by default and the working tree at project scope", async () => {
		seedLocalMarketplace(cwd);
		const userHome = path.join(consumptionPluginsDir(), "widget");
		const projectHome = path.join(cwd, ".agents", "plugins", "widget");

		// Default is user scope: an autonomous install must not put content into
		// the repo unless someone asked for that.
		const asUser = await installAvailablePlugin(cwd, "widget");
		expect(asUser.scope).toBe("user");
		expect(fs.existsSync(userHome)).toBe(true);
		expect(fs.existsSync(projectHome)).toBe(false);
		uninstallPlugin(cwd, "widget");

		const asProject = await installAvailablePlugin(cwd, "widget", undefined, { scope: "project" });
		expect(asProject.scope).toBe("project");
		expect(fs.existsSync(projectHome)).toBe(true);
		expect(fs.existsSync(userHome)).toBe(false);
		// Discovery already covered <cwd>/.agents/plugins, so project scope needs
		// no loader change to become live.
		expect(listInstalledPlugins(cwd).some((p) => p.id === "widget")).toBe(true);
		expect(isPluginInstalled(cwd, "widget")).toBe(true);

		// Uninstall reaches both homes, so a project-scoped plugin is as reversible
		// as a user-scoped one.
		expect(uninstallPlugin(cwd, "widget").removed).toBe(true);
		expect(fs.existsSync(projectHome)).toBe(false);
	});

	it("leaves no nested git repository behind, so a project-scoped install can be committed", async () => {
		// A cloned plugin that keeps its `.git` is an embedded repository once it
		// lands in the working tree: `git add` writes a gitlink instead of the
		// files, so the plugin cannot be committed — the one thing project scope is
		// for. Nothing reads the clone metadata either; installs are never updated
		// from their remote.
		seedMovedTagMarketplace(cwd);
		const outcome = await installAvailablePlugin(cwd, "widget", undefined, { scope: "project" });
		expect(outcome.installed).toBe(true);
		expect(fs.existsSync(path.join(outcome.dest as string, ".git"))).toBe(false);
		// The plugin itself survived the cleanup.
		expect(listInstalledPlugins(cwd).some((p) => p.id === "widget")).toBe(true);
	});

	it("lets a project-scoped plugin shadow a user-scoped one with the same id", async () => {
		seedLocalMarketplace(cwd);
		await installAvailablePlugin(cwd, "widget", undefined, { scope: "user" });
		await installAvailablePlugin(cwd, "widget", undefined, { scope: "project" });

		// defaultPluginDirs lists the project home first and discovery is first-wins,
		// which is what makes "this repo pins its own version" mean anything.
		const found = listInstalledPlugins(cwd).filter((p) => p.id === "widget");
		expect(found).toHaveLength(1);
		expect(found[0].root).toBe(path.join(cwd, ".agents", "plugins", "widget"));
	});

	it("warns that a project-scoped plugin's hooks run for whoever clones the repo", async () => {
		// hoocode has no workspace-trust gate for <cwd>/.agents/plugins, so this
		// is the only place a collaborator's exposure gets stated.
		const market = path.join(cwd, "market");
		writeJson(path.join(market, ".agents-plugin", "marketplace.json"), {
			name: "local",
			plugins: [{ name: "hooky", source: "./plugins/hooky" }],
		});
		writeJson(path.join(market, "plugins", "hooky", ".agents-plugin", "plugin.json"), { name: "hooky" });
		writeJson(path.join(market, "plugins", "hooky", "hooks", "hooks.json"), {
			hooks: { PreToolUse: [{ matcher: "Bash", hooks: [{ type: "command", command: "echo hi" }] }] },
		});
		writeJson(path.join(cwd, ".agents", "marketplaces.json"), {
			marketplaces: [{ location: market, dir: market }],
		});

		const asProject = await installAvailablePlugin(cwd, "hooky", undefined, { scope: "project" });
		expect(asProject.message).toContain("hooks");
		expect(asProject.message).toContain("anyone who clones the repository");

		uninstallPlugin(cwd, "hooky");
		// The same plugin at user scope reaches nobody else, so it says nothing.
		const asUser = await installAvailablePlugin(cwd, "hooky", undefined, { scope: "user" });
		expect(asUser.message).not.toContain("anyone who clones the repository");
	});

	it("says so when an installed plugin contributes nothing hoocode can load", async () => {
		// The dominant shape in github/awesome-copilot: a manifest whose content
		// lives in Copilot UI `extensions/`. Installing it is not a failure, but
		// reporting a bare success reads as a capability that is not there.
		const market = path.join(cwd, "market");
		writeJson(path.join(market, ".agents-plugin", "marketplace.json"), {
			name: "local",
			plugins: [{ name: "manifest-only", source: "./plugins/manifest-only" }],
		});
		writeJson(path.join(market, "plugins", "manifest-only", "plugin.json"), {
			name: "manifest-only",
			extensions: { "com.github.awesome-copilot": { extensions: ["./extensions/manifest-only"] } },
		});
		writeJson(path.join(cwd, ".agents", "marketplaces.json"), {
			marketplaces: [{ location: market, dir: market }],
		});

		const outcome = await installAvailablePlugin(cwd, "manifest-only");
		expect(outcome.installed).toBe(true);
		expect(outcome.message).toContain("no capabilities hoocode can load");
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
