/**
 * The Plugins pane end to end: real keystrokes in, settings.json on disk out,
 * and the startup wiring that reads it back.
 *
 * The other pane tests call the change handler directly, which proves the
 * routing but not that a key ever reaches it. This one drives the component the
 * way a person does - arrows, Enter, Escape - through the same SettingsManager
 * the TUI holds, then re-reads the file the next session would read and runs
 * main.ts's platform wiring over it. What it is really asserting is that the two
 * halves meet: the pane writes exactly the keys `--platform` and
 * `enablePluginTools` are documented to read.
 */

import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import {
	getWorkspacePlatforms,
	parsePlatforms,
	resolvePluginPlatforms,
	setPlatforms,
} from "../src/core/extensions/plugins/formats/platform-targets.js";
import { SettingsManager } from "../src/core/settings-manager.js";
import { SettingsSelectorComponent } from "../src/modes/interactive/components/settings-selector.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

beforeAll(() => initTheme("dark"));

const DOWN = "\x1b[B";
const ENTER = "\r";
const ESC = "\x1b";

let tempDir = "";
beforeEach(() => {
	tempDir = mkdtempSync(join(tmpdir(), "hoo-plugin-pane-"));
});
afterEach(() => {
	rmSync(tempDir, { recursive: true, force: true });
	tempDir = "";
	setPlatforms(undefined);
});

/** The pane as interactive-mode builds it, over a real settings manager. */
function openPane(manager: SettingsManager): SettingsSelectorComponent {
	const config: any = {
		autoCompact: true,
		tools: [],
		toolGroups: [],
		// Not the subject here; the External tools category is covered by its own suite.
		externalTools: [],
		flags: [],
		toolOutputDisplay: "standard",
		toolOutputMaxBytes: 8192,
		toolOutputMaxLines: 200,
		contextGc: true,
		showImages: false,
		imageWidthCells: 80,
		autoResizeImages: true,
		blockImages: false,
		enableSkillCommands: true,
		pluginInstallScope: manager.getPluginInstallScope(),
		enablePluginTools: manager.getEnablePluginTools(),
		projectPinnedSettings: Object.keys(manager.getProjectSettings()),
		platform: getWorkspacePlatforms() ?? [],
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
	const callbacks: any = {
		onEnablePluginToolsChange: (enabled: boolean) => manager.setEnablePluginTools(enabled),
		onPluginInstallScopeChange: (scope: any) => manager.setPluginInstallScope(scope),
		onPlatformChange: (platforms: any) => {
			manager.setPlatform(platforms);
			setPlatforms(platforms);
		},
		onCancel: () => {},
	};
	return new SettingsSelectorComponent(config, callbacks);
}

/**
 * The component the TUI actually gives the keyboard to. `showSelector` focuses
 * `getSettingsList()`, not the wrapper, so that is what a keystroke reaches.
 */
function keyboard(component: SettingsSelectorComponent): { press: (key: string) => void; items: any[] } {
	const list = component.getSettingsList() as any;
	return { press: (key: string) => list.handleInput(key), items: list.items };
}

/** Walk to a top-level row by id with arrow keys, then open it. */
function openCategory(component: SettingsSelectorComponent, id: string): (key: string) => void {
	const { press, items } = keyboard(component);
	const index = items.findIndex((item: any) => item.id === id);
	expect(index, `top-level row ${id}`).toBeGreaterThanOrEqual(0);
	for (let i = 0; i < index; i++) press(DOWN);
	press(ENTER);
	return press;
}

describe("the Plugins pane, driven by keystrokes", () => {
	it("writes the settings a later session reads", async () => {
		const manager = SettingsManager.create(tempDir, tempDir);
		const component = openPane(manager);

		const press = openCategory(component, "cat-plugins");
		// Row 1 of the category: the master switch, off -> on.
		press(ENTER);
		// Row 2: the platform targets, whose submenu toggles claude then github.
		press(DOWN);
		press(ENTER);
		press(ENTER);
		press(DOWN);
		press(ENTER);
		press(ESC);

		await manager.flush();

		const onDisk = JSON.parse(readFileSync(join(tempDir, "settings.json"), "utf-8"));
		expect(onDisk.enablePluginTools).toBe(true);
		expect(onDisk.platform).toEqual(["claude", "github"]);
	});

	it("hands the next session targets its startup wiring can use", async () => {
		const manager = SettingsManager.create(tempDir, tempDir);
		const component = openPane(manager);

		const press = openCategory(component, "cat-plugins");
		press(DOWN);
		press(ENTER);
		press(DOWN); // github
		press(ENTER);
		press(ESC);
		await manager.flush();

		// Applied to the running session already, without a restart.
		expect(getWorkspacePlatforms()).toEqual(["github"]);
		expect(resolvePluginPlatforms()).toEqual(["github"]);

		// And what the next session does with the file: main.ts reads the setting,
		// parses the tokens, and installs them as the session targets.
		setPlatforms(undefined);
		const nextSession = SettingsManager.create(tempDir, tempDir);
		const { platforms, invalid } = parsePlatforms(nextSession.getPlatform() ?? []);
		expect(invalid).toEqual([]);
		setPlatforms(platforms);

		expect(getWorkspacePlatforms()).toEqual(["github"]);
		expect(resolvePluginPlatforms()).toEqual(["github"]);
		expect(nextSession.getEnablePluginTools()).toBe(false);
	});

	it("says so when the project file pins the key the row writes", () => {
		// The row writes the user file; a project .hoocode/settings.json is merged
		// over it, so without this the row looks like it took and silently reverts.
		mkdirSync(join(tempDir, ".hoocode"), { recursive: true });
		writeFileSync(
			join(tempDir, ".hoocode", "settings.json"),
			JSON.stringify({ platform: ["claude"], enablePluginTools: true }),
		);

		const component = openPane(SettingsManager.create(tempDir, tempDir));
		const category = (component.getSettingsList() as any).items.find((item: any) => item.id === "cat-plugins");
		const rows = category.submenu("", () => {}).settingsList.items;

		expect(rows.find((row: any) => row.id === "platform").description).toContain("overrides this row");
		expect(rows.find((row: any) => row.id === "plugin-tools").description).toContain("overrides this row");
		expect(rows.find((row: any) => row.id === "plugin-install-scope").description).not.toContain(
			"overrides this row",
		);
	});

	it("clears the key when every target is switched back off", async () => {
		const manager = SettingsManager.create(tempDir, tempDir);
		manager.setPlatform(["claude"]);
		setPlatforms(["claude"]);

		const component = openPane(manager);
		const press = openCategory(component, "cat-plugins");
		press(DOWN);
		press(ENTER);
		press(ENTER); // claude: on -> off
		press(ESC);
		await manager.flush();

		expect(JSON.parse(readFileSync(join(tempDir, "settings.json"), "utf-8"))).not.toHaveProperty("platform");
		expect(SettingsManager.create(tempDir, tempDir).getPlatform()).toBeUndefined();
		// Unset is not "target nothing": the plugin default comes back.
		expect(resolvePluginPlatforms()).toEqual(["claude"]);
	});
});
