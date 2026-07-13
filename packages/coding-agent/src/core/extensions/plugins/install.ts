/**
 * Plugin install/uninstall/discovery — the shared engine behind both the
 * `/plugin` slash command (human-driven) and the model-facing lifecycle tools
 * (`SearchPlugins`, `InstallPlugin`, ...). Keeping it in one place means the two
 * surfaces can never drift on where plugins live or how sources resolve.
 *
 * Trust model (see docs/plugin-system-spec.md): *adding* a marketplace is the
 * human trust boundary; *installing* from an already-trusted marketplace is the
 * model's discretion (package-manager model), and must stay transparent and
 * reversible. This module performs the mechanical install/remove; the gating
 * (announce, injection carve-out) lives with the callers.
 */

import { spawn } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { getAgentDir } from "../../../config.js";
import { defaultPluginDirs } from "../loader.js";
import type { MarketplacePlatform } from "./formats/types.js";
import { discoverPlugins } from "./index.js";
import type { NormalizedPlugin } from "./manifest.js";
import { parsePluginDir } from "./manifest.js";
import {
	type MarketplacePluginSource,
	type MarketplaceRecord,
	parseMarketplaceDir,
	readMarketplaceStore,
	resolvePluginSource,
} from "./marketplace.js";

/** `.agents/` is the primary, cross-vendor home for installed plugins and the added-marketplace registry. */
export function installedPluginsDir(cwd: string): string {
	return path.join(cwd, ".agents", "plugins");
}

function marketplaceStorePath(cwd: string): string {
	return path.join(cwd, ".agents", "marketplaces.json");
}

function legacyStorePath(cwd: string): string {
	return path.join(cwd, ".hoocode", "marketplaces.json");
}

/** Filesystem-safe directory name derived from a plugin name (matches the `/plugin` command). */
export function sanitizeForDir(s: string): string {
	return s.replace(/[^a-zA-Z0-9._-]+/g, "_").slice(0, 80);
}

/** Absolute path to the curated default marketplace bundled with hoocode. */
export function defaultMarketplaceDir(): string {
	return fileURLToPath(new URL("./default-marketplace/", import.meta.url));
}

/** The default marketplace record — always present so source-level trust is meaningful out of the box. */
export function defaultMarketplaceRecord(): MarketplaceRecord {
	return { location: "hoocode-default", dir: defaultMarketplaceDir() };
}

/**
 * Curated well-known marketplaces, trusted out of the box. Shipping an entry
 * here is a maintainer-level trust decision (the human half of the "adding a
 * marketplace is the human trust boundary" rule) — keep this list short and
 * high-trust. Indices are cloned lazily into the marketplace cache on first
 * search and never auto-updated afterwards (plugin sources inside the official
 * index are sha-pinned).
 */
export const WELL_KNOWN_MARKETPLACES: ReadonlyArray<{ name: string; url: string }> = [
	{ name: "claude-plugins-official", url: "https://github.com/anthropics/claude-plugins-official" },
];

/** Local cache directory for a marketplace fetched from `url` (same convention as `/plugin marketplace add`). */
export function marketplaceCacheDir(cwd: string, url: string): string {
	return path.join(cwd, ".agents", "marketplace-cache", sanitizeForDir(url));
}

/**
 * Clone any well-known marketplace index that is not in the local cache yet.
 * No-op (and no network) when every index is already cached. Returns one error
 * string per marketplace that could not be fetched (offline is non-fatal —
 * search degrades to the marketplaces already available).
 */
export async function ensureWellKnownMarketplaces(cwd: string): Promise<string[]> {
	const errors: string[] = [];
	for (const wk of WELL_KNOWN_MARKETPLACES) {
		const dir = marketplaceCacheDir(cwd, wk.url);
		if (existsSync(dir)) continue;
		mkdirSync(path.dirname(dir), { recursive: true });
		const res = await cloneGitRepo(wk.url, dir);
		if (res.code !== 0) {
			rmSync(dir, { recursive: true, force: true });
			errors.push(`${wk.name}: ${(res.stderr || res.stdout).trim()}`);
		}
	}
	return errors;
}

/**
 * All marketplace records in effect: the bundled default first (curated,
 * trusted), then user-added ones from `.agents/` (falling back to legacy
 * `.hoocode/`). Deduplicated by directory.
 */
