/**
 * Plugin format adapter contract.
 *
 * A `PluginFormatAdapter` owns *everything* format-specific for one vendor
 * layout — how a plugin directory is detected and parsed (the reader), how a
 * marketplace index file is located, and how a plugin is scaffolded from a
 * normalized draft (the writer). The rest of the plugin subsystem is written
 * against {@link NormalizedPlugin} / {@link PluginDraft} and never branches on a
 * format again.
 *
 * This is the extension point the plugin system is designed around: Claude Code
 * and GitHub Copilot conventions evolve independently, so each lives in its own
 * adapter file ({@link ./claude.ts}, {@link ./copilot.ts}, {@link ./agents.ts}).
 * Supporting a new vendor — or tracking a change in an existing one — means
 * editing (or adding) exactly one adapter and registering it; no other module
 * changes.
 */

import type { NormalizedPlugin } from "../manifest.js";

/** Internal format id — matches `NormalizedPlugin.format` / `NormalizedMarketplace.format`. */
export type PluginFormatId = "agents" | "claude" | "copilot";

/**
 * Public platform token surfaced to users. The Copilot format lives under
 * `.github/`, so it is surfaced as `"github"` rather than the internal
 * `"copilot"`.
 */
export type MarketplacePlatform = "agents" | "claude" | "github";

/** A file produced by a format's scaffolder. `path` is relative to the plugin root (POSIX). */
export interface EmittedFile {
	path: string;
	content: string;
}

/** An authored skill: passive instructions loaded lazily (SKILL.md-style). */
export interface AuthoredSkill {
	name: string;
	description?: string;
	/** Instruction body (markdown). */
	body: string;
}

/** An authored slash command: a passive prompt template. */
export interface AuthoredCommand {
	name: string;
	description?: string;
	/** Prompt template body (markdown). */
	body: string;
}

/** An authored subagent definition. `tools` is a comma-separated allowlist (Claude Code convention). */
export interface AuthoredAgent {
	name: string;
	description?: string;
	/** Comma-separated `allowed-tools` string, e.g. "read, grep, glob". */
	tools?: string;
	model?: string;
	/** System-prompt / instruction body (markdown). */
	body: string;
}

/** An authored shell hook wired to a tool event. Active code — gated on authoring. */
export interface AuthoredHook {
	/** Claude Code event name: PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, Stop, ... */
	event: string;
	/** Regex matched against the tool name. Empty / "*" matches everything. */
	matcher?: string;
	command: string;
	/** Timeout in seconds. */
	timeout?: number;
}

/** An authored MCP server. Active code — gated on authoring. */
export interface AuthoredMcpServer {
	name: string;
	command: string;
	args?: string[];
	env?: Record<string, string>;
}

/**
 * Format-agnostic authoring spec consumed by {@link PluginFormatAdapter.emit}.
 * `ProposePlugin` builds one of these and asks each requested adapter to render
 * it, so the same authored capability lands correctly in Claude Code and
 * Copilot layouts without the authoring code knowing either.
 */
export interface PluginDraft {
	id: string;
	version?: string;
	description?: string;
	author?: string;
	skills?: AuthoredSkill[];
	commands?: AuthoredCommand[];
	agents?: AuthoredAgent[];
	hooks?: AuthoredHook[];
	mcpServers?: AuthoredMcpServer[];
}

/** Reader + writer for a single plugin format. */
export interface PluginFormatAdapter {
	/** Internal format id (stable; used in `NormalizedPlugin.format`). */
	readonly id: PluginFormatId;
	/** Public platform token (Copilot → `"github"`). */
	readonly platform: MarketplacePlatform;
	/**
	 * Precedence when several formats coexist in one directory (lower wins).
	 * Native `.agents-plugin` beats Claude beats Copilot.
	 */
	readonly precedence: number;
	/** Short human label for messages and scaffolding summaries. */
	readonly label: string;
	/** Location of this format's marketplace index file, relative to a marketplace root (POSIX). */
	readonly marketplaceFile: string;

	/** True if `root` carries this format's plugin manifest. */
	detectPlugin(root: string): boolean;
	/** Parse `root` as a plugin of this format. Returns null when unparseable. */
	parsePlugin(root: string): NormalizedPlugin | null;
	/** Render `draft` into this format's on-disk layout (paths relative to the plugin root). */
	emit(draft: PluginDraft): EmittedFile[];
}
