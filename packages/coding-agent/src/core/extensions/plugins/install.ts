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
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { getAgentDir } from "../../../config.js";
import { defaultPluginDirs } from "../loader.js";
import { hasAnyManifest } from "./formats/index.js";
import type { MarketplacePlatform } from "./formats/types.js";
import { discoverPlugins } from "./index.js";
import {
	candidatePluginDirs,
	installHomeForScope,
	marketplaceCacheDir,
	marketplaceCacheMetaPath,
	marketplaceCacheRoot,
	marketplaceStorePath,
	type PluginInstallScope,
	pluginHomeRoots,
	sanitizeForDir,
} from "./locations.js";
import type { NormalizedPlugin } from "./manifest.js";
import { parsePluginDir } from "./manifest.js";
import {
	type MarketplacePluginSource,
	type MarketplaceRecord,
	parseMarketplaceDir,
	readMarketplaceStore,
	resolvePluginSource,
} from "./marketplace.js";

/**
 * Marketplace registries older versions wrote into the working tree, newest
 * convention first. Still read so a user who added a marketplace before the
 * registry moved to the agent dir does not silently lose it; nothing writes here.
 */
function legacyStorePaths(cwd: string): string[] {
	return [path.join(cwd, ".agents", "marketplaces.json"), path.join(cwd, ".hoocode", "marketplaces.json")];
}

