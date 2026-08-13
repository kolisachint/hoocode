/**
 * Where plugins live on disk.
 *
 * Three distinct roles, deliberately named apart — an earlier design called all
 * three "staging", which is how the wrong directory ends up in code:
 *
 *   Draft dir          Ephemeral. Holds an authored plugin while the eval gates
 *                      run, and is deleted on any failure. Nothing here is ever
 *                      loaded; it is promoted into a production home or thrown
 *                      away. {@link makeDraftDir}
 *   Consumption home   Persistent. Where marketplace installs land, in whatever
 *                      format the marketplace served. hoocode installing for
 *                      itself, at the scope the caller chose
 *                      ({@link installHomeForScope}).
 *   Production home    Persistent, global, per platform. Where a plugin hoocode
 *                      *authored* lives. {@link productionPluginDir}
 *
 * Draft and production are user-scoped without exception. Consumption is the one
 * that takes a scope, and `user` stays the default: a plugin is portable,
 * versioned and reusable across projects, so the working tree is the wrong place
 * for it *by default* — the capability would be invisible in every other repo,
 * and an autonomous install would dirty `git status` with content unrelated to
 * the task. `project` exists for the other case, where a team wants the plugin
 * pinned in the repo and shared with collaborators, and is a deliberate choice
 * someone makes per install or per setting rather than a default anything falls
 * into.
 *
 * Authoring stays user-scoped regardless (architecture doc §5.5): a project
 * destination for an *authored* plugin has no coherent production home, whereas
 * an installed plugin has an obvious one.
 *
 * The two production homes are asymmetric because the vendors are. Claude Code
 * discovers `~/.claude/skills/<id>/` in place, with no install step, so an
 * authored plugin is live there on its next session. Copilot CLI has no
 * equivalent — `copilot plugin install` copies into a cache it owns — so a
 * github artifact lives in a hoocode-owned home and reaches the ecosystem
 * through the publish lane instead.
 *
 * See docs/plugin-system-architecture.md §5.3 and §8.3.
 */

import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { getAgentDir } from "../../../config.js";
import type { PluginPlatform } from "./formats/platform-targets.js";

/** Filesystem-safe directory name derived from a plugin name (matches the `/plugin` command). */
export function sanitizeForDir(s: string): string {
	return s.replace(/[^a-zA-Z0-9._-]+/g, "_").slice(0, 80);
}

/**
 * Home root the global surfaces hang off. Derived from the agent dir rather than
 * `homedir()` so `HOOCODE_AGENT_DIR` relocates every plugin location together —
 * which is what makes these paths testable.
 */
function homeRoot(agentDir: string): string {
	return path.dirname(agentDir);
}

/** `~/.agents/plugins/` — where user-scoped marketplace installs land. Format-agnostic. */
export function consumptionPluginsDir(agentDir: string = getAgentDir()): string {
	return path.join(homeRoot(agentDir), ".agents", "plugins");
}

/**
 * `<cwd>/.agents/plugins/` — where project-scoped installs land.
 *
 * Also the home older versions installed *everything* into, back when scope was
 * not a choice (see docs/plugin-system-architecture.md §5.4). It stayed on the
 * discovery path the whole time, which is why project scope needs no loader
 * change: `defaultPluginDirs` already reads it, ahead of the user home, so a
 * project-scoped plugin shadows a user-scoped one of the same id.
 */
export function projectPluginsDir(cwd: string): string {
	return path.join(cwd, ".agents", "plugins");
}

/**
 * Where a marketplace install goes.
 *
 * hoocode has no `enabledPlugins`-style registry — a plugin on the discovery
 * path is enabled — so scope is a *destination*, not a flag written elsewhere:
 *
 *   user     `~/.agents/plugins/`   portable across every checkout, invisible to collaborators
 *   project  `<cwd>/.agents/plugins/` committed with the repo, shared, shadows the user copy
 */
export type PluginInstallScope = "user" | "project";

/** Resolve the install home for `scope`. */
export function installHomeForScope(scope: PluginInstallScope, cwd: string, agentDir: string = getAgentDir()): string {
	return scope === "project" ? projectPluginsDir(cwd) : consumptionPluginsDir(agentDir);
}

/**
 * Root of a platform's production home — the parent that {@link productionPluginDir}
 * places plugin directories under.
 */
