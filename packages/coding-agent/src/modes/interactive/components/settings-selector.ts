import type { ThinkingLevel } from "@kolisachint/hoocode-agent-core";
import type { Transport } from "@kolisachint/hoocode-ai";
import {
	Container,
	getCapabilities,
	Input,
	type SelectItem,
	SelectList,
	type SelectListLayoutOptions,
	type SettingItem,
	SettingsList,
	Spacer,
	Text,
} from "@kolisachint/hoocode-tui";
import type { MarketplacePlatform } from "../../../core/extensions/plugins/formats/types.js";
import type { PromptSurface } from "../../../core/light.js";
import type { LearnSettingKey, WarningSettings } from "../../../core/settings-manager.js";
import { getSelectListTheme, getSettingsListTheme, getThemeDescription, theme } from "../theme/theme.js";
import { DynamicBorder } from "./dynamic-border.js";
import { keyDisplayText } from "./keybinding-hints.js";

const SETTINGS_SUBMENU_SELECT_LIST_LAYOUT: SelectListLayoutOptions = {
	minPrimaryColumnWidth: 12,
	maxPrimaryColumnWidth: 32,
};

const THINKING_DESCRIPTIONS: Record<ThinkingLevel, string> = {
	off: "No reasoning",
	minimal: "Very brief reasoning (~1k tokens)",
	low: "Light reasoning (~2k tokens)",
	medium: "Moderate reasoning (~8k tokens)",
	high: "Deep reasoning (~16k tokens)",
	xhigh: "Maximum reasoning (~32k tokens)",
};

interface ToolToggleInfo {
	/** Tool name (e.g. "read", "bash"). */
	name: string;
	/** Whether the tool is currently enabled (not in the persisted disabled set). */
	enabled: boolean;
	/**
	 * What this tool's serialized schema costs on every request. Priced whether or
	 * not it is on: for a tool that is off, this is what turning it on will cost.
	 * Undefined for a tool that was disabled before launch, whose schema this
	 * session never built.
	 */
	tokens?: number;
}

interface FlagInfo {
	/** Flag name (without the leading --). */
	name: string;
	description?: string;
	type: "boolean" | "string";
	/** Current effective value. */
	value: boolean | string;
}

interface ToolGroupInfo {
	/** Group identifier (e.g. "web", "embsearch"). */
	id: string;
	label: string;
	description: string;
	/** Whether the group is currently enabled (its tools are available). */
	enabled: boolean;
}

export interface SettingsConfig {
	autoCompact: boolean;
	tools: ToolToggleInfo[];
	toolGroups: ToolGroupInfo[];
	flags: FlagInfo[];
	toolOutputDisplay: "collapsed" | "peek" | "standard";
	toolOutputMaxBytes: number;
	toolOutputMaxLines: number;
	contextGc: boolean;
	showImages: boolean;
	imageWidthCells: number;
	autoResizeImages: boolean;
	blockImages: boolean;
	enableSkillCommands: boolean;
	light: boolean;
	pluginInstallScope: "user" | "project";
	enablePluginTools: boolean;
	/**
	 * settings.json keys the project file sets. A pane row writes user scope, so
	 * anything listed here wins again on the next session however the row is set.
	 */
	projectPinnedSettings: string[];
	/** Platform layouts in force this session (empty = unset, per-consumer defaults apply). */
	platform: MarketplacePlatform[];
	steeringMode: "all" | "one-at-a-time";
	followUpMode: "all" | "one-at-a-time";
	transport: Transport;
	thinkingLevel: ThinkingLevel;
	availableThinkingLevels: ThinkingLevel[];
	currentTheme: string;
	availableThemes: string[];
	hideThinkingBlock: boolean;
	collapseChangelog: boolean;
	enableInstallTelemetry: boolean;
	doubleEscapeAction: "fork" | "tree" | "none";
	treeFilterMode: "default" | "no-tools" | "user-only" | "labeled-only" | "all";
	showHardwareCursor: boolean;
	editorBorder: "rule" | "box";
	editorPaddingX: number;
	autocompleteMaxVisible: number;
	quietStartup: boolean;
	clearOnShrink: boolean;
	showTerminalProgress: boolean;
	warnings: WarningSettings;
	voiceSilenceMs: number;
	webtoolsTimeoutSecs: number;
	learn: Record<LearnSettingKey, number>;
	/**
	 * Re-measure the fixed per-turn surface (system prompt + active tool schemas).
	 * Called on open and after every change, so a toggle that costs tokens shows
	 * what it cost. Omitted by callers that have no session to measure.
	 */
	measureTokenSurface?: () => PromptSurface;
}

export interface SettingsCallbacks {
	onAutoCompactChange: (enabled: boolean) => void;
	onToolEnabledChange: (name: string, enabled: boolean) => void;
	onToolGroupChange: (id: string, enabled: boolean) => void;
	onToolOutputDisplayChange: (level: "collapsed" | "peek" | "standard") => void;
	onToolOutputMaxBytesChange: (bytes: number) => void;
	onToolOutputMaxLinesChange: (lines: number) => void;
	onContextGcChange: (enabled: boolean) => void;
	onFlagChange: (name: string, value: boolean | string) => void;
	onShowImagesChange: (enabled: boolean) => void;
	onImageWidthCellsChange: (width: number) => void;
	onAutoResizeImagesChange: (enabled: boolean) => void;
	onBlockImagesChange: (blocked: boolean) => void;
	onEnableSkillCommandsChange: (enabled: boolean) => void;
	onLightChange: (enabled: boolean) => void;
	onPluginInstallScopeChange: (scope: "user" | "project") => void;
	onEnablePluginToolsChange: (enabled: boolean) => void;
	onPlatformChange: (platforms: MarketplacePlatform[]) => void;
	onSteeringModeChange: (mode: "all" | "one-at-a-time") => void;
	onFollowUpModeChange: (mode: "all" | "one-at-a-time") => void;
	onTransportChange: (transport: Transport) => void;
	onThinkingLevelChange: (level: ThinkingLevel) => void;
	onThemeChange: (theme: string) => void;
	onThemePreview?: (theme: string) => void;
	onHideThinkingBlockChange: (hidden: boolean) => void;
	onCollapseChangelogChange: (collapsed: boolean) => void;
	onEnableInstallTelemetryChange: (enabled: boolean) => void;
	onDoubleEscapeActionChange: (action: "fork" | "tree" | "none") => void;
	onTreeFilterModeChange: (mode: "default" | "no-tools" | "user-only" | "labeled-only" | "all") => void;
	onShowHardwareCursorChange: (enabled: boolean) => void;
	onEditorBorderChange: (border: "rule" | "box") => void;
	onEditorPaddingXChange: (padding: number) => void;
	onAutocompleteMaxVisibleChange: (maxVisible: number) => void;
	onQuietStartupChange: (enabled: boolean) => void;
	onClearOnShrinkChange: (enabled: boolean) => void;
	onShowTerminalProgressChange: (enabled: boolean) => void;
	onWarningsChange: (warnings: WarningSettings) => void;
	onVoiceSilenceMsChange: (ms: number) => void;
	onWebtoolsTimeoutSecsChange: (secs: number) => void;
	onLearnSettingChange: (key: LearnSettingKey, value: number) => void;
	onCancel: () => void;
}

