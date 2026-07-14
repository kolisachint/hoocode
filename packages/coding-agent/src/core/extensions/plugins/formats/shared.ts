/**
 * Shared, format-agnostic helpers used by the plugin format adapters.
 *
 * Kept deliberately free of any runtime dependency on `../manifest.js` (it only
 * imports *types* from there) so the adapter registry can be wired without a
 * module cycle: `manifest.ts → formats/index.ts → formats/<adapter>.ts →
 * shared.ts`, with the back-reference to manifest types erased at compile time.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import type { PluginHooksConfig } from "../manifest.js";
import type { AuthoredHook, WorkspaceLayout } from "./types.js";

/** Raw manifest shape accepted by the `plugin.json`-style formats (agents/claude/copilot). */
export interface RawManifest {
	name?: string;
	version?: string;
	description?: string;
	author?: string | { name?: string };
	/** Optional capability-dir override(s): a path (or list of paths) relative to the plugin root, e.g. `"skills": "./skills/"`. */
	skills?: unknown;
	commands?: unknown;
	agents?: unknown;
	themes?: unknown;
	hooks?: PluginHooksConfig | { hooks?: PluginHooksConfig };
	/** Inline server map, or a path (Claude Code form, e.g. `"./.mcp.json"`) relative to the plugin root. */
	mcpServers?: Record<string, unknown> | string;
	/** Native-only. */
	providers?: unknown;
}

export function readJson<T>(file: string): T | null {
	try {
		return JSON.parse(fs.readFileSync(file, "utf8")) as T;
	} catch {
		return null;
	}
}

export function dirIfExists(root: string, name: string): string | undefined {
	const p = path.join(root, name);
	return fs.existsSync(p) && fs.statSync(p).isDirectory() ? p : undefined;
}

/**
 * Resolve a capability directory: an explicit manifest override — a path (or
 * list of paths, first existing wins) relative to the plugin root, e.g.
 * `"skills": "./skills/"` — beats the conventional directory.
 */
export function resolveCapabilityDir(root: string, override: unknown, conventional: string): string | undefined {
	const candidates = Array.isArray(override) ? override : [override];
	for (const c of candidates) {
		if (typeof c !== "string" || !c.trim()) continue;
		const p = path.resolve(root, c.trim());
		if (fs.existsSync(p) && fs.statSync(p).isDirectory()) return p;
	}
	return dirIfExists(root, conventional);
}

/** Coerce an author field (string or `{ name }`) to a plain string. */
export function parseAuthor(author: RawManifest["author"]): string | undefined {
	return typeof author === "string" ? author : author?.name;
}

