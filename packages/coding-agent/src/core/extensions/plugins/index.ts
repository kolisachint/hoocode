/**
 * Plugin discovery and wiring.
 *
 * Discovers plugin directories under the `plugins/` folders, parses their
 * manifests (see {@link parsePluginDir}), and turns each into a synthetic
 * {@link ExtensionFactory} that registers the plugin's capabilities through the
 * existing ExtensionAPI. The factory is loaded by the standard extension loader,
 * so plugins are just extensions assembled from a manifest instead of code.
 *
 * Capability wiring (minimum):
 *  - skills / commands / themes → `resources_discover` resource paths
 *  - agents                     → `addModeSearchPath` (best-effort; see design doc)
 *  - providers (native only)    → `registerProvider`
 *  - hooks                      → shell-protocol bridge (see hooks-bridge.ts)
 *  - mcpServers                 → parsed; wiring deferred (see design doc)
 */

import * as fs from "node:fs";
import * as path from "node:path";
import type { ExtensionAPI, ExtensionFactory } from "../types.js";
import { installPluginHooks } from "./hooks-bridge.js";
import { type NormalizedPlugin, parsePluginDir } from "./manifest.js";

export type { NormalizedPlugin } from "./manifest.js";
export { parsePluginDir } from "./manifest.js";

/**
 * Discover plugins across the given `plugins/` directories.
 * First-wins on duplicate ids (project dirs should be listed before global).
 */
export function discoverPlugins(pluginDirs: string[]): NormalizedPlugin[] {
	const plugins: NormalizedPlugin[] = [];
	const seen = new Set<string>();

	for (const dir of pluginDirs) {
		if (!fs.existsSync(dir)) continue;
		let entries: fs.Dirent[];
		try {
			entries = fs.readdirSync(dir, { withFileTypes: true });
		} catch {
			continue;
		}
		for (const entry of entries) {
			if (!entry.isDirectory() && !entry.isSymbolicLink()) continue;
			const plugin = parsePluginDir(path.join(dir, entry.name));
			if (plugin && !seen.has(plugin.id)) {
				seen.add(plugin.id);
				plugins.push(plugin);
			}
		}
	}

	return plugins;
}

/** Build a synthetic extension factory that wires one normalized plugin. */
export function buildPluginFactory(plugin: NormalizedPlugin): ExtensionFactory {
	const factory: ExtensionFactory = (pi: ExtensionAPI) => {
		// Resources: contribute the plugin's skill/command/theme directories.
		if (plugin.skillsDir || plugin.commandsDir || plugin.themesDir) {
			pi.on("resources_discover", () => ({
				skillPaths: plugin.skillsDir ? [plugin.skillsDir] : undefined,
				promptPaths: plugin.commandsDir ? [plugin.commandsDir] : undefined,
				themePaths: plugin.themesDir ? [plugin.themesDir] : undefined,
			}));
		}

		// Agents → mode search path (best-effort; structure transform is a follow-up).
		if (plugin.agentsDir) {
			pi.addModeSearchPath(plugin.agentsDir);
		}

		// Providers (native plugins only).
		for (const provider of plugin.providers ?? []) {
			pi.registerProvider(provider.name, provider.config);
		}

		// Hooks: true-parity shell bridge.
		if (plugin.hooks) {
			installPluginHooks(pi, plugin.hooks, plugin.root, () => {
				// Non-blocking hook failures are intentionally quiet (Claude Code parity).
			});
		}
	};

	factory.displayName = `plugin:${plugin.id}`;
	return factory;
}