/**
 * The `/learn` thresholds as pane rows: the presets to cycle through, and what
 * each one buys. Kept as a table because all five are the same shape — a
 * positive integer with a handful of sensible values — and the pane, the change
 * handler and the category row all read from it rather than repeating the list.
 */
const LEARN_SETTINGS: ReadonlyArray<{
	key: LearnSettingKey;
	label: string;
	description: string;
	presets: number[];
}> = [
	{
		key: "learnMaxSessions",
		label: "Sessions scanned",
		description: "How many recent sessions in this directory /learn mines. Raise it on a repo you touch rarely.",
		presets: [10, 20, 30, 50, 100],
	},
	{
		key: "learnMaxAgeDays",
		label: "Session age limit",
		description: "Ignore sessions older than this many days. A pattern that stopped is not a rule.",
		presets: [7, 14, 30, 60, 90, 180],
	},
	{
		key: "learnMinRepeats",
		label: "Directive repeats",
		description:
			"Times a directive must recur before it is proposed. The signal/noise dial: raise it for fewer, better-evidenced proposals.",
		presets: [2, 3, 4, 5],
	},
	{
		key: "learnMinRequestRepeats",
		label: "Workflow repeats",
		description: "Non-overlapping repeats a tool sequence needs before it is proposed as a skill.",
		presets: [2, 3, 4, 5, 6],
	},
	{
		key: "learnMaxProposals",
		label: "Max proposals",
		description: "Cap on each list in the digest. Every proposal costs the model context.",
		presets: [3, 5, 8, 12, 20],
	},
];

const LEARN_KEYS: ReadonlySet<string> = new Set(LEARN_SETTINGS.map((setting) => setting.key));

/**
 * Preset list for a numeric row, guaranteed to contain the value in force.
 *
 * Without this a value set by hand in settings.json — say 45 days — is absent
 * from the cycle, so the first keypress silently snaps it to the first preset,
 * discarding a deliberate choice the pane never showed as unusual. Every numeric
 * cycle row goes through this for that reason.
 */
function presetValues(presets: number[], current: number): string[] {
	const all = presets.includes(current) ? presets : [...presets, current].sort((a, b) => a - b);
	return all.map(String);
}

/**
 * A submenu component for selecting from a list of options.
 */
class WarningSettingsSubmenu extends Container {
	private settingsList: SettingsList;
	private state: WarningSettings;

	constructor(warnings: WarningSettings, onChange: (warnings: WarningSettings) => void, onCancel: () => void) {
		super();

		this.state = { ...warnings };

		const items: SettingItem[] = [
			{
				id: "anthropic-extra-usage",
				label: "Anthropic extra usage",
				description: "Warn when Anthropic subscription auth may use paid extra usage",
				currentValue: (this.state.anthropicExtraUsage ?? true) ? "true" : "false",
				values: ["true", "false"],
			},
		];

		this.settingsList = new SettingsList(
			items,
			Math.min(items.length, 10),
			getSettingsListTheme(),
			(id, newValue) => {
				switch (id) {
					case "anthropic-extra-usage":
						this.state = { ...this.state, anthropicExtraUsage: newValue === "true" };
						onChange({ ...this.state });
						break;
				}
			},
			onCancel,
		);

		this.addChild(this.settingsList);
	}

	handleInput(data: string): void {
		this.settingsList.handleInput(data);
	}
}

/**
 * The artifact platform targets, in the order the pane shows them. Each row is an
 * independent on/off toggle because `--platform` (and the `platform` setting it
 * mirrors) takes a list: emitting for two platforms at once is a supported shape.
 * The tokens here are the canonical ones — the CLI aliases (`copilot`, `gh`,
 * `native`) fold into these before anything reads them.
 */
const PLATFORM_ROWS: ReadonlyArray<{
	platform: MarketplacePlatform;
	label: string;
	description: string;
}> = [
	{
		platform: "claude",
		label: "claude",
		description:
			"Claude Code layout: .claude/ scaffolds, and authored plugins drop into ~/.claude/skills/<id>/. The default when nothing is set.",
	},
	{
		platform: "github",
		label: "github (copilot, gh)",
		description:
			"Copilot layout: .github/ scaffolds, and authored plugins are produced under ~/.agents/publish/github/<id>/.",
	},
	{
		platform: "agents",
		label: "agents (native)",
		description:
			"Cross-vendor .agents/ layout. Scaffolds only - a plugin belongs to no marketplace in this layout, so plugin authoring ignores it.",
	},
];

/** Row value for the platform setting: the selection, or the fallback when it is empty. */
function platformSummary(platforms: readonly MarketplacePlatform[]): string {
	return platforms.length > 0 ? platforms.join(", ") : "default (claude)";
}

/**
 * Submenu for the session's artifact platform targets (the `platform` setting,
 * same knob as `--platform`).
 *
 * A list rather than a cycle: the setting is a list, and the three tokens are not
 * mutually exclusive. Turning everything off is legal and means "unset" — the
 * per-consumer defaults come back (claude for plugins, .hoocode/ for scaffolds).
 */
class PlatformSubmenu extends Container {
	private settingsList: SettingsList;
	private selected: Set<MarketplacePlatform>;

