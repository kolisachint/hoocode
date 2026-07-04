/**
 * Plugin marketplaces — install plugins from a curated index hosted in a git repo
 * (or a local directory).
 *
 * A marketplace is a directory containing an index manifest in one of three formats:
 *  - Native:  `.agents-plugin/marketplace.json`  (preferred; the `.agents` surface)
 *  - Claude:  `.claude-plugin/marketplace.json`
 *  - Copilot: `.github/marketplace.json`  (Copilot-style git index)
 *
 * All three use the same shape: `{ name?, owner?, plugins: [{ name, source, description? }] }`.
 * A plugin `source` is either a path relative to the marketplace root (a plugin
 * directory inside the repo), a git URL, or an `npm:<spec>` reference.
 *
 * When more than one is present the native `.agents-plugin` format wins, then
 * Claude, then Copilot (no merge) — matching the "`.agents/` first" policy used
 * across hoocode's resource surfaces.
 */

import * as fs from "node:fs";
import * as path from "node:path";

export const AGENTS_MARKETPLACE_FILE = path.join(".agents-plugin", "marketplace.json");
export const CLAUDE_MARKETPLACE_FILE = path.join(".claude-plugin", "marketplace.json");
export const COPILOT_MARKETPLACE_FILE = path.join(".github", "marketplace.json");

/**
 * Canonical platform token used by the optional `supportPlatform` field. The
 * `.github/marketplace.json` (Copilot-style) format is surfaced as `"github"` —
 * friendlier than the internal `format: "copilot"` and matching where the file
 * lives. Authored aliases (`copilot`, `gh`) normalize to `github`.
 */
export type MarketplacePlatform = "agents" | "claude" | "github";

/** Map an internal marketplace `format` to its public platform token. */
function formatToPlatform(format: NormalizedMarketplace["format"]): MarketplacePlatform {
	return format === "copilot" ? "github" : format;
}

/** Normalize an authored `supportPlatform` (string | string[]) to canonical tokens. */
export function normalizePlatforms(value: unknown): MarketplacePlatform[] {
	const raw = Array.isArray(value) ? value : value == null ? [] : [value];
	const out: MarketplacePlatform[] = [];
	for (const v of raw) {
		if (typeof v !== "string") continue;
		const t = v.trim().toLowerCase();
		const canonical: MarketplacePlatform | undefined =
			t === "agents" || t === "native"
				? "agents"
				: t === "claude"
					? "claude"
					: t === "github" || t === "copilot" || t === "gh"
						? "github"
						: undefined;
		if (canonical && !out.includes(canonical)) out.push(canonical);
	}
	return out;
}

export interface MarketplacePluginEntry {
	name: string;
	/** Relative path, git URL, or `npm:<spec>`. */
	source: string;
	description?: string;
	/**
	 * Optional platform(s) this entry targets. Only set when authored on the
	 * entry; absent means "no per-entry restriction" (the marketplace's
	 * platforms apply). Purely informational today — nothing filters on it.
	 */
	supportPlatform?: MarketplacePlatform[];
}

export interface NormalizedMarketplace {
	name: string;
	/** Precedence winner when several index files conflict (agents > claude > copilot). */
	format: "agents" | "claude" | "copilot";
	/**
	 * Every platform this marketplace directory offers, so a conflict between
	 * multiple index files (e.g. both `.github/` and `.claude-plugin/`) is
	 * recorded rather than hidden. Always includes the resolved `format`'s
	 * platform; also folds in any authored top-level `supportPlatform`. Never
	 * empty.
	 */
	supportPlatform: MarketplacePlatform[];
	/** Absolute path to the marketplace directory. */
	root: string;
	manifestPath: string;
	plugins: MarketplacePluginEntry[];
}

interface RawMarketplace {
	name?: string;
	owner?: string;
	/** Optional authored platform hint (string | string[]); folded into supportPlatform. */
	supportPlatform?: unknown;
	plugins?: Array<{ name?: string; source?: string; description?: string; supportPlatform?: unknown }>;
}

/** A resolved, installable plugin source. */
export type ResolvedPluginSource =
	| { kind: "local"; path: string }
	| { kind: "git"; url: string }
	| { kind: "npm"; spec: string };

