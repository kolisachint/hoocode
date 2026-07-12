/**
 * Canonical names of the plugin-system (capability-acquisition) tools, in one
 * dependency-free module so the tool definitions, the authoring engine, and the
 * privilege-amplification guardrail can all reference them without an import
 * cycle.
 *
 * {@link PLUGIN_SYSTEM_TOOL_NAMES} is the guardrail set: these tools live on the
 * top-level agent only and may never appear in an authored subagent's allowlist
 * (otherwise a low-trust authored agent could bootstrap privilege via an
 * author → spawn → install loop).
 */

// Lifecycle tools (spec §1).
export const SEARCH_PLUGINS_TOOL_NAME = "SearchPlugins";
export const LIST_PLUGINS_TOOL_NAME = "ListPlugins";
export const SUGGEST_PLUGIN_INSTALL_TOOL_NAME = "SuggestPluginInstall";
export const INSTALL_PLUGIN_TOOL_NAME = "InstallPlugin";
export const UNINSTALL_PLUGIN_TOOL_NAME = "UninstallPlugin";

// Authoring tools (spec §3).
export const PROPOSE_PLUGIN_TOOL_NAME = "ProposePlugin";
export const PROPOSE_EXECUTABLE_PLUGIN_TOOL_NAME = "ProposeExecutablePlugin";

/** Every capability-acquisition tool — the guardrail set stripped from authored allowlists. */
export const PLUGIN_SYSTEM_TOOL_NAMES: readonly string[] = [
	SEARCH_PLUGINS_TOOL_NAME,
	LIST_PLUGINS_TOOL_NAME,
	SUGGEST_PLUGIN_INSTALL_TOOL_NAME,
	INSTALL_PLUGIN_TOOL_NAME,
	UNINSTALL_PLUGIN_TOOL_NAME,
	PROPOSE_PLUGIN_TOOL_NAME,
	PROPOSE_EXECUTABLE_PLUGIN_TOOL_NAME,
];
