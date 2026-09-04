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
import { CANVAS_ENTRY_FILE } from "../../../canvas/discovery.js";
import type { PluginCanvasExtension, PluginHooksConfig } from "../manifest.js";
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
	/**
	 * Copilot: canvas-extension directory (a path, or list of paths, relative to
	 * the plugin root — e.g. `"extensions": "extensions"`). The Agent Plugins spec
	 * gives the same key a second, unrelated meaning: a map of vendor metadata
	 * namespaces (`{ "com.github.copilot": { logo } }`). Both readings are handled
	 * — see {@link detectCanvasExtensions} — and the map form is preserved
	 * verbatim through `unknownFields`.
	 */
	extensions?: unknown;
	/** Copilot: LSP config path or inline definitions. Parsed for preservation only. */
	lspServers?: unknown;
	/** Any key the adapter does not model. */
	[key: string]: unknown;
}

/**
 * Manifest keys every adapter models; anything else is preserved via
 * `unknownFields`.
 *
 * Surfaced through {@link declaredVocabulary} so the Tier 2 drift check has
 * something to diff the vendor references against. A checker that inferred our
 * coverage by reading the parser would be checking its own inference; a declared
 * set is a claim the codebase makes and the check can falsify.
 */
const MODELLED_MANIFEST_KEYS = new Set([
	"name",
	"version",
	"description",
	"author",
	"skills",
	"commands",
	"agents",
	"themes",
	"hooks",
	"mcpServers",
	"providers",
	"$schema",
]);

/** Manifest keys not modelled here, preserved verbatim so a re-emit cannot drop them. */
export function unknownManifestFields(raw: RawManifest): Record<string, unknown> | undefined {
	const out: Record<string, unknown> = {};
	for (const [key, value] of Object.entries(raw)) {
		if (!MODELLED_MANIFEST_KEYS.has(key)) out[key] = value;
	}
	return Object.keys(out).length > 0 ? out : undefined;
}

/**
 * Vendor content namespace directory. The Agent Plugins layout both vendors are
 * converging on keeps portable capabilities at the plugin root (`skills/`) and
 * puts vendor-specific ones under a reverse-DNS directory, so Copilot's agents
 * and canvas extensions ship as `com.github.copilot/<capability>/`.
 * `github/awesome-copilot` publishes its whole catalog this way.
 */
export const COPILOT_CONTENT_DIR = "com.github.copilot";

/** Conventional canvas-extension container, relative to a plugin root. */
const CANVAS_EXTENSIONS_DIR = "extensions";

/**
 * Canvas-extension containers a plugin may ship, starting with the manifest's own
 * `extensions` path (Copilot's key, e.g. `"extensions": "extensions"`). Both are
 * real: `extensions/` is what a standalone canvas plugin uses, and the namespaced
 * one is what a materialized `awesome-copilot` entry carries.
 */
function canvasContainerDirs(root: string, override: unknown): string[] {
	const dirs: string[] = [];
	// `extensions` is also the key the Agent Plugins spec uses for a *map* of
	// vendor metadata (`{ "com.github.copilot": { logo } }`). `resolveCapabilityDir`
	// ignores anything that is not a string or string array, so the two readings
	// of one key cannot collide — and the map form stays in `unknownFields`, which
	// is what keeps a re-emit from dropping it.
	const declared = resolveCapabilityDir(root, override, CANVAS_EXTENSIONS_DIR);
	if (declared) dirs.push(declared);
	const namespaced = dirIfExists(root, path.join(COPILOT_CONTENT_DIR, CANVAS_EXTENSIONS_DIR));
	if (namespaced && !dirs.includes(namespaced)) dirs.push(namespaced);
	return dirs;
}

/**
 * Canvas extensions a plugin ships (see `docs/canvas-extensions-design.md` §4.3).
 *
 * Discovery keys off `<dir>/extension.mjs` and nothing else, exactly as
 * `core/canvas/discovery.ts` does for the workspace and user search roots — the
 * entry file is the whole contract, so a plugin's canvases are found by the same
 * rule as everyone else's.
 *
 * A plugin whose *root* carries `extension.mjs` is itself one canvas extension,
 * which is how a single-canvas repository is laid out; its id is the plugin id,
 * since there is no directory name below the root to take one from.
 */
