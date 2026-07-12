/**
 * Process-global registry of MCP servers contributed by extensions/plugins.
 *
 * Plugins declare `mcpServers` in their manifest (or a `.mcp.json`), but MCP
 * connection happens in the hoo-core `mcp-loader` module's `setupMcpLoader` on `session_start`, which
 * reads from fixed file locations. This registry bridges the two: plugin
 * factories register their servers here during load, and `setupMcpLoader` reads
 * them when connecting. Mirrors the module-global approach of
 * `agent-manifest-paths.ts`.
 *
 * Entries are keyed by `source` so a reload can rebuild the set cleanly: call
 * {@link clearExtensionMcpServers} before (re)loading plugins, then register.
 */

/** Standard MCP server config (Claude Desktop / `.mcp.json` shape). */
export interface ExtensionMcpServerConfig {
	command: string;
	args?: string[];
	env?: Record<string, string>;
	background?: boolean;
	/** One-line steering text injected into the system prompt (see McpServerConfig.promptSnippet). */
	promptSnippet?: string;
	/** Guideline bullets injected into the system prompt (see McpServerConfig.promptGuidelines). */
	promptGuidelines?: string[];
}

export interface ExtensionMcpEntry {
	/** Identifier of the contributing plugin/extension (for diagnostics + dedup). */
	source: string;
	/** Server name → config, in standard `mcpServers` format. */
	mcpServers: Record<string, ExtensionMcpServerConfig>;
}

let registered: ExtensionMcpEntry[] = [];

/** Register MCP servers contributed by a plugin/extension. */
export function registerExtensionMcpServers(
	source: string,
	mcpServers: Record<string, ExtensionMcpServerConfig>,
): void {
	if (!mcpServers || Object.keys(mcpServers).length === 0) return;
	registered.push({ source, mcpServers });
}

/** Get all currently registered extension MCP servers. */
export function getExtensionMcpServers(): ExtensionMcpEntry[] {
	return [...registered];
}

/** Clear the registry. Call before (re)loading plugins so reloads don't accumulate. */
export function clearExtensionMcpServers(): void {
	registered = [];
}