export function productionRoot(platform: PluginPlatform, agentDir: string = getAgentDir()): string {
	// Claude: the documented skills-directory drop-in. A folder here carrying a
	// `.claude-plugin/plugin.json` loads as `<id>@skills-dir` with no install step.
	if (platform === "claude") return path.join(homeRoot(agentDir), ".claude", "skills");
	// GitHub: no vendor drop-in exists, so this is hoocode's own home for a
	// github-targeted artifact. PackagePlugin works in it and publish reads from it.
	return path.join(homeRoot(agentDir), ".agents", "publish", "github");
}

/** Where an authored plugin for `platform` lives. */
export function productionPluginDir(platform: PluginPlatform, id: string, agentDir: string = getAgentDir()): string {
	return path.join(productionRoot(platform, agentDir), sanitizeForDir(id));
}

/** `~/.agents/marketplaces.json` — the added-marketplace registry. */
export function marketplaceStorePath(agentDir: string = getAgentDir()): string {
	return path.join(homeRoot(agentDir), ".agents", "marketplaces.json");
}

/** Root of the local marketplace clone cache. A cache is never repo content. */
export function marketplaceCacheRoot(agentDir: string = getAgentDir()): string {
	return path.join(homeRoot(agentDir), ".agents", "marketplace-cache");
}

/** Local cache directory for a marketplace fetched from `url`. */
export function marketplaceCacheDir(url: string, agentDir: string = getAgentDir()): string {
	return path.join(marketplaceCacheRoot(agentDir), sanitizeForDir(url));
}

/**
 * Records when each cached marketplace index was last fetched.
 *
 * Kept beside the caches rather than inferred from directory mtimes: a clone's
 * mtime moves for reasons that have nothing to do with freshness, and "when did
 * we last talk to the remote" is the only question the TTL is asking.
 */
export function marketplaceCacheMetaPath(agentDir: string = getAgentDir()): string {
	return path.join(marketplaceCacheRoot(agentDir), ".fetched.json");
}

/**
 * The parent directories hoocode owns and may therefore remove a plugin from:
 * the two production homes, both consumption homes (user and project), and the
 * `.hoocode/plugins` home older versions installed into.
 *
 * Deliberately excludes `<cwd>/.claude/skills` and `<cwd>/.agents/skills`. Those
 * are discovered (loader.ts `defaultPluginDirs`) but are repository content a
 * team committed, so uninstall must not delete out of them — hoocode installed
 * nothing there.
 */
export function pluginHomeRoots(cwd: string, agentDir: string = getAgentDir()): string[] {
	return [
		productionRoot("claude", agentDir),
		productionRoot("github", agentDir),
		consumptionPluginsDir(agentDir),
		projectPluginsDir(cwd),
		path.join(cwd, ".hoocode", "plugins"),
	];
}

/**
 * Every directory a plugin with `id` could occupy, in the order a lookup should
 * try them: the platform production homes, then the consumption home, then the
 * legacy project homes. Used to find an existing plugin without knowing which
 * role wrote it.
 */
export function candidatePluginDirs(cwd: string, id: string, agentDir: string = getAgentDir()): string[] {
	const slug = sanitizeForDir(id);
	return pluginHomeRoots(cwd, agentDir).map((root) => path.join(root, slug));
}

/**
 * Persistent, writable directory unique to an installed plugin — the target of
 * `${CLAUDE_PLUGIN_DATA}` / `${COPILOT_PLUGIN_DATA}`, which both vendors document
 * as the place for plugin runtime state.
 *
 * Deliberately outside every plugin home. A plugin's own directory is replaced
 * wholesale on promote and deleted on uninstall, and the vendors are explicit
 * that this must not live "inside the installed-plugins cache directory" — state
 * that vanishes on reinstall is not state.
 */
export function pluginDataDir(id: string, agentDir: string = getAgentDir()): string {
	return path.join(homeRoot(agentDir), ".agents", "plugin-data", sanitizeForDir(id));
}

/** {@link pluginDataDir}, created if absent. */
export function ensurePluginDataDir(id: string, agentDir: string = getAgentDir()): string {
	const dir = pluginDataDir(id, agentDir);
	mkdirSync(dir, { recursive: true });
	return dir;
}

/** Create an ephemeral draft directory. The caller must promote or {@link discardDraftDir} it. */
export function makeDraftDir(): string {
	return mkdtempSync(path.join(tmpdir(), "hoo-plugin-draft-"));
}

/** Delete a draft directory. Safe to call twice. */
export function discardDraftDir(dir: string): void {
	rmSync(dir, { recursive: true, force: true });
}
