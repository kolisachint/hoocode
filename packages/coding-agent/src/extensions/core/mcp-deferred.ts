/**
 * Deferred MCP tool schemas (spec §2) — the light, testable half.
 *
 * MCP tool schemas are the only genuinely heavy thing an installed plugin's
 * server adds to context: every tool's full JSON schema lands up front. When
 * deferral is enabled (opt-in, top-level agent only), the loader injects tool
 * *names* only and materializes each schema on demand — the same deferred-tool /
 * ToolSearch shape the harness uses. This module holds the format-agnostic
 * catalog helpers; the connect/register machinery lives in mcp-loader.ts.
 *
 * Only half the cost used to be deferred. Schemas were withheld, but the full
 * catalog — every name and description — was rendered into the resolver's
 * description on every request, and against a ~2,710-token tool-schema budget an
 * unbounded catalog dominates. That was forced by the matcher: exact-string-only
 * selection means a name you cannot see is a tool you cannot reach, so the dump
 * could not be trimmed. Retrieval (`core/capabilities`) breaks the deadlock, and
 * {@link formatDeferredCatalog} spends the budget accordingly — full list while
 * it is cheap, servers and counts once it is not.
 *
 * See docs/plugin-system-architecture.md §6.1–§6.3.
 */

import type { CapabilityDoc } from "../../core/capabilities/registry.js";

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
 * Catalog size at which the full listing stops being the cheaper option.
 *
 * Below it, a flat catalog is both cheaper than a retrieval round-trip and
 * strictly more deterministic — the model can see every tool it has, with no
 * query it had to think to write (§6.4). Above it, the listing is the dominant
 * cost and the search path is worth the indirection. Thirty is the low end of
 * the §6.3 range: at ~15 tokens an entry that is roughly 450 tokens of budget,
 * and the entries past it are increasingly ones this session will never touch.
 */
export const CATALOG_EAGER_LIMIT = 30;

/** One catalog line: name and a clipped first line of description. */
function entryLine(e: DeferredMcpToolEntry): string {
	const desc = e.description.split("\n")[0]?.slice(0, 120) ?? "";
	return `  ${e.toolName}${desc ? ` — ${desc}` : ""}`;
}

function groupByServer(entries: DeferredMcpToolEntry[]): Map<string, DeferredMcpToolEntry[]> {
	const byServer = new Map<string, DeferredMcpToolEntry[]>();
	for (const e of entries) {
		const list = byServer.get(e.server) ?? [];
		byServer.set(e.server, list);
		list.push(e);
	}
	return byServer;
}

/**
 * Render the catalog for the resolver's description.
 *
 * Two shapes, chosen by size. The summary is not merely a truncated list: it
 * names every server and its tool count, so the model still knows the whole
 * surface exists and what it is about — it just has to ask for the part it
 * wants. A list cut off at thirty entries would instead hide the tail
 * completely, which is the failure mode worth avoiding.
 */
export function formatDeferredCatalog(entries: DeferredMcpToolEntry[], limit = CATALOG_EAGER_LIMIT): string {
	if (entries.length === 0) return "(no MCP tools available)";
	const byServer = groupByServer(entries);

	if (entries.length <= limit) {
		const lines: string[] = [];
		for (const [server, list] of byServer) {
			lines.push(`${server}:`);
			for (const e of list) lines.push(entryLine(e));
		}
		return lines.join("\n");
	}

	const lines = [`${entries.length} tools across ${byServer.size} server(s):`];
	for (const [server, list] of byServer) {
		const sample = list
			.slice(0, 4)
			.map((e) => e.toolName.replace(/^mcp_[^_]+_/, ""))
			.join(", ");
		lines.push(`  ${server} — ${list.length} tool(s): ${sample}${list.length > 4 ? ", …" : ""}`);
	}
	lines.push('Search for the rest by capability (e.g. `query: "create a pull request"`).');
	return lines.join("\n");
}

/**
 * Select the catalog entries a resolve request refers to. Matches on the full
 * registered name (`mcp_server_tool`), and also on a bare tool name when it is
 * unambiguous, so the model can ask by either.
 *
 * Stays exact on purpose. Retrieval is a separate parameter, and collapsing the
 * two would make "resolve exactly these tools" a fuzzy operation — which is the
 * one thing a caller naming a tool it already knows about does not want.
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

/** The catalog as capability documents, for the index. */
export function toCapabilityDocs(entries: DeferredMcpToolEntry[], deferred: boolean): CapabilityDoc[] {
	return entries.map((e) => ({
		id: e.toolName,
		kind: "mcp-tool" as const,
		name: e.toolName,
		description: e.description,
		source: e.server,
		deferred,
	}));
}
