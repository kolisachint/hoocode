/**
 * Deferred MCP tool schemas (spec §2) — the light, testable half.
 *
 * MCP tool schemas are the only genuinely heavy thing an installed plugin's
 * server adds to context: every tool's full JSON schema lands up front. When
 * deferral is enabled (default on, top-level agent only), the loader injects
 * tool *names* only and materializes each schema on demand — the same
 * deferred-tool / ToolSearch shape the harness uses. This module holds the
 * format-agnostic catalog helpers; the connect/register machinery lives in
 * mcp-loader.ts.
 */

/**
 * Kick off one connect per config concurrently and settle into per-config
 * outcomes in the ORIGINAL config order. Startup latency becomes the slowest
 * server's handshake rather than the sum, while registration order (and thus
 * the deferred catalog text and tool listing) stays deterministic across runs
 * for stable, cache-friendly system prompts.
 */
export async function connectAllInOrder<C, T>(
	configs: readonly C[],
	connect: (config: C) => Promise<T>,
): Promise<{ config: C; result: PromiseSettledResult<T> }[]> {
	const settled = await Promise.allSettled(configs.map((c) => connect(c)));
	return configs.map((config, i) => ({ config, result: settled[i] }));
}

/** A deferred tool as surfaced to the model: name + one-line description, no schema. */
export interface DeferredMcpToolEntry {
	/** Registered tool name (`mcp_<server>_<tool>`). */
	toolName: string;
	/** Server that owns the tool. */
	server: string;
	/** Short description (light — the JSON schema is what we defer). */
	description: string;
}

/**
 * Render the deferred catalog as the names-only surface embedded in the resolver
 * tool's description. `serverSnippets` optionally annotates a server's header
 * line with its configured steering text (see McpServerConfig.promptSnippet).
 */
export function formatDeferredCatalog(
	entries: DeferredMcpToolEntry[],
	serverSnippets?: ReadonlyMap<string, string>,
): string {
	if (entries.length === 0) return "(no MCP tools available)";
	const byServer = new Map<string, DeferredMcpToolEntry[]>();
	for (const e of entries) {
		const list = byServer.get(e.server) ?? [];
		byServer.set(e.server, list);
		list.push(e);
	}
	const lines: string[] = [];
	for (const [server, list] of byServer) {
		const snippet = serverSnippets?.get(server);
		lines.push(`${server}:${snippet ? ` ${snippet}` : ""}`);
		for (const e of list) {
			const desc = e.description.split("\n")[0]?.slice(0, 120) ?? "";
			lines.push(`  ${e.toolName}${desc ? ` — ${desc}` : ""}`);
		}
	}
	return lines.join("\n");
}

/**
 * Select the catalog entries a resolve request refers to. Matches on the full
 * registered name (`mcp_server_tool`), and also on a bare tool name when it is
 * unambiguous, so the model can ask by either.
 */
export function selectResolvable(entries: DeferredMcpToolEntry[], names: string[]): DeferredMcpToolEntry[] {
	const wanted = new Set(names.map((n) => n.trim()).filter(Boolean));
	if (wanted.size === 0) return [];
	const out: DeferredMcpToolEntry[] = [];
	const seen = new Set<string>();
	for (const entry of entries) {
		if (seen.has(entry.toolName)) continue;
		const bareTool = entry.toolName.replace(/^mcp_[^_]+_/, "");
		if (wanted.has(entry.toolName) || wanted.has(bareTool)) {
			out.push(entry);
			seen.add(entry.toolName);
		}
	}
	return out;
}