function readJson<T>(file: string): T | null {
	try {
		return JSON.parse(fs.readFileSync(file, "utf8")) as T;
	} catch {
		return null;
	}
}

/**
 * Parse a marketplace directory. Native `.agents-plugin` wins, then Claude, then
 * Copilot when more than one file is present. Returns null if no manifest exists
 * or it is invalid.
 */
export function parseMarketplaceDir(dir: string): NormalizedMarketplace | null {
	const agentsPath = path.join(dir, AGENTS_MARKETPLACE_FILE);
	const claudePath = path.join(dir, CLAUDE_MARKETPLACE_FILE);
	const copilotPath = path.join(dir, COPILOT_MARKETPLACE_FILE);

	// Record every index format present so a conflict (more than one file) is
	// visible via supportPlatform, not silently dropped.
	const present: Array<{ format: NormalizedMarketplace["format"]; path: string }> = [];
	if (fs.existsSync(agentsPath)) present.push({ format: "agents", path: agentsPath });
	if (fs.existsSync(claudePath)) present.push({ format: "claude", path: claudePath });
	if (fs.existsSync(copilotPath)) present.push({ format: "copilot", path: copilotPath });
	if (present.length === 0) return null;

	// Precedence winner (present is built in agents > claude > copilot order).
	const { format, path: manifestPath } = present[0];

	const raw = readJson<RawMarketplace>(manifestPath);
	if (!raw) return null;

	const plugins: MarketplacePluginEntry[] = [];
	for (const entry of raw.plugins ?? []) {
		if (entry?.name && entry.source) {
			const entryPlatforms = normalizePlatforms(entry.supportPlatform);
			plugins.push({
				name: entry.name,
				source: entry.source,
				description: entry.description,
				...(entryPlatforms.length > 0 ? { supportPlatform: entryPlatforms } : {}),
			});
		}
	}

	// supportPlatform = platforms of all present index files, plus any authored
	// top-level hint, deduped. Always non-empty (at least the resolved format).
	const supportPlatform: MarketplacePlatform[] = [];
	for (const p of [...present.map((e) => formatToPlatform(e.format)), ...normalizePlatforms(raw.supportPlatform)]) {
		if (!supportPlatform.includes(p)) supportPlatform.push(p);
	}

	return {
		name: (raw.name ?? path.basename(dir)).trim() || path.basename(dir),
		format,
		supportPlatform,
		root: dir,
		manifestPath,
		plugins,
	};
}

/** Classify and resolve a plugin `source` against its marketplace root. */
export function resolvePluginSource(source: string, marketplaceRoot: string): ResolvedPluginSource {
	const s = source.trim();
	if (s.startsWith("npm:")) return { kind: "npm", spec: s.slice(4) };
	if (/^https?:\/\//.test(s) || s.startsWith("git@") || s.endsWith(".git")) return { kind: "git", url: s };
	// Otherwise a path relative to (or absolute within) the marketplace repo.
	return { kind: "local", path: path.isAbsolute(s) ? s : path.resolve(marketplaceRoot, s) };
}

// ============================================================================
// Marketplace registry (persisted list of added marketplaces)
// ============================================================================

export interface MarketplaceRecord {
	/** Original location the user added (git URL or local path). */
	location: string;
	/** Local directory to read the manifest from (clone dir for git, else location). */
	dir: string;
}

interface MarketplaceStoreFile {
	marketplaces?: MarketplaceRecord[];
}

/** Read the persisted marketplace list from `<storePath>`. */
export function readMarketplaceStore(storePath: string): MarketplaceRecord[] {
	const parsed = readJson<MarketplaceStoreFile>(storePath);
	return Array.isArray(parsed?.marketplaces) ? parsed.marketplaces : [];
}

/** Write the marketplace list to `<storePath>`. */
export function writeMarketplaceStore(storePath: string, records: MarketplaceRecord[]): void {
	fs.mkdirSync(path.dirname(storePath), { recursive: true });
	fs.writeFileSync(storePath, `${JSON.stringify({ marketplaces: records }, null, 2)}\n`, "utf8");
}
