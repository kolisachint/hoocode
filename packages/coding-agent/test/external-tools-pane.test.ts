/**
 * The external Rust binaries (rg, fd, embsearch, webtools, voicetools) as they
 * appear in the `/settings` pane.
 *
 * These binaries were shipped but never surfaced: hoocode degrades quietly
 * without each of them, so nothing ever told a user they existed or what they
 * would add. The pane category is the fix, and these tests hold the two
 * properties that make it worth having - every binary is listed, and a row whose
 * setting is inert without a binary says so instead of looking functional.
 */

import { beforeAll, describe, expect, it } from "vitest";
import type { ExternalToolStatus } from "../src/core/external-tools.js";
import { buildRowGates, describeExternalTools, EXTERNAL_TOOLS, statusLabel } from "../src/core/external-tools.js";
import { SettingsSelectorComponent } from "../src/modes/interactive/components/settings-selector.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

beforeAll(() => initTheme("dark"));

/** A status with the binary deliberately absent, so the gated-row paths are exercised. */
function missing(tool: ExternalToolStatus["tool"]): ExternalToolStatus {
	const doc = EXTERNAL_TOOLS.find((entry) => entry.tool === tool);
	if (!doc) throw new Error(`no catalog entry for ${tool}`);
	return {
		...doc,
		name: tool,
		repo: doc.tool === "rg" ? "BurntSushi/ripgrep" : "kolisachint/x",
		overrideEnv: `HOOCODE_${tool.toUpperCase()}_BINARY`,
		path: null,
		source: null,
		installed: false,
		downloadable: true,
	};
}

function present(tool: ExternalToolStatus["tool"]): ExternalToolStatus {
	return { ...missing(tool), path: `/bin/${tool}`, source: "path", installed: true };
}

function paneConfig(externalTools: ExternalToolStatus[]): any {
	return {
		autoCompact: true,
		tools: [],
		toolGroups: [
			{ id: "web", label: "Web tools", description: "webfetch + websearch.", enabled: false },
			{ id: "embsearch", label: "Semantic search", description: "Semantic index.", enabled: true },
		],
		externalTools,
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
	};
}

const noopCallbacks: any = new Proxy({}, { get: () => () => {} });

function buildPane(externalTools: ExternalToolStatus[]) {
	return new SettingsSelectorComponent(paneConfig(externalTools), noopCallbacks);
}

function items(pane: ReturnType<typeof buildPane>): any[] {
	return (pane.getSettingsList() as any).items;
}

/** Open a submenu row and return the rows it holds. */
function submenuItems(row: any): any[] {
	return (row.submenu("", () => {}) as any).settingsList.items;
}

/**
 * Leaf rows live inside category submenus, so a test for one has to open the
 * category that holds it. Walks one level, which is all the pane has.
 */
function leaf(pane: ReturnType<typeof buildPane>, id: string): any {
	for (const row of items(pane)) {
		if (row.id === id) return row;
		if (!row.submenu) continue;
		const found = submenuItems(row).find((child: any) => child.id === id);
		if (found) return found;
	}
	throw new Error(`no row ${id} in the pane`);
}

describe("external tools catalog", () => {
	it("documents every binary the tools manager can resolve", () => {
		// The catalog is the only place that explains these binaries. A tool the
		// manager knows about but the catalog does not is invisible all over again.
		expect(EXTERNAL_TOOLS.map((doc) => doc.tool).sort()).toEqual(
			["embsearch", "fd", "rg", "voicetools", "webtools"].sort(),
		);
	});

	it("gives every binary a fallback, because none of them is required", () => {
		for (const doc of EXTERNAL_TOOLS) {
			expect(doc.fallback.length, doc.tool).toBeGreaterThan(0);
			expect(doc.enables.length, doc.tool).toBeGreaterThan(0);
		}
	});

	it("resolves live status without downloading anything", () => {
		const statuses = describeExternalTools();
		expect(statuses).toHaveLength(EXTERNAL_TOOLS.length);
		for (const status of statuses) {
			expect(status.installed).toBe(status.path !== null);
			expect(statusLabel(status).length).toBeGreaterThan(0);
		}
	});

	it("distinguishes where an installed binary came from", () => {
		expect(statusLabel(present("rg"))).toBe("system");
		expect(statusLabel({ ...present("rg"), source: "managed" })).toBe("installed");
		expect(statusLabel({ ...present("rg"), source: "override" })).toBe("env override");
		expect(statusLabel(missing("rg"))).toBe("not installed");
		expect(statusLabel({ ...missing("rg"), downloadable: false })).toBe("unavailable");
	});

	it("maps gated rows back to the binary they need", () => {
		const gates = buildRowGates([missing("webtools"), missing("voicetools"), missing("embsearch")]);
		expect(gates.get("group:web")?.tool).toBe("webtools");
		expect(gates.get("webtools-timeout-secs")?.tool).toBe("webtools");
		expect(gates.get("voice-silence-ms")?.tool).toBe("voicetools");
		expect(gates.get("group:embsearch")?.tool).toBe("embsearch");
		// rg/fd gate nothing: their absence changes speed, not what a setting does.
		expect(gates.has("tools")).toBe(false);
	});
});

describe("external tools in the settings pane", () => {
	it("puts the category at the top level with a count", () => {
		const row = items(buildPane([present("rg"), missing("webtools")])).find((item) => item.id === "cat-external");
		expect(row).toBeDefined();
		expect(row.currentValue).toBe("1 of 2 installed");
		// Searching for a binary by name must land on the category that holds it.
		expect(row.keywords).toContain("webtools");
	});

	it("lists one row per binary, each opening a detail submenu", () => {
		const pane = buildPane(describeExternalTools());
		const category = items(pane).find((item) => item.id === "cat-external");
		const rows = submenuItems(category);
		expect(rows.map((row: any) => row.id)).toEqual(EXTERNAL_TOOLS.map((doc) => `ext-${doc.tool}`));
		// Status rows are readouts of the machine, not settings: nothing to cycle.
		for (const row of rows) expect(row.values).toBeUndefined();
		expect(rows[0].submenu).toBeTypeOf("function");
	});

	it("marks a gated row when its binary is missing, and leaves it settable", () => {
		const voice = leaf(buildPane([missing("voicetools")]), "voice-silence-ms");
		expect(voice.valueSuffix).toBe("needs voicetools");
		expect(voice.description).toContain("voicetools");
		// The setting still cycles: it is what the feature reads once the binary
		// lands, so freezing the row would strand it.
		expect(voice.values.length).toBeGreaterThan(1);
	});

	it("leaves a gated row unmarked when its binary is present", () => {
		const web = leaf(buildPane([present("webtools")]), "webtools-timeout-secs");
		expect(web.valueSuffix).toBeUndefined();
		expect(web.description).not.toContain("Needs the");
	});

	it("marks the tool-group switches whose tools cannot run yet", () => {
		const pane = buildPane([missing("webtools"), present("embsearch")]);
		const tools = items(pane).find((item) => item.id === "tools");
		const rows = submenuItems(tools);
		const web = rows.find((row: any) => row.id === "group:web");
		const semantic = rows.find((row: any) => row.id === "group:embsearch");
		expect(web.valueSuffix).toBe("needs webtools");
		expect(web.description).toContain("fetches the first time");
		expect(semantic.valueSuffix).toBeUndefined();
	});

	it("says the binary will not arrive when the environment cannot fetch it", () => {
		const offline = { ...missing("webtools"), downloadable: false };
		const web = leaf(buildPane([offline]), "webtools-timeout-secs");
		expect(web.description).toContain("will not fetch it");
	});
});
