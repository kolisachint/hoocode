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
import type { WarningSettings } from "../../../core/settings-manager.js";
import { getSelectListTheme, getSettingsListTheme, theme } from "../theme/theme.js";
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

export interface ToolToggleInfo {
	/** Tool name (e.g. "read", "bash"). */
	name: string;
	/** Whether the tool is currently enabled (not in the persisted disabled set). */
	enabled: boolean;
}

export interface FlagInfo {
	/** Flag name (without the leading --). */
	name: string;
	description?: string;
	type: "boolean" | "string";
	/** Current effective value. */
	value: boolean | string;
}

export interface ToolGroupInfo {
	/** Group identifier (e.g. "web", "browser", "file", "embsearch"). */
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
	editorPaddingX: number;
	autocompleteMaxVisible: number;
	quietStartup: boolean;
	clearOnShrink: boolean;
	showTerminalProgress: boolean;
	warnings: WarningSettings;
	voiceSilenceMs: number;
	webtoolsTimeoutSecs: number;
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
	onEditorPaddingXChange: (padding: number) => void;
	onAutocompleteMaxVisibleChange: (maxVisible: number) => void;
	onQuietStartupChange: (enabled: boolean) => void;
	onClearOnShrinkChange: (enabled: boolean) => void;
	onShowTerminalProgressChange: (enabled: boolean) => void;
	onWarningsChange: (warnings: WarningSettings) => void;
	onVoiceSilenceMsChange: (ms: number) => void;
	onWebtoolsTimeoutSecsChange: (secs: number) => void;
	onCancel: () => void;
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
 * Submenu for tool availability. The first rows are group switches (web,
 * browser, document, semantic search) that decide whether a group's tools
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

export interface ToolSettingsConfig {
	toolOutputMaxBytes: number;
	toolOutputMaxLines: number;
	contextGc: boolean;
}

export interface ToolSettingsCallbacks {
	onToolOutputMaxBytesChange: (bytes: number) => void;
	onToolOutputMaxLinesChange: (lines: number) => void;
	onContextGcChange: (enabled: boolean) => void;
}

/**
 * Submenu for per-tool runtime settings. These feed the tool runtime the next
 * time it is built (next session / rebuild), so changes apply to future tool
 * calls rather than retroactively.
 */
class ToolSettingsSubmenu extends Container {
	private settingsList: SettingsList;

