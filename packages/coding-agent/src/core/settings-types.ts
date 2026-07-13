/**
 * Settings schema shared across the app.
 *
 * The `Settings` interface and its nested option groups describe the on-disk
 * global/project settings.json shape. Extracted from settings-manager.ts so the
 * schema can be imported without pulling in the manager implementation.
 */

import type { Transport } from "@kolisachint/hoocode-ai";

export interface CompactionSettings {
	enabled?: boolean; // default: true
	reserveTokens?: number; // default: 16384
	keepRecentTokens?: number; // default: 20000
	maxContextRatio?: number; // default: 0.75 - compact once context exceeds this fraction of the window, even before the reserveTokens rule fires (bounds transcript growth on large windows)
}

export interface ToolOutputSettings {
	maxBytes?: number; // default: 16384 (16KB) - byte cap on a single read/bash tool result before truncation
	maxLines?: number; // default: 800 - line cap on a single read/bash tool result before truncation
}

export interface BranchSummarySettings {
	reserveTokens?: number; // default: 16384 (tokens reserved for prompt + LLM response)
	skipPrompt?: boolean; // default: false - when true, skips "Summarize branch?" prompt and defaults to no summary
}

export interface ProviderRetrySettings {
	timeoutMs?: number; // SDK/provider request timeout in milliseconds
	maxRetries?: number; // SDK/provider retry attempts
	maxRetryDelayMs?: number; // default: 60000 (max server-requested delay before failing)
}

export interface RetrySettings {
	enabled?: boolean; // default: true
	maxRetries?: number; // default: 3
	baseDelayMs?: number; // default: 2000 (exponential backoff: 2s, 4s, 8s)
	provider?: ProviderRetrySettings;
}

export interface TerminalSettings {
	showImages?: boolean; // default: true (only relevant if terminal supports images)
	imageWidthCells?: number; // default: 60 (preferred inline image width in terminal cells)
	clearOnShrink?: boolean; // default: false (clear empty rows when content shrinks)
	showTerminalProgress?: boolean; // default: false (OSC 9;4 terminal progress indicators)
}

export interface ImageSettings {
	autoResize?: boolean; // default: true (resize images to 2000x2000 max for better model compatibility)
	blockImages?: boolean; // default: false - when true, prevents all images from being sent to LLM providers
}

export interface ThinkingBudgetsSettings {
	minimal?: number;
	low?: number;
	medium?: number;
	high?: number;
}

export interface MarkdownSettings {
	codeBlockIndent?: string; // default: "  "
}

export interface WarningSettings {
	anthropicExtraUsage?: boolean; // default: true
}

/**
 * Model categories for subagent model selection.
 * Categories map to explicit model IDs (e.g., "<provider>/<model-id>").
 * When a category is not configured, no override is applied and the agent's or
 * parent's default model is used.
 */
export interface ModelCategories {
	/** Quick, cheap models for read-only exploration (grep, find, file discovery) */
	fast?: string;
	/** Balanced models for general work (planning, moderate complexity) */
	standard?: string;
	/** Most capable models for complex reasoning (multi-file refactors) */
	capable?: string;
}

export type TransportSetting = Transport;

/**
 * Package source for npm/git packages.
 * - String form: load all resources from the package
 * - Object form: filter which resources to load
 */
export type PackageSource =
	| string
	| {
			source: string;
			extensions?: string[];
			skills?: string[];
			prompts?: string[];
			themes?: string[];
	  };

