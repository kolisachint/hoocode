/**
 * Live plugin activation — the same-turn half of model-driven plugin install.
 *
 * The human `/plugin install` path activates via a full ctx.reload(). The
 * model-facing InstallPlugin tool has no reload capability, so without this
 * module an installed plugin's capabilities would be unusable until a human
 * reloads — breaking the notice-gap → search → install → use loop.
 *
 * This module installs a {@link LivePluginActivator} (see plugin-activation.ts)
 * at session_start that activates what can be activated safely mid-session:
 *
 *   - MCP servers → connected now; tools land in the deferred catalog or
 *     register eagerly, matching the session's deferMcpSchemas mode
 *   - skills / commands / agents / themes / hooks / providers → reported as
 *     reload-needed; they flow through the resource loader (and, for hooks,
 *     the human-confirmation trust boundary), which only a reload rebuilds
 *
 * A later /reload converges: it rediscovers the installed plugin from disk and
 * rebuilds everything through the normal loader path.
 */

import { registerExtensionMcpServers } from "../../core/extension-mcp-servers.js";
import { parsePluginDir, resolveMcpServers } from "../../core/extensions/plugins/index.js";
import type { ExtensionAPI, ExtensionContext, SessionStartEvent } from "../../core/extensions/types.js";
import { type LivePluginActivationResult, setLivePluginActivator } from "../../core/plugin-activation.js";
import { currentSubagentDepth } from "../../core/subagent-depth.js";
import { activateMcpServersLive } from "./mcp-loader.js";

/** Build the activator for one session. Exported for tests. */
export function buildLivePluginActivator(
	pi: ExtensionAPI,
	notify: (message: string, level: "info" | "warning" | "error") => void,
): (pluginRoot: string) => Promise<LivePluginActivationResult> {
	return async (pluginRoot: string) => {
		const plugin = parsePluginDir(pluginRoot);
		if (!plugin) return { error: `no recognizable plugin manifest in ${pluginRoot}` };

		const needsReload: string[] = [];
		if (plugin.skillsDir) needsReload.push("skills");
		if (plugin.commandsDir) needsReload.push("commands");
		if (plugin.agentsDir) needsReload.push("agents");
		if (plugin.themesDir) needsReload.push("themes");
		if (plugin.hooks) needsReload.push("hooks");
		if (plugin.providers?.length) needsReload.push("providers");

		let mcpTools: string[] = [];
		let mcpDeferred = false;
		let mcpErrors: string[] = [];
		let mcpSkipped: string[] = [];
		if (plugin.mcpServers && Object.keys(plugin.mcpServers).length > 0) {
			const servers = resolveMcpServers(plugin.mcpServers, plugin.root);
			// Also record them in the extension MCP registry so in-session state
			// matches what a plugin loaded at startup would have produced.
			registerExtensionMcpServers(plugin.id, servers);
			const activation = await activateMcpServersLive(pi, servers, notify);
			mcpTools = activation.registeredTools;
			mcpDeferred = activation.deferred;
			mcpErrors = activation.errors;
			mcpSkipped = activation.skipped;
		}

		return { pluginId: plugin.id, mcpTools, mcpDeferred, mcpErrors, mcpSkipped, needsReload };
	};
}

export function setupPluginActivator(pi: ExtensionAPI): void {
	pi.on("session_start", (_event: SessionStartEvent, ctx: ExtensionContext) => {
		// Subagent children never carry the plugin lifecycle tools (spec §3), so
		// they don't need — and must not have — a live activator.
		if (currentSubagentDepth() > 0) return;
		setLivePluginActivator(buildLivePluginActivator(pi, (message, level) => ctx.ui.notify(message, level)));
	});
}
