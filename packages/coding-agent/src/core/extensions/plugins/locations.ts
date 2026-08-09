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
 *   Consumption home   Persistent, global. Where marketplace installs land, in
 *                      whatever format the marketplace served. hoocode
 *                      installing for itself. {@link consumptionPluginsDir}
 *   Production home    Persistent, global, per platform. Where a plugin hoocode
 *                      *authored* lives. {@link productionPluginDir}
 *
 * All three are user-scoped. A plugin is portable, versioned and reusable across
 * projects, so writing one into the working tree means the capability is
 * invisible in every other repo and the agent dirties `git status` with content
 * unrelated to the task. The project-local home is still *read* so plugins
 * installed by older versions keep working ({@link legacyProjectPluginsDir}).
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

/** `~/.agents/plugins/` — where marketplace installs land. Format-agnostic. */
export function consumptionPluginsDir(agentDir: string = getAgentDir()): string {
	return path.join(homeRoot(agentDir), ".agents", "plugins");
}

/** The project-local home older versions installed into. Read-only now; still uninstalled from. */
export function legacyProjectPluginsDir(cwd: string): string {
	return path.join(cwd, ".agents", "plugins");
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
 * Every directory a plugin with `id` could occupy, in the order a lookup should
 * try them: the platform production homes, then the consumption home, then the
 * legacy project home. Used to find an existing plugin without knowing which
 * role wrote it.
 */
export function candidatePluginDirs(cwd: string, id: string, agentDir: string = getAgentDir()): string[] {
	const slug = sanitizeForDir(id);
	return [
		productionPluginDir("claude", id, agentDir),
		productionPluginDir("github", id, agentDir),
		path.join(consumptionPluginsDir(agentDir), slug),
		path.join(legacyProjectPluginsDir(cwd), slug),
	];
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
