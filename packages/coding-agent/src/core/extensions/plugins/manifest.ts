/**
 * Plugin manifest parsing and normalization.
 *
 * A "plugin" is a directory that bundles agent capabilities (skills, commands,
 * agents/modes, themes, providers, hooks, MCP servers) behind a single manifest.
 * Two manifest formats are supported and parsed into one {@link NormalizedPlugin}:
 *
 *  - `.agents-plugin/plugin.json`  — native format (a strict superset)
 *  - `.claude-plugin/plugin.json`  — Claude Code compatible format
 *
 * When a directory carries both, the native format wins (no merge).
 */

import * as fs from "node:fs";
import * as path from "node:path";
import type { ProviderConfig } from "../types.js";

/** Marker subdirectory + manifest filename for each supported format. */
export const NATIVE_MANIFEST_DIR = ".agents-plugin";
export const CLAUDE_MANIFEST_DIR = ".claude-plugin";
export const MANIFEST_FILE = "plugin.json";

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
	/** Which manifest format produced this plugin. */
	format: "agents" | "claude";
	/** Resolved capability directories (only set when they exist on disk). */
	skillsDir?: string;
	commandsDir?: string;
	agentsDir?: string;
	themesDir?: string;
	/** Parsed hooks (from `hooks/hooks.json` or inline `hooks`). */
	hooks?: PluginHooksConfig;
	/** Parsed MCP servers (from `.mcp.json` or inline `mcpServers`). Wiring deferred. */
	mcpServers?: Record<string, unknown>;
	/** Native-only: providers to register. */
	providers?: PluginProvider[];
}

interface RawManifest {
	name?: string;
	version?: string;
	description?: string;
	author?: string | { name?: string };
	hooks?: PluginHooksConfig | { hooks?: PluginHooksConfig };
	mcpServers?: Record<string, unknown>;
	/** Native-only. */
	providers?: PluginProvider[];
}

function readJson<T>(file: string): T | null {
	try {
		return JSON.parse(fs.readFileSync(file, "utf8")) as T;
	} catch {
		return null;
	}
}

function dirIfExists(root: string, name: string): string | undefined {
	const p = path.join(root, name);
	return fs.existsSync(p) && fs.statSync(p).isDirectory() ? p : undefined;
}

/** Accept either `{ hooks: {...} }` or a bare event map. */
function normalizeHooks(raw: RawManifest["hooks"], root: string): PluginHooksConfig | undefined {
	let config: PluginHooksConfig | undefined;
	if (raw && typeof raw === "object") {
		config = "hooks" in raw && raw.hooks ? (raw.hooks as PluginHooksConfig) : (raw as PluginHooksConfig);
	}
	// Fall back to the conventional hooks/hooks.json file.
	if (!config) {
		const hooksFile = path.join(root, "hooks", "hooks.json");
		if (fs.existsSync(hooksFile)) {
			const fileRaw = readJson<RawManifest["hooks"]>(hooksFile);
			if (fileRaw && typeof fileRaw === "object") {
				config =
					"hooks" in fileRaw && fileRaw.hooks
						? (fileRaw.hooks as PluginHooksConfig)
						: (fileRaw as PluginHooksConfig);
			}
		}
	}
	return config && Object.keys(config).length > 0 ? config : undefined;
}

function normalizeMcp(raw: RawManifest["mcpServers"], root: string): Record<string, unknown> | undefined {
	if (raw && Object.keys(raw).length > 0) return raw;
	const mcpFile = path.join(root, ".mcp.json");
	if (fs.existsSync(mcpFile)) {
		const fileRaw = readJson<{ mcpServers?: Record<string, unknown> }>(mcpFile);
		if (fileRaw?.mcpServers && Object.keys(fileRaw.mcpServers).length > 0) return fileRaw.mcpServers;
	}
	return undefined;
}

/**
 * Parse and normalize a single plugin directory.
 * Returns null if no recognized manifest is present or it is invalid.
 */
export function parsePluginDir(root: string): NormalizedPlugin | null {
	// Native format wins when both are present (no merge).
	const nativePath = path.join(root, NATIVE_MANIFEST_DIR, MANIFEST_FILE);
	const claudePath = path.join(root, CLAUDE_MANIFEST_DIR, MANIFEST_FILE);

	let manifestPath: string;
	let format: NormalizedPlugin["format"];
	if (fs.existsSync(nativePath)) {
		manifestPath = nativePath;
		format = "agents";
	} else if (fs.existsSync(claudePath)) {
		manifestPath = claudePath;
		format = "claude";
	} else {
		return null;
	}

	const raw = readJson<RawManifest>(manifestPath);
	if (!raw) return null;

	const id = (raw.name ?? path.basename(root)).trim();
	if (!id) return null;

	const author = typeof raw.author === "string" ? raw.author : raw.author?.name;

	return {
		id,
		version: raw.version,
		description: raw.description,
		author,
		root,
		manifestPath,
		format,
		skillsDir: dirIfExists(root, "skills"),
		commandsDir: dirIfExists(root, "commands"),
		agentsDir: dirIfExists(root, "agents"),
		themesDir: dirIfExists(root, "themes"),
		hooks: normalizeHooks(raw.hooks, root),
		mcpServers: normalizeMcp(raw.mcpServers, root),
		// Providers are native-only; ignored on the Claude-compat path.
		providers: format === "agents" && Array.isArray(raw.providers) ? raw.providers : undefined,
	};
}
