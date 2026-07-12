/**
 * Process-global bridge between the model-facing InstallPlugin tool and the
 * live plugin activation machinery.
 *
 * InstallPlugin is created in main.ts as a plain custom tool with no
 * ExtensionAPI handle, while connecting MCP servers and registering tools
 * requires one. The hoo-core plugin-activator module closes over its `pi` at
 * session_start and installs the activator here; the tool looks it up at
 * execute time. Mirrors the module-global approach of extension-mcp-servers.ts.
 *
 * The activator is re-installed on every session_start (startup and reload),
 * so a captured stale `pi` is always replaced before the next tool call.
 */

/** What a live activation pass did for one installed plugin. */
export interface LivePluginActivationSummary {
	pluginId: string;
	/** Full `mcp_<server>_<tool>` names now callable (eager) or resolvable via ResolveMcpTools (deferred). */
	mcpTools: string[];
	/** Whether the MCP tools went into the deferred catalog (call ResolveMcpTools first) or are directly callable. */
	mcpDeferred: boolean;
	/** Per-server MCP connect failures. */
	mcpErrors: string[];
	/** MCP servers skipped because a same-named server is already active. */
	mcpSkipped: string[];
	/** Capability kinds the plugin bundles that only activate on the next reload (skills, commands, hooks, ...). */
	needsReload: string[];
}

export type LivePluginActivationResult = LivePluginActivationSummary | { error: string };

export type LivePluginActivator = (pluginRoot: string) => Promise<LivePluginActivationResult>;

let activator: LivePluginActivator | undefined;

/** Install (or replace) the live activator. Called by hoo-core on each session_start. */
export function setLivePluginActivator(fn: LivePluginActivator | undefined): void {
	activator = fn;
}

/** The current live activator, or undefined when live activation is unavailable (e.g. subagent child). */
export function getLivePluginActivator(): LivePluginActivator | undefined {
	return activator;
}

/** Render an activation result as the tool-result text appended after the install message. */
export function formatLiveActivationSummary(result: LivePluginActivationResult): string {
	if ("error" in result) {
		return `Live activation failed: ${result.error}. The plugin activates on the next reload.`;
	}
	const lines: string[] = [];
	if (result.mcpTools.length > 0) {
		lines.push(
			result.mcpDeferred
				? `Connected its MCP server(s); ${result.mcpTools.length} tool(s) added to the deferred catalog — call ResolveMcpTools to use them: ${result.mcpTools.join(", ")}.`
				: `Connected its MCP server(s); ${result.mcpTools.length} tool(s) now callable: ${result.mcpTools.join(", ")}.`,
		);
	}
	for (const skipped of result.mcpSkipped) {
		lines.push(`MCP server "${skipped}" was not started: a server with that name is already active (kept existing).`);
	}
	for (const error of result.mcpErrors) {
		lines.push(`MCP connect failed — ${error}.`);
	}
	if (result.needsReload.length > 0) {
		lines.push(`Also bundles ${result.needsReload.join(", ")} — these activate on the next reload.`);
	}
	if (lines.length === 0) {
		lines.push("No live-activatable capabilities found; the plugin's content activates on the next reload.");
	}
	return lines.join("\n");
}