	constructor(
		platforms: readonly MarketplacePlatform[],
		onChange: (platforms: MarketplacePlatform[]) => void,
		onDone: (summary: string) => void,
	) {
		super();

		this.selected = new Set(platforms);

		const items: SettingItem[] = PLATFORM_ROWS.map(({ platform, label, description }) => ({
			id: platform,
			label,
			description,
			currentValue: this.selected.has(platform) ? "on" : "off",
			values: ["on", "off"],
		}));

		this.settingsList = new SettingsList(
			items,
			Math.min(items.length, 10),
			getSettingsListTheme(),
			(id, newValue) => {
				const platform = id as MarketplacePlatform;
				if (newValue === "on") this.selected.add(platform);
				else this.selected.delete(platform);
				onChange(this.ordered());
			},
			() => onDone(platformSummary(this.ordered())),
		);

		this.addChild(this.settingsList);
	}

	/** Selection in the pane's order, so the persisted list does not depend on click order. */
	private ordered(): MarketplacePlatform[] {
		return PLATFORM_ROWS.map((row) => row.platform).filter((platform) => this.selected.has(platform));
	}

	handleInput(data: string): void {
		this.settingsList.handleInput(data);
	}
}

/**
 * Submenu for tool availability. The first rows are group switches (web,
 * semantic search) that decide whether a group's tools
 * exist at all — this is the same master switch that governs, e.g., the
 * webfetch/websearch tools. Below them are per-tool on/off toggles for the
 * tools that are currently available.
 *
 * Group switches change tool availability and apply on the next session;
 * per-tool toggles apply live and persist. A minimum core (read/bash/edit/
 * write) is guarded so the agent can never be left with no way to act.
 */
class ToolsSubmenu extends Container {
	private settingsList: SettingsList;
	private enabled: Map<string, boolean>;
	private static readonly CORE = new Set(["read", "bash", "edit", "write"]);
	private static readonly GROUP_PREFIX = "group:";

	constructor(
		tools: ToolToggleInfo[],
		groups: ToolGroupInfo[],
		onChange: (name: string, enabled: boolean) => void,
		onGroupChange: (id: string, enabled: boolean) => void,
		onCancel: () => void,
	) {
		super();

		this.enabled = new Map(tools.map((t) => [t.name, t.enabled]));

		const groupItems: SettingItem[] = groups.map((group) => ({
			id: `${ToolsSubmenu.GROUP_PREFIX}${group.id}`,
			label: `[group] ${group.label}`,
			description: `${group.description} Governs whether these tools exist; applies on the next session.`,
			currentValue: group.enabled ? "on" : "off",
			values: ["on", "off"],
		}));

		const toolItems: SettingItem[] = tools.map((tool) => ({
			id: tool.name,
			label: tool.name,
			description: ToolsSubmenu.CORE.has(tool.name)
				? "Core tool. Disabling leaves the agent unable to perform this action in every session."
				: "Disable to remove this tool from the agent this session and every future session.",
			currentValue: tool.enabled ? "on" : "off",
			// What the schema costs on every request, whether the tool is on or off:
			// off, it is the price of turning it back on. This is the number that
			// makes a tool worth disabling, so it belongs beside the switch.
			valueSuffix: tool.tokens !== undefined ? `${tokenCount(tool.tokens)} tok/turn` : undefined,
			values: ["on", "off"],
		}));

		const items = [...groupItems, ...toolItems];

		this.settingsList = new SettingsList(
			items,
			Math.min(items.length, 12),
			getSettingsListTheme(),
			(id, newValue) => {
				const wantEnabled = newValue === "on";
				if (id.startsWith(ToolsSubmenu.GROUP_PREFIX)) {
					onGroupChange(id.slice(ToolsSubmenu.GROUP_PREFIX.length), wantEnabled);
					return;
				}
				// Guard: never let the last core tool be turned off.
				if (!wantEnabled && ToolsSubmenu.CORE.has(id)) {
					const remainingCore = [...ToolsSubmenu.CORE].filter((n) => n !== id && this.enabled.get(n));
					if (remainingCore.length === 0) {
						this.settingsList.updateValue(id, "on");
						return;
					}
				}
				this.enabled.set(id, wantEnabled);
				onChange(id, wantEnabled);
			},
			onCancel,
			{ enableSearch: true },
		);

		this.addChild(this.settingsList);
	}

	handleInput(data: string): void {
		this.settingsList.handleInput(data);
	}
}

/** Byte-cap presets shown as human labels; mapped back to raw byte counts. */
const TOOL_OUTPUT_BYTE_PRESETS: ReadonlyArray<[label: string, bytes: number]> = [
	["8 KB", 8 * 1024],
	["16 KB", 16 * 1024],
	["32 KB", 32 * 1024],
	["64 KB", 64 * 1024],
	["128 KB", 128 * 1024],
];

function bytesToLabel(bytes: number): string {
	const match = TOOL_OUTPUT_BYTE_PRESETS.find(([, b]) => b === bytes);
	return match ? match[0] : `${Math.round(bytes / 1024)} KB`;
}

/** Byte-cap labels to cycle through, including a hand-set cap that matches no preset. */
function byteLabels(current: number): string[] {
	const all: Array<[string, number]> = TOOL_OUTPUT_BYTE_PRESETS.some(([, bytes]) => bytes === current)
		? [...TOOL_OUTPUT_BYTE_PRESETS]
		: [...TOOL_OUTPUT_BYTE_PRESETS, [bytesToLabel(current), current]];
	return all.sort((a, b) => a[1] - b[1]).map(([label]) => label);
}

interface ToolSettingsConfig {
	toolOutputDisplay: "collapsed" | "peek" | "standard";
	toolOutputMaxBytes: number;
	toolOutputMaxLines: number;
}

interface ToolSettingsCallbacks {
	onToolOutputDisplayChange: (level: "collapsed" | "peek" | "standard") => void;
	onToolOutputMaxBytesChange: (bytes: number) => void;
	onToolOutputMaxLinesChange: (lines: number) => void;
}

/**
 * Submenu for what a tool result looks like: how much of it is rendered, and
 * where it is truncated. The display level applies to the transcript at once;
 * the caps feed the tool runtime, so they bind on future tool calls.
 *
 * Context GC is deliberately not here. It is not about a tool's output but about
 * what stays in the outgoing context, which is the Context category's subject.
 */
class ToolSettingsSubmenu extends Container {
	private settingsList: SettingsList;

