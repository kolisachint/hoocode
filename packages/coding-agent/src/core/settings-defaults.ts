import type { Settings } from "./settings-manager.js";

export const DEFAULT_SETTINGS = {
	transport: "auto",
	steeringMode: "one-at-a-time",
	followUpMode: "one-at-a-time",
	compaction: {
		enabled: true,
		reserveTokens: 16384,
		keepRecentTokens: 20000,
	},
	branchSummary: {
		reserveTokens: 16384,
		skipPrompt: false,
	},
	retry: {
		enabled: true,
		maxRetries: 3,
		baseDelayMs: 2000,
		provider: {
			maxRetryDelayMs: 60000,
		},
	},
	hideThinkingBlock: false,
	quietStartup: false,
	collapseChangelog: false,
	enableInstallTelemetry: true,
	enableSkillCommands: true,
	enableSubagent: false,
	enableTodoWrite: true,
	terminal: {
		showImages: true,
		imageWidthCells: 60,
		clearOnShrink: false,
		showTerminalProgress: false,
	},
	images: {
		autoResize: true,
		blockImages: false,
	},
	doubleEscapeAction: "tree",
	treeFilterMode: "default",
	editorPaddingX: 0,
	autocompleteMaxVisible: 5,
	markdown: {
		codeBlockIndent: "  ",
	},
	warnings: {
		anthropicExtraUsage: true,
	},
	packages: [],
	extensions: [],
	skills: [],
	prompts: [],
	slashCommands: [],
	themes: [],
} satisfies Settings;
