/**
 * Plugin marketplaces — install plugins from a curated index hosted in a git repo
 * (or a local directory).
 *
 * A marketplace is a directory containing an index manifest in one of two formats:
 *  - Claude:  `.claude-plugin/marketplace.json`
 *  - Copilot: `.github/marketplace.json`  (Copilot-style git index)
 *
 * Both use the same shape: `{ name?, owner?, plugins: [{ name, source, description? }] }`.
 * A plugin `source` is either a path relative to the marketplace root (a plugin
 * directory inside the repo), a git URL, or an `npm:<spec>` reference.
 */

import * as fs from "node:fs";
import * as path from "node:path";

export const CLAUDE_MARKETPLACE_FILE = path.join(".claude-plugin", "marketplace.json");
export const COPILOT_MARKETPLACE_FILE = path.join(".github", "marketplace.json");

export interface MarketplacePluginEntry {
	name: string;
	/** Relative path, git URL, or `npm:<spec>`. */
	source: string;
	description?: string;
}

export interface NormalizedMarketplace {
	name: string;
	format: "claude" | "copilot";
	/** Absolute path to the marketplace directory. */
	root: string;
	manifestPath: string;
	plugins: MarketplacePluginEntry[];
}

interface RawMarketplace {
	name?: string;
	owner?: string;
	plugins?: Array<{ name?: string; source?: string; description?: string }>;
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
 * Parse a marketplace directory. Claude format wins when both files are present.
 * Returns null if neither manifest exists or it is invalid.
 */
export function parseMarketplaceDir(dir: string): NormalizedMarketplace | null {
	const claudePath = path.join(dir, CLAUDE_MARKETPLACE_FILE);
	const copilotPath = path.join(dir, COPILOT_MARKETPLACE_FILE);

	let manifestPath: string;
	let format: NormalizedMarketplace["format"];
	if (fs.existsSync(claudePath)) {
		manifestPath = claudePath;
		format = "claude";
	} else if (fs.existsSync(copilotPath)) {
		manifestPath = copilotPath;
		format = "copilot";
	} else {
		return null;
	}

	const raw = readJson<RawMarketplace>(manifestPath);
	if (!raw) return null;

	const plugins: MarketplacePluginEntry[] = [];
	for (const entry of raw.plugins ?? []) {
		if (entry?.name && entry.source) {
			plugins.push({ name: entry.name, source: entry.source, description: entry.description });
		}
	}

	return {
		name: (raw.name ?? path.basename(dir)).trim() || path.basename(dir),
		format,
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
