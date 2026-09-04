/**
 * The artifact platform targets (`--platform` / the `platform` setting) as they
 * appear in the `/settings` pane.
 *
 * The setting was CLI- and file-only, which made it a per-invocation flag in
 * practice; the pane row is what turns it into a set-once-per-environment
 * choice. These tests reach into the settings list's items rather than rendering
 * the TUI - enough to catch the failure that matters, a row that is present but
 * routes its change nowhere, or a row nothing can reach.
 */

import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { SettingsManager } from "../src/core/settings-manager.js";
import { SettingsSelectorComponent } from "../src/modes/interactive/components/settings-selector.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

beforeAll(() => initTheme("dark"));

let tempDir = "";
afterEach(() => {
	if (tempDir) {
		rmSync(tempDir, { recursive: true, force: true });
		tempDir = "";
	}
});

/** The pane needs every setting; only `platform` matters here, so the rest are plausible defaults. */
function paneConfig(platform: string[] = []): any {
	return {
		autoCompact: true,
		tools: [],
		toolGroups: [],
		// Not the subject here; the External tools category is covered by its own suite.
		externalTools: [],
		flags: [],
		toolOutputView: "peek",
		toolOutputMaxBytes: 8192,
		toolOutputMaxLines: 200,
		contextGc: true,
		showImages: false,
		imageWidthCells: 80,
		autoResizeImages: true,
		blockImages: false,
		enableSkillCommands: true,
		pluginInstallScope: "user",
		enablePluginTools: false,
		projectPinnedSettings: [],
		platform,
		steeringMode: "all",
		followUpMode: "all",
		transport: "auto",
		thinkingLevel: "off",
		availableThinkingLevels: ["off"],
		currentTheme: "dark",
		availableThemes: ["dark"],
		hideThinkingBlock: false,
		collapseChangelog: false,
		enableInstallTelemetry: false,
		doubleEscapeAction: "tree",
		treeFilterMode: "default",
		showHardwareCursor: false,
		editorBorder: "box",
		editorPaddingX: 1,
		autocompleteMaxVisible: 10,
		quietStartup: false,
		clearOnShrink: false,
		showTerminalProgress: false,
		warnings: {},
		voiceSilenceMs: 800,
		webtoolsTimeoutSecs: 30,
		learn: {
			learnMaxSessions: 20,
			learnMaxAgeDays: 30,
			learnMinRepeats: 2,
			learnMinRequestRepeats: 3,
			learnMaxProposals: 8,
		},
	};
}

/** Open the Plugins category and hand back the rows inside it. */
function pluginRows(component: SettingsSelectorComponent): any {
	const topLevel = component.getSettingsList() as any;
	const category = topLevel.items.find((item: any) => item.id === "cat-plugins");
	expect(category, "Plugins category row").toBeDefined();
	return { category, submenu: category.submenu("", () => {}).settingsList };
}

/**
 * Every id reachable by opening rows from the top level down.
 *
 * Follows any submenu built on a settings list - categories, but also Tools,
 * Tool output and the platform toggles - so a row cannot hide from this by
 * moving one level deeper. The select-list submenus (theme, thinking) expose no
 * settings list and are left alone; their own row is already counted.
 */
function reachableIds(component: SettingsSelectorComponent): Set<string> {
	const ids = new Set<string>();
	const walk = (list: any) => {
		for (const item of list.items) {
			ids.add(item.id);
			if (!item.submenu) continue;
			const nested = item.submenu(item.currentValue, () => {})?.settingsList;
			if (nested) walk(nested);
		}
	};
	walk(component.getSettingsList() as any);
	return ids;
}