	constructor(config: ToolSettingsConfig, callbacks: ToolSettingsCallbacks, onCancel: () => void) {
		super();

		const items: SettingItem[] = [
			{
				id: "tool-output-display",
				label: "Display",
				description:
					"How tool results render. 'standard': shown (expandable). 'collapsed': hidden. 'peek': hidden with a ▸ reveal caret (press the expand key to reveal).",
				currentValue: config.toolOutputDisplay,
				values: ["standard", "collapsed", "peek"],
			},
			{
				id: "output-max-bytes",
				label: "Max bytes",
				description: "Byte cap on a single read/bash result before truncation. Applies to future tool calls.",
				currentValue: bytesToLabel(config.toolOutputMaxBytes),
				values: byteLabels(config.toolOutputMaxBytes),
			},
			{
				id: "output-max-lines",
				label: "Max lines",
				description: "Line cap on a single read/bash result before truncation. Applies to future tool calls.",
				currentValue: String(config.toolOutputMaxLines),
				values: presetValues([200, 400, 800, 1600, 3200], config.toolOutputMaxLines),
			},
		];

		this.settingsList = new SettingsList(
			items,
			Math.min(items.length, 10),
			getSettingsListTheme(),
			(id, newValue) => {
				switch (id) {
					case "tool-output-display":
						callbacks.onToolOutputDisplayChange(newValue as "collapsed" | "peek" | "standard");
						break;
					case "output-max-bytes": {
						const preset = TOOL_OUTPUT_BYTE_PRESETS.find(([label]) => label === newValue);
						// A hand-set cap has no preset entry; its label is "<n> KB" by
						// construction, so read the number back out of it.
						const bytes = preset ? preset[1] : Math.round(parseFloat(newValue) * 1024);
						if (Number.isFinite(bytes) && bytes > 0) callbacks.onToolOutputMaxBytesChange(bytes);
						break;
					}
					case "output-max-lines":
						callbacks.onToolOutputMaxLinesChange(parseInt(newValue, 10));
						break;
				}
			},
			onCancel,
		);

		this.addChild(this.settingsList);
	}

	handleInput(data: string): void {
		this.settingsList.handleInput(data);
	}
}

/** Single-line text editor for a string flag value. */
class FlagStringEditSubmenu extends Container {
	private input: Input;

	constructor(flagName: string, currentValue: string, done: (value?: string) => void) {
		super();

		this.addChild(new Text(theme.bold(theme.fg("accent", `Flag: --${flagName}`)), 0, 0));
		this.addChild(new Spacer(1));
		this.addChild(new Text(theme.fg("muted", "Enter a value · Enter to save · Esc to cancel"), 0, 0));
		this.addChild(new Spacer(1));

		this.input = new Input();
		this.input.setValue(currentValue);
		this.input.onSubmit = (value: string) => done(value);
		this.input.onEscape = () => done();
		this.addChild(this.input);
	}

	handleInput(data: string): void {
		this.input.handleInput(data);
	}
}

/**
 * Submenu listing extension-registered flags. Boolean flags toggle on/off;
 * string flags open a text editor. Changes persist to settings.json and are
 * applied live best-effort — extensions that read a flag only at load time
 * pick up the new value on the next launch.
 */
class FlagsSubmenu extends Container {
	private settingsList: SettingsList;

	constructor(flags: FlagInfo[], onChange: (name: string, value: boolean | string) => void, onCancel: () => void) {
		super();

		const items: SettingItem[] = flags.map((flag) => {
			const baseDescription = flag.description ?? "Extension-registered flag.";
			const description = `${baseDescription} Persists across sessions; some flags need a restart to fully apply.`;
			if (flag.type === "boolean") {
				return {
					id: flag.name,
					label: flag.name,
					description,
					currentValue: flag.value ? "on" : "off",
					values: ["on", "off"],
				};
			}
			return {
				id: flag.name,
				label: flag.name,
				description,
				currentValue: String(flag.value ?? ""),
				submenu: (currentValue, done) => new FlagStringEditSubmenu(flag.name, currentValue, done),
			};
		});

		const typeByName = new Map(flags.map((f) => [f.name, f.type]));

		this.settingsList = new SettingsList(
			items,
			Math.min(items.length, 12),
			getSettingsListTheme(),
			(id, newValue) => {
				if (typeByName.get(id) === "boolean") {
					onChange(id, newValue === "on");
				} else {
					onChange(id, newValue);
				}
			},
			onCancel,
			{ enableSearch: true },
		);

		this.addChild(this.settingsList);
	}

	handleInput(data: string): void {
		this.settingsList.handleInput(data);
	}
}

/**
 * Generic submenu holding a subset of leaf settings under a category label.
 * Shares the parent's change handler so cycle rows behave exactly as they did
 * when flat; nested submenu rows (theme, thinking, warnings) keep their own
 * factories.
 */
class CategorySubmenu extends Container {
	private settingsList: SettingsList;

	constructor(items: SettingItem[], onChange: (id: string, newValue: string) => void, onCancel: () => void) {
		super();
		this.settingsList = new SettingsList(
			items,
			Math.min(items.length, 10),
			getSettingsListTheme(),
			onChange,
			onCancel,
			{
				enableSearch: true,
			},
		);
		this.addChild(this.settingsList);
	}

	handleInput(data: string): void {
		this.settingsList.handleInput(data);
	}
}

class SelectSubmenu extends Container {
	private selectList: SelectList;

	constructor(
		title: string,
		description: string,
		options: SelectItem[],
		currentValue: string,
		onSelect: (value: string) => void,
		onCancel: () => void,
		onSelectionChange?: (value: string) => void,
	) {
		super();

		// Title
		this.addChild(new Text(theme.bold(theme.fg("accent", title)), 0, 0));

		// Description
		if (description) {
			this.addChild(new Spacer(1));
			this.addChild(new Text(theme.fg("muted", description), 0, 0));
		}

		// Spacer
		this.addChild(new Spacer(1));

		// Select list
		this.selectList = new SelectList(
			options,
			Math.min(options.length, 10),
			getSelectListTheme(),
			SETTINGS_SUBMENU_SELECT_LIST_LAYOUT,
		);

		// Pre-select current value
		const currentIndex = options.findIndex((o) => o.value === currentValue);
		if (currentIndex !== -1) {
			this.selectList.setSelectedIndex(currentIndex);
		}

		this.selectList.onSelect = (item) => {
			onSelect(item.value);
		};

		this.selectList.onCancel = onCancel;

		if (onSelectionChange) {
			this.selectList.onSelectionChange = (item) => {
				onSelectionChange(item.value);
			};
		}

		this.addChild(this.selectList);

		// Hint
		this.addChild(new Spacer(1));
		this.addChild(new Text(theme.fg("dim", "  Enter to select · Esc to go back"), 0, 0));
	}

