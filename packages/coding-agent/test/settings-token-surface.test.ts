/**
 * What `/settings` tells you a change costs.
 *
 * Everything in the system prompt and in an active tool's schema is re-sent on
 * every request, and the pane is where that is decided - so the pane prices it:
 * a line under the list for the whole surface, and a per-tool price beside each
 * switch. The prices have to move when a toggle moves them, which is what these
 * tests hold in place, along with the grouping that keeps the token-budget
 * settings together.
 */

import { beforeAll, describe, expect, it } from "vitest";
import type { PromptSurface } from "../src/core/light.js";
import { measurePromptSurface, measureToolSchemaTokens } from "../src/core/light.js";
import { SettingsSelectorComponent } from "../src/modes/interactive/components/settings-selector.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

beforeAll(() => initTheme("dark"));

const SURFACE: PromptSurface = {
	systemPromptTokens: 1430,
	toolSchemaTokens: 2710,
	totalTokens: 4140,
	tools: [
		{ name: "read", tokens: 162 },
		{ name: "bash", tokens: 128 },
	],
};

function paneConfig(overrides: Record<string, unknown> = {}): any {
	return {
		autoCompact: true,
		tools: [
			{ name: "read", enabled: true, tokens: 162 },
			{ name: "ProposePlugin", enabled: false, tokens: 849 },
			{ name: "websearch", enabled: true },
		],
		toolGroups: [],
		// Not the subject here; the External tools category is covered by its own suite.
		externalTools: [],
		flags: [],
		toolOutputView: "glance",
		toolOutputMaxBytes: 8192,
		toolOutputMaxLines: 200,
		contextGc: true,
		showImages: false,
		imageWidthCells: 80,
		autoResizeImages: true,
		blockImages: false,
		enableSkillCommands: true,
		light: false,
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
		learn: {
			learnMaxSessions: 20,
			learnMaxAgeDays: 30,
			learnMinRepeats: 2,
			learnMinRequestRepeats: 3,
			learnMaxProposals: 8,
		},
		...overrides,
	};
}

/** Everything the pane renders, ANSI stripped. */
function rendered(component: SettingsSelectorComponent): string {
	return component
		.render(100)
		.join("\n")
		.replace(/\x1b\[[0-9;]*m/g, "");
}

describe("the per-turn surface, shown in the pane", () => {
	it("prices the whole surface under the list, exactly rather than rounded", () => {
		const pane = new SettingsSelectorComponent(paneConfig({ measureTokenSurface: () => SURFACE }), {} as any);

		expect(rendered(pane)).toContain(
			"Per-turn surface: 4,140 tokens  (1,430 system prompt + 2,710 schemas, 2 tools)",
		);
	});

	it("re-prices after a change, since a tool toggle moves the surface at once", () => {
		let surface = SURFACE;
		const pane = new SettingsSelectorComponent(paneConfig({ measureTokenSurface: () => surface }), {
			onAutoCompactChange: () => {},
		} as any);

		surface = { ...SURFACE, toolSchemaTokens: 1861, totalTokens: 3291, tools: [{ name: "read", tokens: 162 }] };
		(pane.getSettingsList() as any).onChange("autocompact", "false");

		expect(rendered(pane)).toContain("Per-turn surface: 3,291 tokens");
	});

	it("says nothing at all when there is no session to measure", () => {
		// The pane is exported, and a caller without a live session should get the
		// settings rather than a line reporting zero tokens.
		expect(rendered(new SettingsSelectorComponent(paneConfig(), {} as any))).not.toContain("Per-turn surface");
	});

	it("prices each tool beside its switch, including the ones that are off", () => {
		const pane = new SettingsSelectorComponent(paneConfig({ measureTokenSurface: () => SURFACE }), {} as any);
		const tools = (pane.getSettingsList() as any).items.find((item: any) => item.id === "tools");

		expect(tools.valueSuffix).toBe("2,710 tok/turn");

		const rows = tools.submenu(tools.currentValue, () => {}).settingsList.items;
		// What a disabled tool costs is what turning it back on will cost.
		expect(rows.find((row: any) => row.id === "ProposePlugin").valueSuffix).toBe("849 tok/turn");
		expect(rows.find((row: any) => row.id === "read").valueSuffix).toBe("162 tok/turn");
		// A tool disabled before launch has no schema this session, so no price.
		expect(rows.find((row: any) => row.id === "websearch").valueSuffix).toBeUndefined();
	});

	it("prices a tool the way the session does", () => {
		// The pane must not invent its own estimate: same helper, same number as
		// --print-token-surface reports for the same tool.
		const tool = { name: "read", description: "Read a file", parameters: { type: "object" } } as any;
		const session = {
			systemPrompt: "sys",
			getActiveToolNames: () => ["read"],
			getAllTools: () => [tool],
		};

		expect(measureToolSchemaTokens(tool)).toBe(measurePromptSurface(session).tools[0].tokens);
	});
});

describe("pane grouping", () => {
	it("keeps the token-budget settings in one place", () => {
		const pane = new SettingsSelectorComponent(paneConfig(), {} as any);
		const context = (pane.getSettingsList() as any).items.find((item: any) => item.id === "cat-context");

		expect(context.label).toBe("Context");
		expect(context.submenu("", () => {}).settingsList.items.map((item: any) => item.id)).toEqual([
			"autocompact",
			"context-gc",
			"light",
		]);
	});

	it("opens something from every top-level row", () => {
		// A bare leaf at the top level is how `plugin-install-scope` came to be
		// orphaned: rows that cycle in place read as category rows that do nothing.
		const items = (new SettingsSelectorComponent(paneConfig(), {} as any).getSettingsList() as any).items;

		for (const item of items) {
			expect(item.submenu, `top-level row "${item.id}" opens a submenu`).toBeDefined();
		}
	});

	it("routes the light preset to its setting", () => {
		const changes: boolean[] = [];
		const pane = new SettingsSelectorComponent(paneConfig(), {
			onLightChange: (enabled: any) => changes.push(enabled),
		} as any);

		const context = (pane.getSettingsList() as any).items.find((item: any) => item.id === "cat-context");
		context.submenu("", () => {}).settingsList.onChange("light", "true");

		expect(changes).toEqual([true]);
	});
});