	constructor(config: ToolSettingsConfig, callbacks: ToolSettingsCallbacks, onCancel: () => void) {
		super();

		const items: SettingItem[] = [
			{
				id: "output-max-bytes",
				label: "Output max bytes",
				description: "Byte cap on a single read/bash result before truncation. Applies to future tool calls.",
				currentValue: bytesToLabel(config.toolOutputMaxBytes),
				values: TOOL_OUTPUT_BYTE_PRESETS.map(([label]) => label),
			},
			{
				id: "output-max-lines",
				label: "Output max lines",
				description: "Line cap on a single read/bash result before truncation. Applies to future tool calls.",
				currentValue: String(config.toolOutputMaxLines),
				values: ["200", "400", "800", "1600", "3200"],
			},
			{
				id: "context-gc",
				label: "Context GC",
				description: "Stub superseded read results (files later edited/re-read) out of the outgoing context.",
				currentValue: config.contextGc ? "true" : "false",
				values: ["true", "false"],
			},
		];

		this.settingsList = new SettingsList(
			items,
			Math.min(items.length, 10),
			getSettingsListTheme(),
			(id, newValue) => {
				switch (id) {
					case "output-max-bytes": {
						const preset = TOOL_OUTPUT_BYTE_PRESETS.find(([label]) => label === newValue);
						if (preset) callbacks.onToolOutputMaxBytesChange(preset[1]);
						break;
					}
					case "output-max-lines":
						callbacks.onToolOutputMaxLinesChange(parseInt(newValue, 10));
						break;
					case "context-gc":
						callbacks.onContextGcChange(newValue === "true");
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
 * Main settings selector component.
 */
export class SettingsSelectorComponent extends Container {
	private settingsList: SettingsList;

	constructor(config: SettingsConfig, callbacks: SettingsCallbacks) {
		super();

		const supportsImages = getCapabilities().images;
		const followUpKey = keyDisplayText("app.message.followUp");
		let currentWarnings = { ...config.warnings };

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
				values: ["60", "80", "120"],
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

		// Skill commands toggle (insert after block-images)
		const blockImagesIndex = items.findIndex((item) => item.id === "block-images");
		items.splice(blockImagesIndex + 1, 0, {
			id: "skill-commands",
			label: "Skill commands",
			description: "Register skills as /skill:name commands",
			currentValue: config.enableSkillCommands ? "true" : "false",
			values: ["true", "false"],
		});

		// Hardware cursor toggle (insert after skill-commands)
		const skillCommandsIndex = items.findIndex((item) => item.id === "skill-commands");
		items.splice(skillCommandsIndex + 1, 0, {
			id: "show-hardware-cursor",
			label: "Show hardware cursor",
			description: "Show the terminal cursor while still positioning it for IME support",
			currentValue: config.showHardwareCursor ? "true" : "false",
			values: ["true", "false"],
		});

		// Editor padding toggle (insert after show-hardware-cursor)
		const hardwareCursorIndex = items.findIndex((item) => item.id === "show-hardware-cursor");
		items.splice(hardwareCursorIndex + 1, 0, {
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
			values: ["3", "5", "7", "10", "15", "20"],
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
			description: "Trailing-silence (ms) before voice capture auto-stops (300-5000). Env: VOICETOOLS_SILENCE_MS.",
			currentValue: String(config.voiceSilenceMs),
			values: ["300", "500", "800", "1200", "2000", "3000", "5000"],
		});

		// Webtools request timeout (insert after voice-silence-ms)
		const voiceSilenceIndex = items.findIndex((item) => item.id === "voice-silence-ms");
		items.splice(voiceSilenceIndex + 1, 0, {
			id: "webtools-timeout-secs",
			label: "Web tools timeout",
			description: "Per-request timeout (secs) for webfetch/websearch (1-120). Env: HOOCODE_WEBTOOLS_TIMEOUT.",
			currentValue: String(config.webtoolsTimeoutSecs),
			values: ["5", "10", "15", "30", "60", "120"],
		});

		// Keep the tool/flag controls together as one block near the top, inserted
		// after the image/terminal splices above so they aren't leapfrogged.
		const toolFlagGroup: SettingItem[] = [
			{
				id: "tools",
				label: "Tools",
				description:
					"Enable/disable tools and tool groups (web, browser, document, semantic search). Changes persist across sessions.",
				currentValue: toolsOff > 0 ? `${toolsOn} on · ${toolsOff} off` : `${toolsOn} on`,
				submenu: (_currentValue, done) =>
					new ToolsSubmenu(
						config.tools,
						config.toolGroups,
						(name, enabled) => callbacks.onToolEnabledChange(name, enabled),
						(id, enabled) => callbacks.onToolGroupChange(id, enabled),
						() => done(),
					),
			},
			{
				id: "tool-output-display",
				label: "Tool output display",
				description:
					"How tool results render. 'standard': shown (expandable). 'collapsed': hidden. 'peek': hidden with a ▸ reveal caret (press the expand key to reveal).",
				currentValue: config.toolOutputDisplay,
				values: ["standard", "collapsed", "peek"],
			},
			{
				id: "tool-settings",
				label: "Tool settings",
				description: "Per-tool runtime settings: output truncation caps and context garbage collection.",
				currentValue: "configure",
				submenu: (_currentValue, done) =>
					new ToolSettingsSubmenu(
						{
							toolOutputMaxBytes: config.toolOutputMaxBytes,
							toolOutputMaxLines: config.toolOutputMaxLines,
							contextGc: config.contextGc,
						},
						{
							onToolOutputMaxBytesChange: callbacks.onToolOutputMaxBytesChange,
							onToolOutputMaxLinesChange: callbacks.onToolOutputMaxLinesChange,
							onContextGcChange: callbacks.onContextGcChange,
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
				case "tool-output-display":
					callbacks.onToolOutputDisplayChange(newValue as "collapsed" | "peek" | "standard");
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
					callbacks.onDoubleEscapeActionChange(newValue as "fork" | "tree");
					break;
				case "tree-filter-mode":
					callbacks.onTreeFilterModeChange(
						newValue as "default" | "no-tools" | "user-only" | "labeled-only" | "all",
					);
					break;
				case "show-hardware-cursor":
					callbacks.onShowHardwareCursorChange(newValue === "true");
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
			}
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
				submenu: (_currentValue, done) => new CategorySubmenu(members, applyChange, () => done()),
			};
		};

		const topItems: SettingItem[] = [
			...(byId.has("autocompact") ? [byId.get("autocompact")!] : []),
			...toolFlagGroup,
			categoryRow(
				"cat-behavior",
				"Behavior",
				"Agent and session behavior: steering, follow-up, thinking, escape, tree filter, transport.",
				["steering-mode", "follow-up-mode", "thinking", "double-escape-action", "tree-filter-mode", "transport"],
			),
			categoryRow(
				"cat-interface",
				"Interface",
				"Appearance and editor: theme, thinking visibility, cursor, padding, autocomplete, terminal.",
				[
					"theme",
					"hide-thinking",
					"show-hardware-cursor",
					"editor-padding",
					"autocomplete-max-visible",
					"clear-on-shrink",
					"terminal-progress",
				],
			),
			categoryRow("cat-images", "Images", "Inline image rendering and resizing.", [
				"show-images",
				"image-width-cells",
				"auto-resize-images",
				"block-images",
			]),
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
		this.addChild(new DynamicBorder());
	}

	getSettingsList(): SettingsList {
		return this.settingsList;
	}
}
