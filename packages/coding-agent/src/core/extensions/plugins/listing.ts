/**
 * Row shapes for the plugin listings, shared by every surface that prints one.
 *
 * `/plugin list`, `SearchPlugins` and `ListPlugins` describe the same objects,
 * and each used to build its own `name [a, b] — description` string, so the same
 * plugin printed three ways and the model's view never quite matched the user's.
 * They all go through here now; only the styling and the width differ.
 */

import type { ListGroup, ListRow } from "../../format-list.js";
import type { AvailablePlugin } from "./install.js";
import type { NormalizedPlugin } from "./manifest.js";

/** Human-readable source for an available plugin (a URL, a path, an npm spec). */
export function formatPluginSource(source: AvailablePlugin["source"]): string {
	if (typeof source === "string") return source;
	if (source.source === "url") return source.url;
	return `${source.url}/${source.path}`;
}

/**
 * Capability classes a plugin actually ships.
 *
 * Covers every capability `NormalizedPlugin` can carry — themes and providers
 * included, which the old `ListPlugins` inline list omitted, so a plugin that
 * shipped only a theme or only a provider reported no capabilities at all.
 */
export function pluginCapabilities(plugin: NormalizedPlugin): string[] {
	return [
		plugin.skillsDir && "skills",
		plugin.commandsDir && "commands",
		plugin.agentsDir && "agents",
		plugin.themesDir && "themes",
		plugin.hooks && "hooks",
		plugin.mcpServers && "mcp",
		plugin.providers?.length && "providers",
	].filter((value): value is string => typeof value === "string");
}

/**
 * Available plugins, grouped by the marketplace offering them.
 *
 * Grouping is what makes provenance legible: a flat list cannot answer "where
 * would this come from if I installed it", which is the one question that
 * matters at a trust boundary. Marketplace order is preserved from the caller.
 */
export function availablePluginGroups(
	plugins: readonly AvailablePlugin[],
	options: { installed?: ReadonlySet<string> } = {},
): ListGroup[] {
	const groups = new Map<string, ListRow[]>();
	for (const plugin of plugins) {
		const rows = groups.get(plugin.marketplaceName) ?? [];
		rows.push({
			name: plugin.name,
			marker: options.installed?.has(plugin.name) ? "✓" : undefined,
			facts: [plugin.supportPlatform.join("/"), plugin.sourceKind],
			detail: plugin.description ?? formatPluginSource(plugin.source),
		});
		groups.set(plugin.marketplaceName, rows);
	}
	return Array.from(groups, ([title, rows]) => ({ title, rows }));
}

/** Installed plugins as rows: id@version, platforms, format, capabilities. */
export function installedPluginRows(plugins: readonly NormalizedPlugin[]): ListRow[] {
	return plugins.map((plugin) => {
		const capabilities = pluginCapabilities(plugin);
		return {
			name: `${plugin.id}${plugin.version ? `@${plugin.version}` : ""}`,
			// The manifest format is parenthesised because it is usually the same
			// word as the platform ("claude/github · claude" reads as a stutter).
			facts: [plugin.supportPlatform.join("/"), `(${plugin.format})`, capabilities.join(", ") || "no capabilities"],
			detail: plugin.description,
		};
	});
}