/** Accept either `{ hooks: {...} }` or a bare event map, falling back to `hooks/hooks.json`. */
export function normalizeHooks(raw: RawManifest["hooks"], root: string): PluginHooksConfig | undefined {
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

/**
 * Resolve inline `mcpServers`, a manifest-supplied file path, or a `.mcp.json`
 * file at `root`.
 *
 * `raw` may be:
 *  - a string path (Claude Code form: `"mcpServers": "./.mcp.json"`), resolved
 *    relative to the plugin root and read as a server file;
 *  - an inline server map (returned as-is);
 *  - undefined, falling back to `<root>/<mcpFileName>`.
 *
 * Server files use `{ mcpServers }` (Claude/native) or `{ servers }` (Code/Copilot);
 * both keys are accepted.
 */
export function normalizeMcp(
	raw: Record<string, unknown> | string | undefined,
	root: string,
	mcpFileName = ".mcp.json",
): Record<string, unknown> | undefined {
	// Claude Code form: a path to a server file, relative to the plugin root.
	if (typeof raw === "string") {
		return raw.trim() ? readMcpFile(path.resolve(root, raw.trim())) : undefined;
	}
	if (raw && Object.keys(raw).length > 0) return raw;
	return readMcpFile(path.join(root, mcpFileName));
}

/** Read a server file, accepting both `{ mcpServers }` and `{ servers }` keys. */
function readMcpFile(mcpFile: string): Record<string, unknown> | undefined {
	if (!fs.existsSync(mcpFile)) return undefined;
	const fileRaw = readJson<{ mcpServers?: Record<string, unknown>; servers?: Record<string, unknown> }>(mcpFile);
	const servers = fileRaw?.mcpServers ?? fileRaw?.servers;
	return servers && Object.keys(servers).length > 0 ? servers : undefined;
}

// ============================================================================
// Emit helpers (used by the writer / scaffolding half of each adapter)
// ============================================================================

/** Serialize a small set of frontmatter fields to YAML. Values are strings only. */
export function emitFrontmatter(fields: Record<string, string | undefined>): string {
	const lines: string[] = ["---"];
	for (const [key, value] of Object.entries(fields)) {
		if (value === undefined || value === "") continue;
		// Quote values that could be misparsed as YAML (contain a colon-space or start punctuation).
		const needsQuote = /[:#]|^[\s>|@`&*!%]/.test(value) || value.includes("\n");
		lines.push(`${key}: ${needsQuote ? JSON.stringify(value) : value}`);
	}
	lines.push("---");
	return lines.join("\n");
}

/** A markdown capability file: frontmatter block + body. */
export function emitMarkdown(fields: Record<string, string | undefined>, body: string): string {
	return `${emitFrontmatter(fields)}\n\n${body.trimEnd()}\n`;
}

/** Pretty-print a JSON manifest with a trailing newline (matches repo convention). */
export function emitJson(value: unknown): string {
	return `${JSON.stringify(value, null, 2)}\n`;
}

/** Convert authored hooks into the on-disk `hooks.json` event-map shape shared by all formats. */
export function authoredHooksToConfig(hooks: AuthoredHook[]): Record<string, unknown> {
	const byEvent: Record<string, Array<{ matcher?: string; hooks: unknown[] }>> = {};
	for (const h of hooks) {
		const group = byEvent[h.event] ?? [];
		byEvent[h.event] = group;
		group.push({
			...(h.matcher ? { matcher: h.matcher } : {}),
			hooks: [{ type: "command", command: h.command, ...(h.timeout ? { timeout: h.timeout } : {}) }],
		});
	}
	return byEvent;
}

/** Slug-safe filename component (no path separators, conservative charset). */
export function slug(name: string): string {
	return name
		.trim()
		.replace(/[^a-zA-Z0-9._-]+/g, "-")
		.replace(/^-+|-+$/g, "")
		.slice(0, 80);
}

/**
 * Render a comma-separated tool allowlist (Claude Code convention, e.g.
 * "read, grep") as a YAML flow sequence (`['read', 'grep']`) for formats whose
 * frontmatter takes tools as a list (Copilot custom agents / prompt files).
 */
export function toolsYamlList(tools: string | undefined): string | undefined {
	const items = (tools ?? "")
		.split(/[,\s]+/)
		.map((t) => t.trim())
		.filter(Boolean);
	if (items.length === 0) return undefined;
	return `[${items.map((t) => `'${t.replace(/'/g, "''")}'`).join(", ")}]`;
}

/**
 * The workspace layout shared by the `.agents/` and `.claude/` conventions:
 * `<root>/skills/<name>/SKILL.md`, `<root>/agents/<name>.md`,
 * `<root>/commands/<name>.md`. Copilot's differs and lives in its adapter.
 */
export function claudeStyleWorkspace(root: string): WorkspaceLayout {
	return {
		root,
		emitSkill: (s) => ({
			path: path.join(root, "skills", slug(s.name), "SKILL.md"),
			content: emitMarkdown({ name: s.name, description: s.description }, s.body),
		}),
		emitAgent: (a) => ({
			path: path.join(root, "agents", `${slug(a.name)}.md`),
			content: emitMarkdown({ name: a.name, description: a.description, tools: a.tools, model: a.model }, a.body),
		}),
		emitCommand: (c) => ({
			path: path.join(root, "commands", `${slug(c.name)}.md`),
			content: emitMarkdown({ name: c.name, description: c.description }, c.body),
		}),
	};
}
