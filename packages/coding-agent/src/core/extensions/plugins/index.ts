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
 *  - skills / themes         → `resources_discover` skill/theme paths
 *  - commands                → `resources_discover` slash-command paths (`.agents/commands`)
 *  - agents                  → `resources_discover` agent paths (`.agents/agents` subagents)
 *  - providers (native only) → `registerProvider`
 *  - hooks                   → shell-protocol bridge (see hooks-bridge.ts)
 *  - mcpServers              → parsed; wiring deferred (see design doc)
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { type ExtensionMcpServerConfig, registerExtensionMcpServers } from "../../extension-mcp-servers.js";
import type { ExtensionAPI, ExtensionFactory } from "../types.js";
import { installPluginHooks } from "./hooks-bridge.js";
import { type NormalizedPlugin, parsePluginDir } from "./manifest.js";

/** Substitute ${CLAUDE_PLUGIN_ROOT} / ${AGENTS_PLUGIN_ROOT} with the plugin root in a string. */
function substituteRoot(value: string, root: string): string {
	return value.replace(/\$\{(?:CLAUDE|AGENTS)_PLUGIN_ROOT\}/g, root);
}

/** Coerce parsed mcpServers into the standard config shape, substituting root vars. */
function resolveMcpServers(
	mcpServers: Record<string, unknown>,
	root: string,
): Record<string, ExtensionMcpServerConfig> {
	const out: Record<string, ExtensionMcpServerConfig> = {};
	for (const [name, raw] of Object.entries(mcpServers)) {
		if (!raw || typeof raw !== "object") continue;
		const cfg = raw as ExtensionMcpServerConfig;
		// Remote servers ({ type: "http" | "sse", url }): pass through for the
		// MCP loader's HTTP transports.
		if (typeof cfg.url === "string" && typeof cfg.command !== "string") {
			out[name] = {
				type: cfg.type === "sse" ? "sse" : "http",
				url: cfg.url,
				headers: cfg.headers
					? Object.fromEntries(Object.entries(cfg.headers).map(([k, v]) => [k, substituteRoot(String(v), root)]))
					: undefined,
				background: cfg.background,
			};
			continue;
		}
		if (typeof cfg.command !== "string") continue;
		out[name] = {
			command: substituteRoot(cfg.command, root),
			args: cfg.args?.map((a) => substituteRoot(String(a), root)),
			env: cfg.env
				? Object.fromEntries(Object.entries(cfg.env).map(([k, v]) => [k, substituteRoot(String(v), root)]))
				: undefined,
			background: cfg.background,
		};
	}
	return out;
}

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
		// Resources: contribute the plugin's capability directories. Commands map to
		// the slash-command surface (`.agents/commands`) and agents to subagent
		// definitions (`.agents/agents`), matching hoocode's native conventions.
		if (plugin.skillsDir || plugin.commandsDir || plugin.themesDir || plugin.agentsDir) {
			pi.on("resources_discover", () => ({
				skillPaths: plugin.skillsDir ? [plugin.skillsDir] : undefined,
				themePaths: plugin.themesDir ? [plugin.themesDir] : undefined,
				slashCommandPaths: plugin.commandsDir ? [plugin.commandsDir] : undefined,
				agentPaths: plugin.agentsDir ? [plugin.agentsDir] : undefined,
			}));
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

		// MCP servers: register for the hoo-core mcp-loader to connect on session_start.
		if (plugin.mcpServers) {
			registerExtensionMcpServers(plugin.id, resolveMcpServers(plugin.mcpServers, plugin.root));
		}
	};

	factory.displayName = `plugin:${plugin.id}`;
	return factory;
}
