/**
 * The `/learn` thresholds as they appear in the `/settings` pane.
 *
 * These settings existed with no UI and no discoverable home, so the pane row is
 * the fix; these tests hold the wiring in place. They reach into the settings
 * list's items rather than rendering the TUI, which is enough to catch the
 * failure that matters — a row that is present but routes its change nowhere.
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

const DEFAULT_LEARN = {
	learnMaxSessions: 20,
	learnMaxAgeDays: 30,
	learnMinRepeats: 2,
	learnMinRequestRepeats: 3,
	learnMaxProposals: 8,
};

/** The pane needs every setting; only `learn` matters here, so the rest are plausible defaults. */
function paneConfig(learn: Record<string, number> = DEFAULT_LEARN): any {
	return {
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
		pluginInstallScope: "user",
		enablePluginTools: false,
		projectPinnedSettings: [],
		platform: [],
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
		learn,
	};
}

/** Open the Learning category and hand back the rows inside it. */
function learningRows(component: SettingsSelectorComponent): any {
	const topLevel = component.getSettingsList() as any;
	const category = topLevel.items.find((item: any) => item.id === "cat-learn");
	expect(category, "Learning category row").toBeDefined();
	return { category, submenu: category.submenu("", () => {}).settingsList };
}

describe("learning settings pane", () => {
	it("offers the thresholds as a top-level category, not buried under Advanced", () => {
		const { category, submenu } = learningRows(new SettingsSelectorComponent(paneConfig(), {} as any));

		expect(category.label).toBe("Learning");
		expect(category.currentValue).toBe("5 settings");
		expect(submenu.items.map((item: any) => item.id)).toEqual(Object.keys(DEFAULT_LEARN));
	});

	it("routes a change to the settings key the row is named for", () => {
		const changes: Array<[string, number]> = [];
		const component = new SettingsSelectorComponent(paneConfig(), {
			onLearnSettingChange: (key: any, value: any) => changes.push([key, value]),
		} as any);

		const { submenu } = learningRows(component);
		submenu.onChange("learnMaxAgeDays", "90");
		submenu.onChange("learnMinRepeats", "4");

		expect(changes).toEqual([
			["learnMaxAgeDays", 90],
			["learnMinRepeats", 4],
		]);
	});

	it("keeps a hand-edited value in the cycle rather than snapping away from it", () => {
		// A value set directly in settings.json is not one of the presets. Dropping
		// it would mean one keypress silently narrowing the window.
		const { submenu } = learningRows(
			new SettingsSelectorComponent(paneConfig({ ...DEFAULT_LEARN, learnMaxAgeDays: 45 }), {} as any),
		);

		const row = submenu.items.find((item: any) => item.id === "learnMaxAgeDays");
		expect(row.currentValue).toBe("45");
		expect(row.values).toEqual(["7", "14", "30", "45", "60", "90", "180"]);
	});
});

describe("setLearnSetting", () => {
	it("persists to the user settings.json that /learn re-reads", async () => {
		tempDir = mkdtempSync(join(tmpdir(), "hoo-learn-settings-"));
		const manager = SettingsManager.create(tempDir, tempDir);

		manager.setLearnSetting("learnMaxAgeDays", 90);
		expect(manager.getLearnSettings().maxAgeDays).toBe(90);

		// Writes are queued, so flush before reading back the way `/learn` does —
		// through a fresh manager, since it never shares the TUI's instance.
		await manager.flush();
		expect(JSON.parse(readFileSync(join(tempDir, "settings.json"), "utf-8")).learnMaxAgeDays).toBe(90);
		expect(SettingsManager.create(tempDir, tempDir).getLearnSettings().maxAgeDays).toBe(90);
	});

	it("refuses a value that would shrink the window to nothing", () => {
		tempDir = mkdtempSync(join(tmpdir(), "hoo-learn-settings-"));
		const manager = SettingsManager.create(tempDir, tempDir);

		manager.setLearnSetting("learnMaxSessions", 0);

		expect(manager.getLearnSettings().maxSessions).toBe(1);
	});
});