export interface Settings {
	lastChangelogVersion?: string;
	defaultProvider?: string;
	defaultModel?: string;
	defaultThinkingLevel?: "off" | "minimal" | "low" | "medium" | "high" | "xhigh";
	modelCategories?: ModelCategories; // Model categories for subagent model selection (fast, standard, capable)
	transport?: TransportSetting; // default: "auto"
	steeringMode?: "all" | "one-at-a-time";
	followUpMode?: "all" | "one-at-a-time";
	theme?: string;
	compaction?: CompactionSettings;
	toolOutput?: ToolOutputSettings; // caps on a single read/bash result (bounds per-turn transcript growth)
	branchSummary?: BranchSummarySettings;
	retry?: RetrySettings;
	hideThinkingBlock?: boolean;
	shellPath?: string; // Custom shell path (e.g., for Cygwin users on Windows)
	quietStartup?: boolean;
	shellCommandPrefix?: string; // Prefix prepended to every bash command (e.g., "shopt -s expand_aliases" for alias support)
	npmCommand?: string[]; // Command used for npm package lookup/install operations, argv-style (e.g., ["mise", "exec", "node@20", "--", "npm"])
	collapseChangelog?: boolean; // Show condensed changelog after update (use /changelog for full)
	enableInstallTelemetry?: boolean; // default: true - anonymous version/update ping after changelog-detected updates
	packages?: PackageSource[]; // Array of npm/git package sources (string or object with filtering)
	extensions?: string[]; // Array of local extension file paths or directories
	skills?: string[]; // Array of local skill file paths or directories
	prompts?: string[]; // Array of local prompt template paths or directories
	slashCommands?: string[]; // Array of local slash-command paths or directories
	themes?: string[]; // Array of local theme file paths or directories
	enableSkillCommands?: boolean; // default: true - register skills as /skill:name commands
	enableSubagent?: boolean; // default: true - enable the subagent tool (delegate tasks to isolated agent loops); set false to disable
	warmSubagents?: boolean; // default: false - dispatch eligible subagents on reused warm RPC workers (experimental)
	maxSubagentDepth?: number; // default: 2 - tree-wide subagent nesting cap (2 = a subagent may spawn one more level; 1 = no nesting)
	nestedSubagentConcurrency?: number; // default: 2 - max concurrent subagents per pool at nesting depth >= 1
	enableTodoWrite?: boolean; // default: true - enable the TodoWrite tool (maintain a live todo list in the task panel)
	enablePluginTools?: boolean; // default: false - master switch for the whole autonomous plugin system: the plugin lifecycle tools (SearchPlugins, InstallPlugin, ...) and ProposePlugin on the top-level agent AND the runtime plugin-reuse nudge. Off by default; set true to opt in.
	deferMcpSchemas?: boolean; // default: true - defer MCP tool schemas (inject names only + ResolveMcpTools on demand) instead of registering every schema up front; set false to eagerly register every schema
	enableWebTools?: boolean; // default: false - enable the webfetch + websearch tools (network access)
	enableBrowserTools?: boolean; // default: false - enable the browser_run + browser_continue tools (browsertools engine)
	enableBrowserLivePreview?: boolean; // default: false - default the live viewer on for browser_run runs and auto-open it
	enableFileTools?: boolean; // default: false - enable the document tools: DocRead/DocEdit/DocWrite + DocScan/DocGrep/DocPeek (filetools binary)
	terminal?: TerminalSettings;
	images?: ImageSettings;
	enabledModels?: string[]; // Model patterns for cycling (same format as --models CLI flag)
	doubleEscapeAction?: "fork" | "tree" | "none"; // Action for double-escape with empty editor (default: "tree")
	treeFilterMode?: "default" | "no-tools" | "user-only" | "labeled-only" | "all"; // Default filter when opening /tree
	thinkingBudgets?: ThinkingBudgetsSettings; // Custom token budgets for thinking levels
	thinkingDisplay?: "summarized" | "omitted"; // How adaptive-thinking models return thinking content. Opus 4.8 defaults to "omitted" (faster tool use); set "summarized" to surface thinking text.
	editorPaddingX?: number; // Horizontal padding for input editor (default: 0)
	autocompleteMaxVisible?: number; // Max visible items in autocomplete dropdown (default: 5)
	showHardwareCursor?: boolean; // Show terminal cursor while still positioning it for IME
	markdown?: MarkdownSettings;
	warnings?: WarningSettings;
	sessionDir?: string; // Custom session storage directory (same format as --session-dir CLI flag)
}
