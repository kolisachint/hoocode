/**
 * Plugin manifest types and the format-dispatching parse entry point.
 *
 * A "plugin" is a directory that bundles agent capabilities (skills, commands,
 * agents/modes, themes, providers, hooks, MCP servers) behind a single manifest.
 * Several vendor formats are supported and parsed into one {@link NormalizedPlugin}:
 *
 *  - `.agents-plugin/plugin.json`      — native format (a strict superset)
 *  - `.claude-plugin/plugin.json`      — Claude Code compatible format
 *  - `.github/copilot-plugin.json`     — GitHub Copilot format (`.github/` layout)
 *
 * The per-format detect/parse/emit logic lives in {@link ./formats}, one adapter
 * per vendor, behind the {@link PluginFormatAdapter} contract — so this module
 * carries only the shared types and delegates directory parsing to the registry.
 * When a directory carries more than one format, the highest-precedence one wins
 * (native > Claude > Copilot; no merge).
 */

import type { ProviderConfig } from "../types.js";
import { parsePluginWithFormats } from "./formats/index.js";
import type { MarketplacePlatform } from "./formats/types.js";

/** Marker subdirectory + manifest filename for each supported format. */
export const NATIVE_MANIFEST_DIR = ".agents-plugin";
export const CLAUDE_MANIFEST_DIR = ".claude-plugin";
export const COPILOT_MANIFEST_DIR = ".github";
export const MANIFEST_FILE = "plugin.json";
export const COPILOT_MANIFEST_FILE = "copilot-plugin.json";

/** A single shell command invoked by a hook. */
export interface PluginHookCommand {
	type?: "command";
	command: string;
	/** Timeout in seconds (Claude Code convention). */
	timeout?: number;
}

/** A matcher group: run these commands when `matcher` matches. */
export interface PluginHookMatcherGroup {
	/** Regex string matched against the tool name. Empty / "*" matches everything. */
	matcher?: string;
	hooks: PluginHookCommand[];
}

/**
 * Hook event map keyed by Claude Code event name
 * (PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, Stop, ...).
 */
export type PluginHooksConfig = Record<string, PluginHookMatcherGroup[]>;

/** A provider contributed by a native plugin (config-only; uses a built-in API handler). */
export interface PluginProvider {
	name: string;
	config: ProviderConfig;
}

/** Normalized, format-agnostic representation of a plugin. */
export interface NormalizedPlugin {
	/** Stable identifier (manifest `name`). */
	id: string;
	version?: string;
	description?: string;
	author?: string;
	/** Absolute path to the plugin root directory. */
	root: string;
	/** Absolute path to the parsed manifest file. */
	manifestPath: string;
	/** Which manifest format produced this plugin (the precedence winner). */
	format: "agents" | "claude" | "copilot";
	/**
	 * Every platform this plugin directory offers. When a directory carries more
	 * than one format manifest (e.g. both `.claude-plugin/` and `.github/`), all
	 * are recorded here — the precedence winner's platform first — rather than
	 * hidden behind the single `format`. Always non-empty (at least `format`'s
	 * platform), mirroring the marketplace's `supportPlatform`.
	 */
	supportPlatform: MarketplacePlatform[];
	/** Resolved capability directories (only set when they exist on disk). */
	skillsDir?: string;
	commandsDir?: string;
	agentsDir?: string;
	themesDir?: string;
	/** Parsed hooks (from `hooks/hooks.json` or inline `hooks`). */
	hooks?: PluginHooksConfig;
	/** Parsed MCP servers (from `.mcp.json` or inline `mcpServers`); connected via the MCP registry. */
	mcpServers?: Record<string, unknown>;
	/** Native-only: providers to register. */
	providers?: PluginProvider[];
}

/**
 * Parse and normalize a single plugin directory.
 * Returns null if no recognized manifest is present or it is invalid.
 *
 * Delegates to the format registry; see {@link ./formats/index.ts}.
 */
export function parsePluginDir(root: string): NormalizedPlugin | null {
	return parsePluginWithFormats(root);
}
