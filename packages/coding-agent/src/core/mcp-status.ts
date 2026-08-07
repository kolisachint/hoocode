/**
 * Process-global registry of MCP servers that actually connected this session.
 *
 * MCP discovery and connection happen inside the hoo-core `mcp-loader` module on
 * `session_start`, from several sources (standard `mcp.json` files, hoocode's
 * per-server JSON files, plugin registrations). The startup resource summary
 * needs a single answer to "which servers are live and how many tools do they
 * bring", so the loader records each outcome here and the UI reads it.
 *
 * Mirrors the module-global approach of `extension-mcp-servers.ts`. The registry
 * is cleared at the start of each connect pass so a session switch or reload
 * rebuilds it instead of accumulating.
 */

export interface McpServerStatus {
	/** Server name, as used in the `mcp_<server>_<tool>` prefix. */
	name: string;
	/** Tools the server exposed on connect. */
	toolCount: number;
	/** Tools default to background execution (task-panel rows) rather than inline. */
	background: boolean;
	/** Tool names are injected but schemas are materialized on demand. */
	deferred: boolean;
	/** `authorizing` means a browser OAuth flow is in flight; tools register when it completes. */
	state: "connected" | "authorizing";
}

const statuses = new Map<string, McpServerStatus>();

/** Record (or replace) the connection outcome for one server. */
export function setMcpServerStatus(status: McpServerStatus): void {
	statuses.set(status.name, status);
}

/** All servers that connected (or are awaiting authorization) this session. */
export function getMcpServerStatuses(): McpServerStatus[] {
	return [...statuses.values()];
}

/** Clear the registry. Called before each connect pass so reloads rebuild it. */
export function clearMcpServerStatuses(): void {
	statuses.clear();
}
