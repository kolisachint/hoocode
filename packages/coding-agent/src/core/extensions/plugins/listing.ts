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
import { pluginScopeOf } from "./locations.js";
import type { NormalizedPlugin } from "./manifest.js";

/** Human-readable source for an available plugin (a URL, a path, an npm spec). */
export function formatPluginSource(source: AvailablePlugin["source"]): string {
	if (typeof source === "string") return source;
	if (source.source === "npm") return source.package;
	// `archive` carries a url like `url` does; without this it printed
	// `https://…/p.zip/undefined` through the git-subdir branch below.
	if (source.source === "url" || source.source === "archive") return source.url;
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
		plugin.canvasExtensions?.length && "canvases",
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
		// npm and archive entries are catalogued but not fetchable here, and saying
		// so in the row is cheaper than an InstallPlugin round trip that only ends
		// in the same message.
		const installable = plugin.sourceKind !== "npm" && plugin.sourceKind !== "archive";
		rows.push({
			name: plugin.name,
			marker: options.installed?.has(plugin.name) ? "✓" : undefined,
			facts: [
				plugin.supportPlatform.join("/"),
				installable ? plugin.sourceKind : `${plugin.sourceKind}, not installable`,
			],
			detail: plugin.description ?? formatPluginSource(plugin.source),
		});
		groups.set(plugin.marketplaceName, rows);
	}
	return Array.from(groups, ([title, rows]) => ({ title, rows }));
}

export interface InstalledRowOptions {
	/** Working directory, used to place each plugin in a scope. */
	cwd?: string;
	/** Whether `cwd` is a trusted workspace; decides if executables actually load. */
	trusted?: boolean;
}

/** Installed plugins as rows: id@version, platforms, format, scope, capabilities. */
export function installedPluginRows(
	plugins: readonly NormalizedPlugin[],
	options: InstalledRowOptions = {},
): ListRow[] {
	return plugins.map((plugin) => {
		const capabilities = pluginCapabilities(plugin);
		// Scope answers "why is this loaded, and who else gets it" — the plugin's
		// own path always knew, and nothing surfaced it.
		const scope = options.cwd ? pluginScopeOf(plugin.root, options.cwd) : undefined;
		// A working-tree plugin's executables load only in a trusted workspace, so
		// listing "hooks" unqualified would overstate what is actually running.
		const withheld = scope && scope !== "user" && !options.trusted && (plugin.hooks || plugin.mcpServers);
		return {
			name: `${plugin.id}${plugin.version ? `@${plugin.version}` : ""}`,
			// The manifest format is parenthesised because it is usually the same
			// word as the platform ("claude/github · claude" reads as a stutter).
			facts: [
				plugin.supportPlatform.join("/"),
				`(${plugin.format})`,
				...(scope ? [scope] : []),
				capabilities.join(", ") || "no capabilities",
				...(withheld ? ["hooks/mcp withheld, workspace not trusted"] : []),
			],
			detail: plugin.description,
		};
	});
}
