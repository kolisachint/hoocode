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
import type { AuthoredHook } from "./types.js";

/** Raw manifest shape accepted by the `plugin.json`-style formats (agents/claude). */
export interface RawManifest {
	name?: string;
	version?: string;
	description?: string;
	author?: string | { name?: string };
	hooks?: PluginHooksConfig | { hooks?: PluginHooksConfig };
	mcpServers?: Record<string, unknown>;
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

/** Read inline `mcpServers`, falling back to a `.mcp.json` file at `root`. */
export function normalizeMcp(
	raw: Record<string, unknown> | undefined,
	root: string,
	mcpFileName = ".mcp.json",
): Record<string, unknown> | undefined {
	if (raw && Object.keys(raw).length > 0) return raw;
	const mcpFile = path.join(root, mcpFileName);
	if (fs.existsSync(mcpFile)) {
		// VS Code / Copilot use `{ servers }`; Claude/native use `{ mcpServers }`. Accept both.
		const fileRaw = readJson<{ mcpServers?: Record<string, unknown>; servers?: Record<string, unknown> }>(mcpFile);
		const servers = fileRaw?.mcpServers ?? fileRaw?.servers;
		if (servers && Object.keys(servers).length > 0) return servers;
	}
	return undefined;
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