export function detectCanvasExtensions(
	root: string,
	pluginId: string,
	override: unknown,
): PluginCanvasExtension[] | undefined {
	const found: PluginCanvasExtension[] = [];
	const seen = new Set<string>();
	const add = (id: string, dir: string): void => {
		if (seen.has(id)) return;
		if (!fs.existsSync(path.join(dir, CANVAS_ENTRY_FILE))) return;
		seen.add(id);
		found.push({ id, dir });
	};

	add(pluginId, root);
	for (const container of canvasContainerDirs(root, override)) {
		let entries: fs.Dirent[];
		try {
			entries = fs.readdirSync(container, { withFileTypes: true });
		} catch {
			continue;
		}
		for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
			if (!entry.isDirectory() && !entry.isSymbolicLink()) continue;
			add(entry.name, path.join(container, entry.name));
		}
	}
	return found.length > 0 ? found : undefined;
}

/**
 * On-disk component surfaces hoocode does not load, checked against both vendors'
 * file-location tables. The files are left alone; naming them is what makes the
 * gap visible (see NormalizedPlugin.unsupportedSurfaces).
 */
const UNSUPPORTED_SURFACES: ReadonlyArray<{ rel: string; label: string }> = [
	{ rel: "workflows", label: "workflows/" },
	{ rel: "output-styles", label: "output-styles/" },
	{ rel: "monitors/monitors.json", label: "monitors/" },
	{ rel: "bin", label: "bin/" },
	{ rel: "settings.json", label: "settings.json" },
	{ rel: ".lsp.json", label: ".lsp.json" },
	{ rel: "lsp.json", label: "lsp.json" },
	{ rel: path.join(".github", "lsp.json"), label: ".github/lsp.json" },
];

/** Which unsupported surfaces are present under `root`. */
export function detectUnsupportedSurfaces(root: string): string[] | undefined {
	const found = UNSUPPORTED_SURFACES.filter((s) => fs.existsSync(path.join(root, s.rel))).map((s) => s.label);
	return found.length > 0 ? found : undefined;
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
/**
 * Render frontmatter, or nothing at all when every field is empty.
 *
 * Emitting bare `---\n---` looks harmless and is not: YAML parses an empty
 * document as `null`, and Claude Code rejects a component whose frontmatter is
 * not a mapping. A command authored without a description hit exactly that, and
 * the artifact was invalid in the ecosystem it was written for while
 * round-tripping happily through our own reader.
 */
function emitFrontmatter(fields: Record<string, string | undefined>): string {
	const lines: string[] = [];
	for (const [key, value] of Object.entries(fields)) {
		if (value === undefined || value === "") continue;
		// Quote values that could be misparsed as YAML (contain a colon-space or start punctuation).
		const needsQuote = /[:#]|^[\s>|@`&*!%]/.test(value) || value.includes("\n");
		lines.push(`${key}: ${needsQuote ? JSON.stringify(value) : value}`);
	}
	return lines.length > 0 ? ["---", ...lines, "---"].join("\n") : "";
}

/** A markdown capability file: frontmatter block + body. */
export function emitMarkdown(fields: Record<string, string | undefined>, body: string): string {
	const frontmatter = emitFrontmatter(fields);
	return frontmatter ? `${frontmatter}\n\n${body.trimEnd()}\n` : `${body.trimEnd()}\n`;
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
 * "read, SearchCodebase") as a YAML flow sequence (`['read', 'SearchCodebase']`) for formats whose
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

/** The manifest keys the adapters model, as a sorted list. Input to the drift check (§2.2). */
export function modelledManifestKeys(): string[] {
	return [...MODELLED_MANIFEST_KEYS].sort();
}

/**
 * Surfaces we know about and deliberately do not load, as the **relative paths**
 * they actually occupy rather than their display labels.
 *
 * The distinction bit once already: the labels are written for humans
 * (`"monitors/"`), the vendor reference writes the real path
 * (`monitors/monitors.json`), and a drift check comparing the two reported a
 * known surface as a new finding.
 */
export function knownUnsupportedSurfaces(): string[] {
	return UNSUPPORTED_SURFACES.map((s) => s.rel.replace(/\\/g, "/")).sort();
}