	handleInput(data: string): void {
		this.selectList.handleInput(data);
	}
}

/**
 * Token counts, grouped and exact.
 *
 * `formatTokens` (2.7k) exists for the fixed-width chrome, where a count must
 * never grow the box it sits in. This pane is the opposite case: the point of
 * showing a number here is to compare it with another one, and "2.7k" hides the
 * difference between the tool that costs 2,710 and the one that costs 2,749.
 */
function tokenCount(tokens: number): string {
	return tokens.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

/**
 * The fixed per-turn cost, as one line under the pane.
 *
 * Every byte of system prompt and active tool schema is re-sent on every
 * request, and the pane is where that number is decided - so the pane is where
 * it should be visible. It reports the *live* session: a tool toggle applies at
 * once and moves it, while a setting that only takes effect next session (a tool
 * group, the light preset) leaves it alone until then.
 */
function formatSurfaceLine(surface: PromptSurface): string {
	const tools = `${surface.tools.length} tool${surface.tools.length === 1 ? "" : "s"}`;
	return (
		`  Per-turn surface: ${tokenCount(surface.totalTokens)} tokens  ` +
		`${theme.fg("dim", `(${tokenCount(surface.systemPromptTokens)} system prompt + ${tokenCount(surface.toolSchemaTokens)} schemas, ${tools})`)}`
	);
}

/**
 * Main settings selector component.
 */
export class SettingsSelectorComponent extends Container {
	private settingsList: SettingsList;
	private surfaceLine?: Text;
	private measureTokenSurface?: () => PromptSurface;

	constructor(config: SettingsConfig, callbacks: SettingsCallbacks) {
		super();

		this.measureTokenSurface = config.measureTokenSurface;

		const supportsImages = getCapabilities().images;
		const followUpKey = keyDisplayText("app.message.followUp");
		let currentWarnings = { ...config.warnings };

		// A row writes the user settings file; a key the project file also sets is
		// merged over it on the next session, so say so rather than letting the row
		// look like it took.
		const projectPinned = new Set(config.projectPinnedSettings);
		const pinnedNote = (key: string): string =>
			projectPinned.has(key)
				? ` This repo's .hoocode/settings.json sets ${key}, which overrides this row from the next session on.`
				: "";

		const initialSurface = config.measureTokenSurface?.();

		const toolsOn = config.tools.filter((t) => t.enabled).length;
		const toolsOff = config.tools.length - toolsOn;

		const items: SettingItem[] = [
			{
				id: "autocompact",
				label: "Auto-compact",
				description: "Automatically compact context when it gets too large",
				currentValue: config.autoCompact ? "true" : "false",
				values: ["true", "false"],
			},
			{
				id: "steering-mode",
				label: "Steering mode",
				description:
					"Enter while streaming queues steering messages. 'one-at-a-time': deliver one, wait for response. 'all': deliver all at once.",
				currentValue: config.steeringMode,
				values: ["one-at-a-time", "all"],
			},
			{
				id: "follow-up-mode",
				label: "Follow-up mode",
				description: `${followUpKey} queues follow-up messages until agent stops. 'one-at-a-time': deliver one, wait for response. 'all': deliver all at once.`,
				currentValue: config.followUpMode,
				values: ["one-at-a-time", "all"],
			},
			{
				id: "transport",
				label: "Transport",
				description: "Preferred transport for providers that support multiple transports",
				currentValue: config.transport,
				values: ["sse", "websocket", "websocket-cached", "auto"],
			},
			{
				id: "hide-thinking",
				label: "Hide thinking",
				description: "Hide thinking blocks in assistant responses",
				currentValue: config.hideThinkingBlock ? "true" : "false",
				values: ["true", "false"],
			},
			{
				id: "collapse-changelog",
				label: "Collapse changelog",
				description: "Show condensed changelog after updates",
				currentValue: config.collapseChangelog ? "true" : "false",
				values: ["true", "false"],
			},
			{
				id: "quiet-startup",
				label: "Quiet startup",
				description: "Disable verbose printing at startup",
				currentValue: config.quietStartup ? "true" : "false",
				values: ["true", "false"],
			},
			{
				id: "install-telemetry",
				label: "Install telemetry",
				description: "Send an anonymous version/update ping after changelog-detected updates",
				currentValue: config.enableInstallTelemetry ? "true" : "false",
				values: ["true", "false"],
			},
			{
				id: "double-escape-action",
				label: "Double-escape action",
				description: "Action when pressing Escape twice with empty editor",
				currentValue: config.doubleEscapeAction,
				values: ["tree", "fork", "none"],
			},
			{
				id: "tree-filter-mode",
				label: "Tree filter mode",
				description: "Default filter when opening /tree",
				currentValue: config.treeFilterMode,
				values: ["default", "no-tools", "user-only", "labeled-only", "all"],
			},
			{
				id: "warnings",
				label: "Warnings",
				description: "Enable or disable individual warnings",
				currentValue: "configure",
				submenu: (_currentValue, done) =>
					new WarningSettingsSubmenu(
						currentWarnings,
						(warnings) => {
							currentWarnings = warnings;
							callbacks.onWarningsChange(warnings);
						},
						() => done(),
					),
			},
			{
				id: "thinking",
				label: "Thinking level",
				description: "Reasoning depth for thinking-capable models",
				currentValue: config.thinkingLevel,
				submenu: (currentValue, done) =>
					new SelectSubmenu(
						"Thinking Level",
						"Select reasoning depth for thinking-capable models",
						config.availableThinkingLevels.map((level) => ({
							value: level,
							label: level,
							description: THINKING_DESCRIPTIONS[level],
						})),
						currentValue,
						(value) => {
							callbacks.onThinkingLevelChange(value as ThinkingLevel);
							done(value);
						},
						() => done(),
					),
			},
			{
				id: "theme",
				label: "Theme",
				description: "Color theme for the interface",
				currentValue: config.currentTheme,
				submenu: (currentValue, done) =>
					new SelectSubmenu(
						"Theme",
						"Select color theme",
						config.availableThemes.map((t) => ({
							value: t,
							label: t,
							description: getThemeDescription(t),
						})),
						currentValue,
						(value) => {
							callbacks.onThemeChange(value);
							done(value);
						},
						() => {
							// Restore original theme on cancel
							callbacks.onThemePreview?.(currentValue);
							done();
						},
						(value) => {
							// Preview theme on selection change
							callbacks.onThemePreview?.(value);
						},
					),
			},
		];

		// Only show image toggle if terminal supports it
		if (supportsImages) {
			// Insert after autocompact
			items.splice(1, 0, {
				id: "show-images",
				label: "Show images",
				description: "Render images inline in terminal",
				currentValue: config.showImages ? "true" : "false",
				values: ["true", "false"],
			});
			items.splice(2, 0, {
				id: "image-width-cells",
				label: "Image width",
				description: "Preferred inline image width in terminal cells",
				currentValue: String(config.imageWidthCells),
				values: presetValues([60, 80, 120], config.imageWidthCells),
			});
		}

		// Image auto-resize toggle (always available, affects both attached and read images)
		items.splice(supportsImages ? 3 : 1, 0, {
			id: "auto-resize-images",
			label: "Auto-resize images",
			description: "Resize large images to 2000x2000 max for better model compatibility",
			currentValue: config.autoResizeImages ? "true" : "false",
			values: ["true", "false"],
		});

		// Block images toggle (always available, insert after auto-resize-images)
		const autoResizeIndex = items.findIndex((item) => item.id === "auto-resize-images");
		items.splice(autoResizeIndex + 1, 0, {
			id: "block-images",
			label: "Block images",
			description: "Prevent images from being sent to LLM providers",
			currentValue: config.blockImages ? "true" : "false",
			values: ["true", "false"],
		});

		// Context GC (insert after block-images), a leaf the Context category picks up.
		items.splice(items.findIndex((item) => item.id === "block-images") + 1, 0, {
			id: "context-gc",
			label: "Context GC",
			description: "Stub superseded read results (files later edited or re-read) out of the outgoing context.",
			currentValue: config.contextGc ? "true" : "false",
			values: ["true", "false"],
		});

		// The light preset (insert after context GC). Read at startup to pick the
		// tool set and the system prompt, so it lands on the next session.
		const blockImagesIdx = items.findIndex((item) => item.id === "context-gc");
		items.splice(blockImagesIdx + 1, 0, {
			id: "light",
			label: "Light preset",
			description:
				"Low-token preset for small or local models: read/write/edit/bash only with stripped schemas, a terse system prompt, and no subagents/TodoWrite/skills/context files. Applies on the next session.",
			currentValue: config.light ? "true" : "false",
			values: ["true", "false"],
		});

		// Skill commands toggle (insert after the light preset)
		const blockImagesIndex = items.findIndex((item) => item.id === "light");
		items.splice(blockImagesIndex + 1, 0, {
			id: "skill-commands",
			label: "Skill commands",
			description: "Register skills as /skill:name commands",
			currentValue: config.enableSkillCommands ? "true" : "false",
			values: ["true", "false"],
		});

		// The autonomous plugin system's master switch (insert after skill-commands).
		// One flag for the lifecycle tools and the reuse nudge, so both flip together.
		const skillCommandsIdx = items.findIndex((item) => item.id === "skill-commands");
		items.splice(skillCommandsIdx + 1, 0, {
			id: "plugin-tools",
			label: "Plugin system",
			description: `Autonomous plugin system: the lifecycle tools (SearchPlugins, InstallPlugin, ...), ProposePlugin, and the plugin-reuse nudge. Tools arrive on the next session; the nudge follows at once.${pinnedNote("enablePluginTools")}`,
			currentValue: config.enablePluginTools ? "true" : "false",
			values: ["true", "false"],
		});

		// Plugin install scope (insert after the master switch). Governs the
		// autonomous InstallPlugin only — /plugin install asks per install.
		const pluginToolsIdx = items.findIndex((item) => item.id === "plugin-tools");
		items.splice(pluginToolsIdx + 1, 0, {
			id: "plugin-install-scope",
			label: "Plugin install scope",
			description: "Where autonomous plugin installs go: user (~/.agents) or project (this repo, shared)",
			currentValue: config.pluginInstallScope,
			values: ["user", "project"],
		});

		// Artifact platform targets (insert after plugin-install-scope). Set once and
		// it holds for every later session: it is the `platform` setting, which
		// `--platform` overrides for a single run.
		const pluginScopeIdx = items.findIndex((item) => item.id === "plugin-install-scope");
		let currentPlatforms = [...config.platform];
		items.splice(pluginScopeIdx + 1, 0, {
			id: "platform",
			label: "Platform",
			description: `Vendor layout(s) hoocode writes artifacts in: authored plugins and the /new-skill //new-agent //new-command scaffolds.${pinnedNote("platform")}`,
			currentValue: platformSummary(currentPlatforms),
			submenu: (_currentValue, done) =>
				new PlatformSubmenu(
					currentPlatforms,
					(platforms) => {
						currentPlatforms = platforms;
						callbacks.onPlatformChange(platforms);
					},
					(summary) => done(summary),
				),
		});

		// Hardware cursor toggle (insert after the platform row)
		const platformIndex = items.findIndex((item) => item.id === "platform");
		items.splice(platformIndex + 1, 0, {
			id: "show-hardware-cursor",
			label: "Show hardware cursor",
			description: "Show the terminal cursor while still positioning it for IME support",
			currentValue: config.showHardwareCursor ? "true" : "false",
			values: ["true", "false"],
		});

		// Editor border toggle (insert after show-hardware-cursor)
		const hardwareCursorIndex = items.findIndex((item) => item.id === "show-hardware-cursor");
		items.splice(hardwareCursorIndex + 1, 0, {
			id: "editor-border",
			label: "Editor border",
			description: "Box draws side borders, rule draws horizontal lines only",
			currentValue: config.editorBorder,
			values: ["box", "rule"],
		});

		// Editor padding toggle (insert after editor-border)
		const editorBorderIndex = items.findIndex((item) => item.id === "editor-border");
		items.splice(editorBorderIndex + 1, 0, {
			id: "editor-padding",
			label: "Editor padding",
			description: "Horizontal padding for input editor (0-3)",
			currentValue: String(config.editorPaddingX),
			values: ["0", "1", "2", "3"],
		});

		// Autocomplete max visible toggle (insert after editor-padding)
		const editorPaddingIndex = items.findIndex((item) => item.id === "editor-padding");
		items.splice(editorPaddingIndex + 1, 0, {
			id: "autocomplete-max-visible",
			label: "Autocomplete max items",
			description: "Max visible items in autocomplete dropdown (3-20)",
			currentValue: String(config.autocompleteMaxVisible),
			values: presetValues([3, 5, 7, 10, 15, 20], config.autocompleteMaxVisible),
		});

		// Clear on shrink toggle (insert after autocomplete-max-visible)
		const autocompleteIndex = items.findIndex((item) => item.id === "autocomplete-max-visible");
		items.splice(autocompleteIndex + 1, 0, {
			id: "clear-on-shrink",
			label: "Clear on shrink",
			description: "Clear empty rows when content shrinks (may cause flicker)",
			currentValue: config.clearOnShrink ? "true" : "false",
			values: ["true", "false"],
		});

		// Terminal progress toggle (insert after clear-on-shrink)
		const clearOnShrinkIndex = items.findIndex((item) => item.id === "clear-on-shrink");
		items.splice(clearOnShrinkIndex + 1, 0, {
			id: "terminal-progress",
			label: "Terminal progress",
			description: "Show OSC 9;4 progress indicators in the terminal tab bar",
			currentValue: config.showTerminalProgress ? "true" : "false",
			values: ["true", "false"],
		});

		// Voice silence window (insert after terminal-progress)
		const terminalProgressIndex = items.findIndex((item) => item.id === "terminal-progress");
		items.splice(terminalProgressIndex + 1, 0, {
			id: "voice-silence-ms",
			label: "Voice silence window",
			description: "Trailing-silence (ms) before voice capture auto-stops (300-10000). Env: VOICETOOLS_SILENCE_MS.",
			currentValue: String(config.voiceSilenceMs),
			values: presetValues([300, 500, 800, 1200, 2000, 3000, 5000, 8000, 10000], config.voiceSilenceMs),
		});

		// Webtools request timeout (insert after voice-silence-ms)
		const voiceSilenceIndex = items.findIndex((item) => item.id === "voice-silence-ms");
		items.splice(voiceSilenceIndex + 1, 0, {
			id: "webtools-timeout-secs",
			label: "Web tools timeout",
			description: "Per-request timeout (secs) for webfetch/websearch (1-120). Env: HOOCODE_WEBTOOLS_TIMEOUT.",
			currentValue: String(config.webtoolsTimeoutSecs),
			values: presetValues([5, 10, 15, 30, 60, 120], config.webtoolsTimeoutSecs),
		});

		// The /learn thresholds, appended as leaf rows and gathered into their own
		// category below. They are written to the user settings.json, which /learn
		// re-reads on every invocation, so a change here applies to the next run.
		const webtoolsIndex = items.findIndex((item) => item.id === "webtools-timeout-secs");
		items.splice(
			webtoolsIndex + 1,
			0,
			...LEARN_SETTINGS.map(({ key, label, description, presets }) => ({
				id: key,
				label,
				description,
				currentValue: String(config.learn[key]),
				values: presetValues(presets, config.learn[key]),
			})),
		);

		// Keep the tool/flag controls together as one block near the top, inserted
		// after the image/terminal splices above so they aren't leapfrogged.
		const toolFlagGroup: SettingItem[] = [
			{
				id: "tools",
				label: "Tools",
				description:
					"Enable/disable tools and tool groups (web, semantic search), each priced by what its schema costs per turn. Changes persist across sessions.",
				currentValue: toolsOff > 0 ? `${toolsOn} on · ${toolsOff} off` : `${toolsOn} on`,
				valueSuffix: initialSurface ? `${tokenCount(initialSurface.toolSchemaTokens)} tok/turn` : undefined,
				submenu: (_currentValue, done) =>
					new ToolsSubmenu(
						config.tools,
						config.toolGroups,
						(name, enabled) => {
							callbacks.onToolEnabledChange(name, enabled);
							// Applied live by the host, so the surface below is already stale.
							this.refreshTokenSurface();
						},
						(id, enabled) => callbacks.onToolGroupChange(id, enabled),
						() => done(),
					),
			},
			{
				id: "tool-output",
				label: "Tool output",
				description: "How much of a tool result is rendered, and where it is truncated.",
				currentValue: config.toolOutputDisplay,
				submenu: (_currentValue, done) =>
					new ToolSettingsSubmenu(
						{
							toolOutputDisplay: config.toolOutputDisplay,
							toolOutputMaxBytes: config.toolOutputMaxBytes,
							toolOutputMaxLines: config.toolOutputMaxLines,
						},
						{
							onToolOutputDisplayChange: callbacks.onToolOutputDisplayChange,
							onToolOutputMaxBytesChange: callbacks.onToolOutputMaxBytesChange,
							onToolOutputMaxLinesChange: callbacks.onToolOutputMaxLinesChange,
						},
						() => done(),
					),
			},
		];
		if (config.flags.length > 0) {
			toolFlagGroup.push({
				id: "flags",
				label: "Flags",
				description: "Set flags registered by extensions. Changes persist across sessions.",
				currentValue: `${config.flags.length} flag${config.flags.length === 1 ? "" : "s"}`,
				submenu: (_currentValue, done) =>
					new FlagsSubmenu(
						config.flags,
						(name, value) => callbacks.onFlagChange(name, value),
						() => done(),
					),
			});
		}
		// Add borders
		this.addChild(new DynamicBorder());

		// Shared change handler for every leaf (cycle) setting; used by the
		// top-level list and each category submenu.
		const applyChange = (id: string, newValue: string): void => {
			switch (id) {
				case "autocompact":
					callbacks.onAutoCompactChange(newValue === "true");
					break;
				case "show-images":
					callbacks.onShowImagesChange(newValue === "true");
					break;
				case "image-width-cells":
					callbacks.onImageWidthCellsChange(parseInt(newValue, 10));
					break;
				case "auto-resize-images":
					callbacks.onAutoResizeImagesChange(newValue === "true");
					break;
				case "block-images":
					callbacks.onBlockImagesChange(newValue === "true");
					break;
				case "skill-commands":
					callbacks.onEnableSkillCommandsChange(newValue === "true");
					break;
				case "light":
					callbacks.onLightChange(newValue === "true");
					break;
				case "plugin-tools":
					callbacks.onEnablePluginToolsChange(newValue === "true");
					break;
				case "plugin-install-scope":
					callbacks.onPluginInstallScopeChange(newValue as "user" | "project");
					break;
				case "platform":
					// Display only. Each toggle inside the submenu already applied itself;
					// closing it hands back the summary purely to refresh this row's value.
					break;
				case "steering-mode":
					callbacks.onSteeringModeChange(newValue as "all" | "one-at-a-time");
					break;
				case "follow-up-mode":
					callbacks.onFollowUpModeChange(newValue as "all" | "one-at-a-time");
					break;
				case "transport":
					callbacks.onTransportChange(newValue as Transport);
					break;
				case "hide-thinking":
					callbacks.onHideThinkingBlockChange(newValue === "true");
					break;
				case "collapse-changelog":
					callbacks.onCollapseChangelogChange(newValue === "true");
					break;
				case "quiet-startup":
					callbacks.onQuietStartupChange(newValue === "true");
					break;
				case "install-telemetry":
					callbacks.onEnableInstallTelemetryChange(newValue === "true");
					break;
				case "double-escape-action":
					callbacks.onDoubleEscapeActionChange(newValue as "fork" | "tree" | "none");
					break;
				case "tree-filter-mode":
					callbacks.onTreeFilterModeChange(
						newValue as "default" | "no-tools" | "user-only" | "labeled-only" | "all",
					);
					break;
				case "show-hardware-cursor":
					callbacks.onShowHardwareCursorChange(newValue === "true");
					break;
				case "editor-border":
					callbacks.onEditorBorderChange(newValue as "rule" | "box");
					break;
				case "editor-padding":
					callbacks.onEditorPaddingXChange(parseInt(newValue, 10));
					break;
				case "autocomplete-max-visible":
					callbacks.onAutocompleteMaxVisibleChange(parseInt(newValue, 10));
					break;
				case "clear-on-shrink":
					callbacks.onClearOnShrinkChange(newValue === "true");
					break;
				case "terminal-progress":
					callbacks.onShowTerminalProgressChange(newValue === "true");
					break;
				case "voice-silence-ms":
					callbacks.onVoiceSilenceMsChange(parseInt(newValue, 10));
					break;
				case "webtools-timeout-secs":
					callbacks.onWebtoolsTimeoutSecsChange(parseInt(newValue, 10));
					break;
				default:
					// The /learn rows are keyed by their settings.json name, so they need
					// no case of their own — the id is the key to write.
					if (LEARN_KEYS.has(id)) callbacks.onLearnSettingChange(id as LearnSettingKey, parseInt(newValue, 10));
					break;
			}
			// Most rows leave the per-turn surface alone; the ones that do not (a tool
			// toggle, anything that rebuilds the system prompt) move it immediately.
			// Re-measuring after every change is cheaper than knowing which is which.
			this.refreshTokenSurface();
		};

		// Partition the flat leaf settings into named category submenus so the
		// top level stays short. `items` holds autocompact + every leaf setting.
		const byId = new Map(items.map((item) => [item.id, item] as const));
		const pick = (ids: string[]): SettingItem[] =>
			ids.map((id) => byId.get(id)).filter((item): item is SettingItem => item !== undefined);
		const categoryRow = (id: string, label: string, description: string, ids: string[]): SettingItem => {
			const members = pick(ids);
			return {
				id,
				label,
				description,
				currentValue: `${members.length} setting${members.length === 1 ? "" : "s"}`,
				// Categories shortened the top level but also hid every setting from
				// its search: searching "theme" matched no category label. The member
				// labels ride along as search text so the query lands on the category
				// that holds the setting.
				keywords: members.map((member) => member.label).join(" "),
				submenu: (_currentValue, done) => new CategorySubmenu(members, applyChange, () => done()),
			};
		};

		const topItems: SettingItem[] = [
			...toolFlagGroup,
			// What the model is sent, and how much of it. Auto-compact used to sit
			// alone at the top level and context GC was filed under tool settings,
			// which left the three settings that decide the token budget in three
			// different places - with the light preset in none of them.
			categoryRow(
				"cat-context",
				"Context",
				"What the model is sent and how much of it: compaction, superseded reads, and the low-token preset.",
				["autocompact", "context-gc", "light"],
			),
			categoryRow(
				"cat-behavior",
				"Behavior",
				"Agent and session behavior: steering, follow-up, thinking, escape, tree filter, transport.",
				["steering-mode", "follow-up-mode", "thinking", "double-escape-action", "tree-filter-mode", "transport"],
			),
			categoryRow(
				"cat-interface",
				"Interface",
				"Appearance and editor: theme, thinking visibility, cursor, border, padding, autocomplete, terminal.",
				[
					"theme",
					"hide-thinking",
					"show-hardware-cursor",
					"editor-border",
					"editor-padding",
					"autocomplete-max-visible",
					"clear-on-shrink",
					"terminal-progress",
				],
			),
			// Artifact production. `plugin-install-scope` had no category and was
			// therefore unreachable from the pane despite having a live callback.
			categoryRow(
				"cat-plugins",
				"Plugins",
				"The autonomous plugin system's master switch, the vendor layout hoocode writes, and where autonomous installs land.",
				["plugin-tools", "platform", "plugin-install-scope"],
			),
			categoryRow("cat-images", "Images", "Inline image rendering and resizing.", [
				"show-images",
				"image-width-cells",
				"auto-resize-images",
				"block-images",
			]),
			// Top level rather than folded into Advanced: these are the thresholds
			// that decide whether /learn finds anything, and burying them is what
			// made them undiscoverable in the first place.
			categoryRow(
				"cat-learn",
				"Learning",
				"Thresholds /learn mines sessions with: how far back to look, and how often something must repeat.",
				LEARN_SETTINGS.map((setting) => setting.key),
			),
			categoryRow("cat-advanced", "Advanced", "Startup, telemetry, skills, warnings, voice, and web tools.", [
				"quiet-startup",
				"collapse-changelog",
				"install-telemetry",
				"skill-commands",
				"warnings",
				"voice-silence-ms",
				"webtools-timeout-secs",
			]),
		];

		this.settingsList = new SettingsList(
			topItems,
			Math.min(topItems.length, 10),
			getSettingsListTheme(),
			applyChange,
			callbacks.onCancel,
			{
				enableSearch: true,
			},
		);

		this.addChild(this.settingsList);
		if (initialSurface) {
			this.surfaceLine = new Text(formatSurfaceLine(initialSurface), 0, 0);
			this.addChild(this.surfaceLine);
		}
		this.addChild(new DynamicBorder());
	}

	/** Re-price the pane after a change that may have altered what each turn sends. */
	private refreshTokenSurface(): void {
		if (!this.surfaceLine || !this.measureTokenSurface) return;
		this.surfaceLine.setText(formatSurfaceLine(this.measureTokenSurface()));
	}

	getSettingsList(): SettingsList {
		return this.settingsList;
	}
}