export function readMarketplaceRecords(cwd: string): MarketplaceRecord[] {
	const records: MarketplaceRecord[] = [];
	const def = defaultMarketplaceRecord();
	if (existsSync(def.dir)) records.push(def);

	// Well-known marketplaces participate once their index is cached locally
	// (see ensureWellKnownMarketplaces; SearchPlugins fetches lazily).
	for (const wk of WELL_KNOWN_MARKETPLACES) {
		const dir = marketplaceCacheDir(cwd, wk.url);
		if (existsSync(dir) && !records.some((x) => x.dir === dir)) {
			records.push({ location: wk.url, dir });
		}
	}

	const primary = readMarketplaceStore(marketplaceStorePath(cwd));
	const user =
		primary.length > 0 || existsSync(marketplaceStorePath(cwd))
			? primary
			: readMarketplaceStore(legacyStorePath(cwd));
	for (const r of user) {
		if (!records.some((x) => x.dir === r.dir)) records.push(r);
	}
	return records;
}

/** A plugin offered by some registered marketplace (not necessarily installed). */
export interface AvailablePlugin {
	name: string;
	description?: string;
	/** Relative path, git URL, `npm:<spec>`, or structured source object. */
	source: MarketplacePluginSource;
	/** Resolved source kind, for display and gating. */
	sourceKind: "local" | "git" | "git-subdir" | "npm";
	marketplaceName: string;
	marketplaceRoot: string;
	/** Platforms this entry targets (per-entry hint, else the marketplace's). */
	supportPlatform: MarketplacePlatform[];
}

/** Every plugin offered across all registered marketplaces (first marketplace wins on name clash). */
export function listAvailablePlugins(cwd: string): AvailablePlugin[] {
	const out: AvailablePlugin[] = [];
	const seen = new Set<string>();
	for (const rec of readMarketplaceRecords(cwd)) {
		const market = parseMarketplaceDir(rec.dir);
		if (!market) continue;
		for (const entry of market.plugins) {
			if (seen.has(entry.name)) continue;
			seen.add(entry.name);
			out.push({
				name: entry.name,
				description: entry.description,
				source: entry.source,
				sourceKind: resolvePluginSource(entry.source, market.root).kind,
				marketplaceName: market.name,
				marketplaceRoot: market.root,
				supportPlatform: entry.supportPlatform ?? market.supportPlatform,
			});
		}
	}
	return out;
}

/** Find a single available plugin by exact name. */
export function findAvailablePlugin(cwd: string, name: string): AvailablePlugin | undefined {
	return listAvailablePlugins(cwd).find((p) => p.name === name);
}

/** All currently installed plugins (project + global plugin dirs). */
export function listInstalledPlugins(cwd: string, agentDir: string = getAgentDir()): NormalizedPlugin[] {
	return discoverPlugins(defaultPluginDirs(cwd, agentDir));
}

/** Whether a plugin with `name` (by id) is already installed. */
export function isPluginInstalled(cwd: string, name: string, agentDir: string = getAgentDir()): boolean {
	return listInstalledPlugins(cwd, agentDir).some((p) => p.id === name);
}

function execGit(args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
	return new Promise((resolve) => {
		const child = spawn("git", args);
		let stdout = "";
		let stderr = "";
		child.stdout?.on("data", (d) => {
			stdout += d.toString();
		});
		child.stderr?.on("data", (d) => {
			stderr += d.toString();
		});
		child.on("error", (e) => resolve({ code: 1, stdout, stderr: stderr || String(e) }));
		child.on("close", (code) => resolve({ code: code ?? 1, stdout, stderr }));
	});
}

/**
 * Clone a git repository into `dest`.
 * If `ref` is provided, does a shallow clone of that branch/tag.
 * If only `sha` is provided, does a full clone and checks out the commit.
 * Returns the clone result; on failure `dest` may be partially created.
 */
async function cloneGitRepo(
	url: string,
	dest: string,
	ref?: string,
	sha?: string,
): Promise<{ code: number; stdout: string; stderr: string }> {
	if (ref) {
		return execGit(["clone", "--branch", ref, "--depth", "1", "--", url, dest]);
	}
	if (sha) {
		const cloneRes = await execGit(["clone", "--", url, dest]);
		if (cloneRes.code !== 0) return cloneRes;
		return execGit(["-C", dest, "checkout", "--quiet", sha]);
	}
	return execGit(["clone", "--depth", "1", "--", url, dest]);
}