describe("platform settings pane", () => {
	it("gives the platform and plugin-scope rows a category to be reached from", () => {
		const { category, submenu } = pluginRows(new SettingsSelectorComponent(paneConfig(), {} as any));

		expect(category.label).toBe("Plugins");
		expect(submenu.items.map((item: any) => item.id)).toEqual(["plugin-tools", "platform", "plugin-install-scope"]);
	});

	it("leaves no setting stranded outside every category", () => {
		// `plugin-install-scope` shipped with a live callback and no category row,
		// so nothing in the pane could reach it. Nothing else may repeat that.
		const ids = reachableIds(new SettingsSelectorComponent(paneConfig(), {} as any));

		for (const id of [
			"autocompact",
			"context-gc",
			"light",
			"tools",
			"tool-output",
			"tool-output-view",
			"output-max-bytes",
			"output-max-lines",
			"platform",
			"plugin-tools",
			"plugin-install-scope",
			"steering-mode",
			"follow-up-mode",
			"thinking",
			"double-escape-action",
			"tree-filter-mode",
			"transport",
			"theme",
			"hide-thinking",
			"show-hardware-cursor",
			"editor-border",
			"editor-padding",
			"autocomplete-max-visible",
			"clear-on-shrink",
			"terminal-progress",
			"auto-resize-images",
			"block-images",
			"quiet-startup",
			"collapse-changelog",
			"install-telemetry",
			"skill-commands",
			"warnings",
			"voice-silence-ms",
			"webtools-timeout-secs",
		]) {
			expect(ids, `"${id}" reachable from the pane`).toContain(id);
		}
	});

	it("shows the fallback when nothing is configured, and the targets when they are", () => {
		const unset = pluginRows(new SettingsSelectorComponent(paneConfig(), {} as any));
		expect(unset.submenu.items.find((item: any) => item.id === "platform").currentValue).toBe("default (claude)");

		const set = pluginRows(new SettingsSelectorComponent(paneConfig(["github", "agents"]), {} as any));
		expect(set.submenu.items.find((item: any) => item.id === "platform").currentValue).toBe("github, agents");
	});

	it("toggles each target independently, since the setting is a list", () => {
		const changes: string[][] = [];
		const component = new SettingsSelectorComponent(paneConfig(["claude"]), {
			onPlatformChange: (platforms: any) => changes.push(platforms),
		} as any);

		const { submenu } = pluginRows(component);
		const row = submenu.items.find((item: any) => item.id === "platform");
		const platformList = row.submenu(row.currentValue, () => {}).settingsList;

		expect(platformList.items.map((item: any) => [item.id, item.currentValue])).toEqual([
			["claude", "on"],
			["github", "off"],
			["agents", "off"],
		]);

		platformList.onChange("github", "on");
		platformList.onChange("claude", "off");

		// Pane order, not toggle order, so the persisted list is stable.
		expect(changes).toEqual([["claude", "github"], ["github"]]);
	});

	it("reports an empty selection as unset rather than as targeting nothing", () => {
		const changes: string[][] = [];
		let summary: string | undefined;
		const component = new SettingsSelectorComponent(paneConfig(["claude"]), {
			onPlatformChange: (platforms: any) => changes.push(platforms),
		} as any);

		const { submenu } = pluginRows(component);
		const row = submenu.items.find((item: any) => item.id === "platform");
		const platformList = row.submenu(row.currentValue, (value: string) => {
			summary = value;
		}).settingsList;

		platformList.onChange("claude", "off");
		platformList.onCancel();

		expect(changes).toEqual([[]]);
		expect(summary).toBe("default (claude)");
	});
});

describe("the plugin system master switch", () => {
	it("routes the row to the setting that gates the tools and the nudge", () => {
		const changes: boolean[] = [];
		const component = new SettingsSelectorComponent(paneConfig(), {
			onEnablePluginToolsChange: (enabled: any) => changes.push(enabled),
		} as any);

		const { submenu } = pluginRows(component);
		expect(submenu.items.find((item: any) => item.id === "plugin-tools").currentValue).toBe("false");

		submenu.onChange("plugin-tools", "true");
		expect(changes).toEqual([true]);
	});
});

describe("setPlatform", () => {
	it("persists the targets to the user settings.json that the next session reads", async () => {
		tempDir = mkdtempSync(join(tmpdir(), "hoo-platform-settings-"));
		const manager = SettingsManager.create(tempDir, tempDir);

		manager.setPlatform(["github", "claude"]);
		expect(manager.getPlatform()).toEqual(["github", "claude"]);

		await manager.flush();
		expect(JSON.parse(readFileSync(join(tempDir, "settings.json"), "utf-8")).platform).toEqual(["github", "claude"]);
		expect(SettingsManager.create(tempDir, tempDir).getPlatform()).toEqual(["github", "claude"]);
	});

	it("removes the key on an empty selection so the per-consumer defaults come back", async () => {
		tempDir = mkdtempSync(join(tmpdir(), "hoo-platform-settings-"));
		const manager = SettingsManager.create(tempDir, tempDir);

		manager.setPlatform(["github"]);
		manager.setPlatform([]);

		expect(manager.getPlatform()).toBeUndefined();
		await manager.flush();
		expect(JSON.parse(readFileSync(join(tempDir, "settings.json"), "utf-8"))).not.toHaveProperty("platform");
	});
});

describe("settings pane search", () => {
	it("finds a setting through the category that holds it", () => {
		// Categories shortened the top level but hid every leaf from its search:
		// "theme" is a row inside Interface, and matched nothing at the top.
		const list = new SettingsSelectorComponent(paneConfig(), {} as any).getSettingsList() as any;

		list.applyFilter("theme");
		expect(list.filteredItems.map((item: any) => item.id)).toContain("cat-interface");

		list.applyFilter("platform");
		expect(list.filteredItems.map((item: any) => item.id)).toContain("cat-plugins");
	});
});