/** Absolute path to the curated default marketplace bundled with hoocode. */
function defaultMarketplaceDir(): string {
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
 * search and refreshed on a TTL thereafter. Refreshing the *index* only changes
 * what is discoverable; installed plugins are still never auto-updated, and
 * plugin sources inside the official index are sha-pinned.
 */
export const WELL_KNOWN_MARKETPLACES: ReadonlyArray<{ name: string; url: string; ref?: string }> = [
	{ name: "claude-plugins-official", url: "https://github.com/anthropics/claude-plugins-official" },
	// The Copilot plugin directory ships both a `.github/plugin/marketplace.json`
	// and a `.claude-plugin/marketplace.json`, so it parses with
	// supportPlatform ["claude", "github"] — the default `github` source.
	{ name: "copilot-plugins", url: "https://github.com/github/copilot-plugins" },
	// GitHub's community index, `.github/plugin/marketplace.json`. Same owner as
	// copilot-plugins, which is what makes it a source-trust decision of the same
	// kind.
	//
	// Pinned to the `marketplace` branch, and that is load-bearing. On `main` a
	// `plugins/<name>/` directory holds only `plugin.json` and `README.md`; the
	// real content lives in the repository's top-level `agents/`, `skills/` and
	// `extensions/` trees and is named from the manifest under the repo's own
	// `extensions["com.github.awesome-copilot"]` namespace. CI *materializes* those
	// references into each plugin directory and publishes the result on
	// `marketplace` (`eng/materialize-plugins.mjs`), which is the branch the
	// vendor's own `copilot plugin install` consumes. Cloning the default branch
	// installed the stub — every entry landed with no capabilities and said so,
	// which read as "hoocode does not model this surface" when it was really
	// "hoocode fetched the wrong ref".
	{ name: "awesome-copilot", url: "https://github.com/github/awesome-copilot", ref: "marketplace" },
];

/**
 * How long a cached marketplace index is considered fresh.
 *
 * Refreshing an *index* is safe to automate: it changes what is discoverable,
 * not what is installed. The rule that an installed plugin is never auto-updated
 * from a remote is untouched — that is the supply-chain boundary, and this is a
 * catalog listing.
 */
const MARKETPLACE_TTL_MS = 24 * 60 * 60 * 1000;

function marketplaceTtlMs(): number {
	const raw = Number(process.env.HOOCODE_MARKETPLACE_TTL_MS);
	return Number.isFinite(raw) && raw >= 0 ? raw : MARKETPLACE_TTL_MS;
}

type CacheMeta = Record<string, string>;

function readCacheMeta(agentDir: string): CacheMeta {
	const parsed = readJsonFile<CacheMeta>(marketplaceCacheMetaPath(agentDir));
	return parsed && typeof parsed === "object" ? parsed : {};
}

function recordFetch(agentDir: string, url: string): void {
	const meta = readCacheMeta(agentDir);
	meta[url] = new Date().toISOString();
	const file = marketplaceCacheMetaPath(agentDir);
	mkdirSync(path.dirname(file), { recursive: true });
	writeFileSync(file, `${JSON.stringify(meta, null, 2)}\n`, "utf8");
}

function isStale(agentDir: string, url: string): boolean {
	const ttl = marketplaceTtlMs();
	if (ttl === 0) return true;
	const at = readCacheMeta(agentDir)[url];
	if (!at) return true;
	const age = Date.now() - Date.parse(at);
	return !Number.isFinite(age) || age > ttl;
}

function readJsonFile<T>(file: string): T | null {
	try {
		return JSON.parse(readFileSync(file, "utf8")) as T;
	} catch {
		return null;
	}
}

/**
 * Bring an existing clone up to date, falling back to a fresh clone when the pull
 * cannot fast-forward (a force-pushed remote, or a corrupt cache).
 *
 * The re-clone goes to a temp directory and is swapped in only on success. An
 * earlier version deleted the cache first, which meant a refresh attempted while
 * offline destroyed a perfectly good index — the opposite of the "offline is
 * non-fatal" property the caller depends on.
 */
async function refreshClone(
	url: string,
	dir: string,
	ref?: string,
): Promise<{ code: number; stdout: string; stderr: string }> {
	const pulled = await execGit(["-C", dir, "pull", "--ff-only"]);
	if (pulled.code === 0) return pulled;

	const staging = mkdtempSync(path.join(os.tmpdir(), "hoo-market-refresh-"));
	const target = path.join(staging, "clone");
	// The re-clone must land on the same ref the cache was created from, or a
	// pinned marketplace silently falls back to the default branch on its first
	// non-fast-forward refresh.
	const cloned = await cloneGitRepo(url, target, ref);
	if (cloned.code !== 0) {
		rmSync(staging, { recursive: true, force: true });
		return cloned;
	}
	rmSync(dir, { recursive: true, force: true });
	mkdirSync(path.dirname(dir), { recursive: true });
	try {
		renameSync(target, dir);
	} catch {
		cpSync(target, dir, { recursive: true });
	}
	rmSync(staging, { recursive: true, force: true });
	return cloned;
}

/**
 * Branch name a cached clone is sitting on, or undefined when it is detached or
 * unreadable (a corrupt cache answers "not the ref I want", which is correct).
 */
async function checkedOutRef(dir: string): Promise<string | undefined> {
	const res = await execGit(["-C", dir, "rev-parse", "--abbrev-ref", "HEAD"]);
	if (res.code !== 0) return undefined;
	const name = res.stdout.trim();
	return name && name !== "HEAD" ? name : undefined;
}

/** Whether `ref` is still a branch on the remote. False on any network failure. */
async function remoteHasRef(url: string, ref: string): Promise<boolean> {
	const res = await execGit(["ls-remote", "--heads", "--", url, ref]);
	return res.code === 0 && res.stdout.trim().length > 0;
}

/**
 * Whether {@link ensureWellKnownMarketplaces} would actually reach the network.
 *
 * Exists so an interactive caller can say "Refreshing marketplace indices…"
 * before a multi-second clone and stay silent when the answer is already on
 * disk. Deliberately does not probe the remote — the ref-mismatch case needs
 * network to detect, so this under-reports rather than paying for a round trip
 * to decide whether to print a line.
 */
export function wellKnownMarketplacesAreStale(agentDir: string = getAgentDir()): boolean {
	return WELL_KNOWN_MARKETPLACES.some(
		(wk) => !existsSync(marketplaceCacheDir(wk.url, agentDir)) || isStale(agentDir, wk.url),
	);
}

export interface EnsureMarketplacesOptions {
	/** Refresh regardless of the TTL. */
	force?: boolean;
	/**
	 * Marketplaces to ensure; defaults to {@link WELL_KNOWN_MARKETPLACES}.
	 *
	 * A seam, and a deliberate one. The ref-pinning rules below — discard a cache
	 * on the wrong branch, fall back when a pin is retired, and do not loop
	 * between those two — are only exercised against a remote whose branches can
	 * be moved and deleted, which no real marketplace will do on demand. Without
	 * this the whole path could only be checked by hand against GitHub, and a
	 * check that needs the network is a check that rots.
	 */
	marketplaces?: ReadonlyArray<{ name: string; url: string; ref?: string }>;
}

/**
 * Make sure every well-known marketplace index is cached and reasonably fresh.
 *
 * Clones what is missing and pulls what has gone stale. Without the TTL the
 * indices were fetched exactly once and never again, so a plugin added upstream
 * stayed permanently invisible — the inherit half of the system quietly stopped
 * inheriting. Offline is non-fatal: each failure is returned and search degrades
 * to whatever is already on disk.
 *
 * The returned strings are notes for the caller to surface, not fatal errors —
 * a retired ref that fell back to the default branch reports here too, because
 * the consequence (entries that install with no capabilities) is otherwise
 * invisible and inexplicable.
 */
export async function ensureWellKnownMarketplaces(
	agentDir: string = getAgentDir(),
	options: EnsureMarketplacesOptions = {},
): Promise<string[]> {
	const errors: string[] = [];
	for (const wk of options.marketplaces ?? WELL_KNOWN_MARKETPLACES) {
		const dir = marketplaceCacheDir(wk.url, agentDir);
		let present = existsSync(dir);
		// A cache is keyed by URL, not by ref, so a marketplace that gains (or
		// changes) a pin finds a clone of the wrong branch sitting in its slot. A
		// `pull --ff-only` there would faithfully keep updating the wrong branch
		// forever, so the mismatch is resolved the only way it can be: discard and
		// clone the ref that is asked for.
		//
		// Guarded on the ref still existing upstream, which is not paranoia — it is
		// what stops a loop. When a pin is retired, the fallback below leaves a
		// default-branch clone in the slot; that clone mismatches the pin forever,
		// so an unguarded check would discard and re-download it on every single
		// run. Probing costs one `ls-remote` and only on the mismatch path.
		if (present && wk.ref && (await checkedOutRef(dir)) !== wk.ref && (await remoteHasRef(wk.url, wk.ref))) {
			rmSync(dir, { recursive: true, force: true });
			present = false;
		}
		if (present && !options.force && !isStale(agentDir, wk.url)) continue;

		mkdirSync(path.dirname(dir), { recursive: true });
		let res = present ? await refreshClone(wk.url, dir, wk.ref) : await cloneGitRepo(wk.url, dir, wk.ref);
		// A pinned ref that cannot be cloned is the one failure worth retrying
		// differently. The pin names someone else's distribution branch — for
		// awesome-copilot, a name held in a workflow env var — so it can be renamed
		// or retired without warning, and a cold cache would then lose that
		// marketplace outright rather than degrade. A warm cache already degrades
		// correctly (the stale copy is kept below, and it is *built* content, which
		// beats an unbuilt default branch), so this applies to the cold case only.
		let degraded: string | undefined;
		if (res.code !== 0 && !present && wk.ref) {
			rmSync(dir, { recursive: true, force: true });
			const fallback = await cloneGitRepo(wk.url, dir);
			if (fallback.code === 0) {
				degraded =
					`${wk.name}: branch "${wk.ref}" is unavailable, so the default branch was used instead. ` +
					"Plugin entries there may be unbuilt and install with no capabilities.";
				res = fallback;
			}
		}
		if (res.code !== 0) {
			// A failed *refresh* keeps the stale copy: an out-of-date index is far
			// more useful than none, and the network may simply be down.
			if (!present) rmSync(dir, { recursive: true, force: true });
			errors.push(`${wk.name}: ${(res.stderr || res.stdout).trim()}`);
			continue;
		}
		if (degraded) errors.push(degraded);
		recordFetch(agentDir, wk.url);
	}
	return errors;
}

/**
 * Refresh every cached index — well-known and user-added — ignoring the TTL.
 * The explicit half of the freshness story, for when a user knows the upstream
 * changed and does not want to wait out the interval.
 */
export async function refreshMarketplaces(
	cwd: string,
	agentDir: string = getAgentDir(),
): Promise<{ refreshed: string[]; errors: string[] }> {
	const errors = await ensureWellKnownMarketplaces(agentDir, { force: true });
	const refreshed = WELL_KNOWN_MARKETPLACES.filter((wk) => existsSync(marketplaceCacheDir(wk.url, agentDir))).map(
		(wk) => wk.name,
	);

	// User-added marketplaces are cached the same way when they came from git; a
	// local-path marketplace is read in place and has nothing to refresh.
	for (const record of readMarketplaceRecords(cwd, agentDir)) {
		const isCachedClone = record.dir.startsWith(marketplaceCacheRoot(agentDir));
		if (!isCachedClone || WELL_KNOWN_MARKETPLACES.some((wk) => wk.url === record.location)) continue;
		const res = await refreshClone(record.location, record.dir);
		if (res.code !== 0) errors.push(`${record.location}: ${(res.stderr || res.stdout).trim()}`);
		else {
			recordFetch(agentDir, record.location);
			refreshed.push(record.location);
		}
	}
	return { refreshed, errors };
}

/**
 * All marketplace records in effect: the bundled default first (curated,
 * trusted), then user-added ones from `.agents/` (falling back to legacy
 * `.hoocode/`). Deduplicated by directory.
 */
export function readMarketplaceRecords(cwd: string, agentDir: string = getAgentDir()): MarketplaceRecord[] {
	const records: MarketplaceRecord[] = [];
	const def = defaultMarketplaceRecord();
	if (existsSync(def.dir)) records.push(def);

	// Well-known marketplaces participate once their index is cached locally
	// (see ensureWellKnownMarketplaces; SearchPlugins fetches lazily).
	for (const wk of WELL_KNOWN_MARKETPLACES) {
		const dir = marketplaceCacheDir(wk.url, agentDir);
		if (existsSync(dir) && !records.some((x) => x.dir === dir)) {
			records.push({ location: wk.url, dir });
		}
	}

	// The registry lives in the agent dir. Project-local ones are a migration
	// read-path: merged in, never written back, so an older setup keeps working.
	const user = [
		...readMarketplaceStore(marketplaceStorePath(agentDir)),
		...legacyStorePaths(cwd).flatMap((p) => readMarketplaceStore(p)),
	];
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
	/** Resolved source kind, for display and gating. `npm`/`archive` are listed but not installable. */
	sourceKind: "local" | "git" | "git-subdir" | "npm" | "archive";
	marketplaceName: string;
	marketplaceRoot: string;
	/** Platforms this entry targets (per-entry hint, else the marketplace's). */
	supportPlatform: MarketplacePlatform[];
}

/** Every plugin offered across all registered marketplaces (first marketplace wins on name clash). */
export function listAvailablePlugins(cwd: string, agentDir: string = getAgentDir()): AvailablePlugin[] {
	const out: AvailablePlugin[] = [];
	const seen = new Set<string>();
	for (const rec of readMarketplaceRecords(cwd, agentDir)) {
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
export function findAvailablePlugin(
	cwd: string,
	name: string,
	agentDir: string = getAgentDir(),
): AvailablePlugin | undefined {
	return listAvailablePlugins(cwd, agentDir).find((p) => p.name === name);
}

/** All currently installed plugins (project + global plugin dirs). */
export function listInstalledPlugins(cwd: string, agentDir: string = getAgentDir()): NormalizedPlugin[] {
	return discoverPlugins(defaultPluginDirs(cwd, agentDir));
}

/**
 * Whether `name` is already installed, by either name it can go by.
 *
 * A marketplace entry name and the plugin's own manifest `name` are allowed to
 * differ — both vendors say so outright — and installs land in a directory named
 * for the *entry*, while discovery reports the *manifest* id. Matching ids alone
 * meant a plugin installed as `42crunch-api-security-testing` (manifest id
 * `api-security-testing`) never counted as installed, so every InstallPlugin
 * call for the catalog name re-cloned it.
 */
export function isPluginInstalled(cwd: string, name: string, agentDir: string = getAgentDir()): boolean {
	if (listInstalledPlugins(cwd, agentDir).some((p) => p.id === name)) return true;
	return candidatePluginDirs(cwd, name, agentDir).some((p) => existsSync(p));
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
 * Materialize exactly `sha` at `dest`, without cloning the repository's history.
 *
 * `git fetch --depth 1 <sha>` is what makes honoring a pin affordable: a pinned
 * commit is usually not the tip of any branch, so the alternative is a full
 * clone — 55s on a large plugin repo against ~1s here, on sources where two
 * thirds of the official Claude marketplace is pinned. Servers that refuse a
 * by-sha fetch (AWS CodeCommit and other non-GitHub hosts) fall back to a clone
 * deep enough to reach the commit, then check it out.
 */
async function fetchGitCommit(
	url: string,
	dest: string,
	sha: string,
	ref?: string,
): Promise<{ code: number; stdout: string; stderr: string }> {
	mkdirSync(dest, { recursive: true });
	const init = await execGit(["init", "--quiet", dest]);
	if (init.code === 0) {
		const remote = await execGit(["-C", dest, "remote", "add", "origin", url]);
		if (remote.code === 0) {
			const fetched = await execGit(["-C", dest, "fetch", "--quiet", "--depth", "1", "origin", sha]);
			if (fetched.code === 0) {
				const checkout = await execGit(["-C", dest, "checkout", "--quiet", "FETCH_HEAD"]);
				if (checkout.code === 0) return checkout;
			}
		}
	}

	// Fallback: a full clone (shallow cannot reach an arbitrary commit), then the
	// pin. `ref` narrows it when the entry named one.
	rmSync(dest, { recursive: true, force: true });
	const cloned = ref
		? await execGit(["clone", "--branch", ref, "--", url, dest])
		: await execGit(["clone", "--", url, dest]);
	if (cloned.code !== 0) return cloned;
	return execGit(["-C", dest, "checkout", "--quiet", sha]);
}

/**
 * Clone a git repository into `dest` at the requested pin.
 *
 * **`sha` outranks `ref`.** Both vendors define it that way ("when both `ref`
 * and `sha` are set, the `sha` is the effective pin"), and a marketplace that
 * ships both is stating the exact commit it vouches for — a tag can be moved or
 * re-pointed after the catalog pinned it, so preferring `ref` installed a
 * different commit than the index promised. It is not hypothetical: the official
 * catalog's `42crunch-api-security-testing` pins `30287f5` while its `v1.5.5`
 * tag now resolves to `faf5305`.
 *
 * Returns the git result; on failure `dest` may be partially created.
 */
async function cloneGitRepo(
	url: string,
	dest: string,
	ref?: string,
	sha?: string,
): Promise<{ code: number; stdout: string; stderr: string }> {
	if (sha) return fetchGitCommit(url, dest, sha, ref);
	if (ref) return execGit(["clone", "--branch", ref, "--depth", "1", "--", url, dest]);
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

export interface InstallOptions {
	/**
	 * Where the plugin lands. Defaults to `user`. The human path (`/plugin
	 * install`) asks; the autonomous path passes the `pluginInstallScope` setting.
	 */
	scope?: PluginInstallScope;
}

export interface InstallOutcome {
	installed: boolean;
	/** Install destination directory (when installed). */
	dest?: string;
	/** The scope the plugin was installed at. */
	scope?: PluginInstallScope;
	/**
	 * The plugin's own manifest id, when it differs from the marketplace entry
	 * name it was installed by. This is the id `ListPlugins` reports, so the
	 * caller can name both rather than leaving the model to reconcile them.
	 */
	id?: string;
	/** Platforms the installed plugin supports (read back from disk). */
	supportPlatform?: MarketplacePlatform[];
	/** Human-readable summary suitable for a tool result or notification. */
	message: string;
}

/** Capability surfaces hoocode can actually load out of an installed plugin. */
function loadableCapabilities(plugin: NormalizedPlugin): string[] {
	return [
		plugin.skillsDir && "skills",
		plugin.commandsDir && "commands",
		plugin.agentsDir && "subagents",
		plugin.themesDir && "themes",
		plugin.hooks && "hooks",
		plugin.mcpServers && Object.keys(plugin.mcpServers).length > 0 && "MCP servers",
		plugin.providers?.length && "providers",
		// Not loaded at session start like the rest — a canvas has no passive half —
		// but it is unquestionably something the install brought, and reporting it
		// is what stops a canvas plugin reading as an install that did nothing.
		plugin.canvasExtensions?.length && "canvases",
	].filter((c): c is string => typeof c === "string");
}

/**
 * Install an available plugin by name into the consumption home for
 * `options.scope` — `~/.agents/plugins/` for `user` (the default), or
 * `<cwd>/.agents/plugins/` for `project`.
 *
 * `user` is the default for a reason worth keeping in view: a plugin is portable
 * and reusable across projects, so installing into the repo hides the capability
 * from every other checkout and puts content into `git status` that has nothing
 * to do with the change being made. `project` inverts both of those on purpose —
 * it is how a team pins a plugin to a repository — so it is chosen, never
 * defaulted into.
 *
 * Copies local sources; clones git sources. Transparent + reversible by
 * construction — the plugin lands in a named directory and
 * {@link uninstallPlugin} removes it from either scope. Callers activate the
 * result (live activation via AgentSession.activatePlugin, or a reload).
 */
export async function installAvailablePlugin(
	cwd: string,
	name: string,
	agentDir: string = getAgentDir(),
	options: InstallOptions = {},
): Promise<InstallOutcome> {
	const found = findAvailablePlugin(cwd, name, agentDir);
	if (!found) return { installed: false, message: `Plugin "${name}" not found in any registered marketplace.` };

	const scope = options.scope ?? "user";
	const resolved = resolvePluginSource(found.source, found.marketplaceRoot);
	const home = installHomeForScope(scope, cwd, agentDir);
	const dest = path.join(home, sanitizeForDir(name));
	rmSync(dest, { recursive: true, force: true });
	mkdirSync(home, { recursive: true });

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
	} else if (resolved.kind === "npm") {
		return {
			installed: false,
			message:
				`"${name}" is published as an npm package (${resolved.spec}), a source type hoocode cannot install yet. ` +
				"The entry is listed so you know it exists; install it through the vendor CLI, or ask the marketplace " +
				"for a git source.",
		};
	} else {
		return {
			installed: false,
			message:
				`"${name}" is published as a zip archive (${resolved.url}), a source type hoocode cannot install yet. ` +
				"The entry is listed so you know it exists; install it through the vendor CLI, or ask the marketplace " +
				"for a git source.",
		};
	}

	// Drop the clone's `.git`. Nothing reads it — an installed plugin is never
	// updated from its remote, by design (that is the supply-chain boundary) — and
	// keeping it actively breaks project scope: a repository nested inside the
	// user's repository is an embedded repo, so `git add` writes a gitlink instead
	// of the plugin's files and the install cannot actually be committed. Which is
	// the one thing project scope is for. Cheaper installs at user scope too.
	rmSync(path.join(dest, ".git"), { recursive: true, force: true });

	let parsed = parsePluginDir(dest);
	if (!hasAnyManifest(dest)) {
		// Manifest-less plugin dir (some marketplaces index bare capability trees,
		// e.g. a plugin that is just a `skills/` directory). Such a directory now
		// *parses* on its own — the vendors make the manifest optional — but the
		// entry's name and description live only in the marketplace index, and the
		// derived id would be the sanitized directory name. Synthesize a native
		// manifest so both survive the install.
		const manifestDir = path.join(dest, ".agents-plugin");
		mkdirSync(manifestDir, { recursive: true });
		writeFileSync(
			path.join(manifestDir, "plugin.json"),
			`${JSON.stringify(
				{ name: found.name, ...(found.description ? { description: found.description } : {}) },
				null,
				2,
			)}\n`,
			"utf8",
		);
		parsed = parsePluginDir(dest);
	}
	if (!parsed) {
		rmSync(dest, { recursive: true, force: true });
		return { installed: false, message: `Installed source for "${name}" has no recognizable plugin manifest.` };
	}
	const capabilities = loadableCapabilities(parsed);
	let message =
		`Installed "${name}" from marketplace "${found.marketplaceName}" ` +
		`(${parsed.supportPlatform.join(", ")}) to ${dest} [${scope} scope]. ` +
		`Remove it with UninstallPlugin.`;
	if (scope === "project") {
		// Project scope puts the plugin in the working tree, which is the point —
		// but it is also a `git status` entry and, once committed, code that runs
		// for whoever clones next. Executable capabilities are the half that
		// matters: hoocode has no workspace-trust gate for `<cwd>/.agents/plugins`
		// (see docs/plugin-system-architecture.md §5.9), so a committed hook or MCP
		// server loads for collaborators without a prompt.
		const executables = [parsed.hooks && "hooks", parsed.mcpServers && "MCP servers"].filter(Boolean);
		message += ` It is now part of the working tree — commit it to share it with collaborators.`;
		if (executables.length > 0) {
			message +=
				` It carries ${executables.join(" and ")}, which run for anyone who clones the repository;` +
				` keep it out of version control if that is not what you want.`;
		}
	}
	if (parsed.id !== name) {
		// The entry name and the manifest name are allowed to differ, and the
		// difference is otherwise invisible until ListPlugins reports an id the
		// caller never asked for.
		message += ` Listed by ListPlugins as "${parsed.id}"; either name uninstalls it.`;
	}
	if (parsed.canvasExtensions?.length) {
		// A canvas is the one capability that is not loaded on activation: it has no
		// passive half, so it waits for someone to open it. Saying so here is the
		// difference between "installed and inert" and "installed, run /canvas".
		const ids = parsed.canvasExtensions.map((extension) => extension.id).join(", ");
		message += ` Canvases: ${ids} — open one with /canvas open <id>.`;
	}
	if (capabilities.length === 0) {
		// An install that adds nothing should not read as an install that worked.
		const surfaces = parsed.unsupportedSurfaces?.length
			? ` Unsupported surfaces present: ${parsed.unsupportedSurfaces.join(", ")}.`
			: "";
		message +=
			" Note: it contributes no capabilities hoocode can load" +
			` (no skills, commands, subagents, canvases, hooks or MCP servers).${surfaces}`;
	}
	return {
		installed: true,
		dest,
		scope,
		...(parsed.id !== name ? { id: parsed.id } : {}),
		supportPlatform: parsed.supportPlatform,
		message,
	};
}

export interface UninstallOutcome {
	removed: boolean;
	message: string;
}

/** True when `target` is `root` or sits inside it. */
function isUnderDir(target: string, root: string): boolean {
	const normalized = path.resolve(root);
	if (path.resolve(target) === normalized) return true;
	return path.resolve(target).startsWith(normalized.endsWith(path.sep) ? normalized : `${normalized}${path.sep}`);
}

/**
 * Remove an installed or authored plugin from every location it could occupy:
 * the consumption home, both production homes, and the legacy project-local
 * directories that older versions wrote into.
 *
 * `name` is matched two ways, because a plugin has two names: the marketplace
 * entry name it was installed under (which is what the directory is called) and
 * the manifest id `ListPlugins` reports. Resolving only the first left a plugin
 * whose names differ impossible to remove through the tool surface — the id the
 * model was shown was the one uninstall did not accept.
 *
 * Removal by id stays inside {@link pluginHomeRoots}, so a plugin discovered in
 * `<cwd>/.claude/skills` — repository content the team committed, which hoocode
 * did not install — is never deleted out of the working tree.
 */
export function uninstallPlugin(cwd: string, name: string, agentDir: string = getAgentDir()): UninstallOutcome {
	const homes = pluginHomeRoots(cwd, agentDir);
	const byManifestId = listInstalledPlugins(cwd, agentDir)
		.filter((p) => p.id === name)
		.map((p) => p.root)
		.filter((root) => homes.some((home) => isUnderDir(root, home)));
	const candidates = [...new Set([...candidatePluginDirs(cwd, name, agentDir), ...byManifestId])];
	const present = candidates.filter((p) => existsSync(p));
	if (present.length === 0) return { removed: false, message: `Plugin "${name}" is not installed.` };
	for (const p of present) rmSync(p, { recursive: true, force: true });
	return { removed: true, message: `Removed "${name}".` };
}