/**
 * Resolve a git-subdir source: clone the repo, checkout the requested ref/sha,
 * and copy the subdirectory to `dest`. Cleans up the temporary clone on failure.
 */
async function installGitSubdir(
	url: string,
	subdir: string,
	dest: string,
	ref?: string,
	sha?: string,
): Promise<{ ok: true } | { ok: false; message: string }> {
	const tmpDir = mkdtempSync(path.join(os.tmpdir(), "hoo-plugin-clone-"));
	try {
		const cloneRes = await cloneGitRepo(url, tmpDir, ref, sha);
		if (cloneRes.code !== 0) {
			return { ok: false, message: `git clone failed: ${cloneRes.stderr || cloneRes.stdout}`.trim() };
		}
		const src = path.resolve(tmpDir, subdir);
		if (!existsSync(src)) {
			return { ok: false, message: `Plugin subdirectory not found in cloned repo: ${subdir}` };
		}
		cpSync(src, dest, { recursive: true });
		return { ok: true };
	} finally {
		rmSync(tmpDir, { recursive: true, force: true });
	}
}

export interface InstallOutcome {
	installed: boolean;
	/** Install destination directory (when installed). */
	dest?: string;
	/** Platforms the installed plugin supports (read back from disk). */
	supportPlatform?: MarketplacePlatform[];
	/** Human-readable summary suitable for a tool result or notification. */
	message: string;
}

/**
 * Install an available plugin by name into `.agents/plugins`. Copies local
 * sources; clones git sources. Transparent + reversible by construction — the
 * plugin lands in a named directory and {@link uninstallPlugin} removes it.
 * Callers activate the result (live activation via AgentSession.activatePlugin,
 * or a reload).
 */
export async function installAvailablePlugin(cwd: string, name: string): Promise<InstallOutcome> {
	const found = findAvailablePlugin(cwd, name);
	if (!found) return { installed: false, message: `Plugin "${name}" not found in any registered marketplace.` };

	const resolved = resolvePluginSource(found.source, found.marketplaceRoot);
	const dest = path.join(installedPluginsDir(cwd), sanitizeForDir(name));
	rmSync(dest, { recursive: true, force: true });
	mkdirSync(installedPluginsDir(cwd), { recursive: true });

	if (resolved.kind === "local") {
		if (!existsSync(resolved.path)) {
			return { installed: false, message: `Plugin source path not found: ${resolved.path}` };
		}
		cpSync(resolved.path, dest, { recursive: true });
	} else if (resolved.kind === "git") {
		const res = await cloneGitRepo(resolved.url, dest, resolved.ref, resolved.sha);
		if (res.code !== 0) {
			rmSync(dest, { recursive: true, force: true });
			return { installed: false, message: `git clone failed: ${res.stderr || res.stdout}`.trim() };
		}
	} else if (resolved.kind === "git-subdir") {
		const res = await installGitSubdir(resolved.url, resolved.path, dest, resolved.ref, resolved.sha);
		if (!res.ok) {
			rmSync(dest, { recursive: true, force: true });
			return { installed: false, message: res.message };
		}
	} else {
		return { installed: false, message: `npm plugin sources are not supported yet (${resolved.spec}).` };
	}

	const parsed = parsePluginDir(dest);
	if (!parsed) {
		rmSync(dest, { recursive: true, force: true });
		return { installed: false, message: `Installed source for "${name}" has no recognizable plugin manifest.` };
	}
	return {
		installed: true,
		dest,
		supportPlatform: parsed.supportPlatform,
		message:
			`Installed "${name}" from marketplace "${found.marketplaceName}" ` +
			`(${parsed.supportPlatform.join(", ")}) to ${path.relative(cwd, dest) || dest}. ` +
			`Remove it with UninstallPlugin.`,
	};
}

export interface UninstallOutcome {
	removed: boolean;
	message: string;
}

/** Remove an installed plugin from `.agents/plugins` (and the legacy `.hoocode/plugins`). */
export function uninstallPlugin(cwd: string, name: string): UninstallOutcome {
	const candidates = [
		path.join(installedPluginsDir(cwd), sanitizeForDir(name)),
		path.join(cwd, ".hoocode", "plugins", sanitizeForDir(name)),
	];
	const present = candidates.filter((p) => existsSync(p));
	if (present.length === 0) return { removed: false, message: `Plugin "${name}" is not installed.` };
	for (const p of present) rmSync(p, { recursive: true, force: true });
	return { removed: true, message: `Removed "${name}".` };
}
