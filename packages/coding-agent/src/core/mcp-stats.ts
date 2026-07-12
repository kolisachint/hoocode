/**
 * Per-server MCP reliability stats — a local, observed success-rate signal.
 *
 * Every MCP tool call and connect attempt updates an in-memory aggregate that
 * is persisted (debounced, best-effort) to `<agentDir>/mcp-stats.json`. The
 * signal is surfaced where the model or human chooses between servers: the
 * connect notification, the ResolveMcpTools deferred catalog, and ListPlugins.
 *
 * Attribution is deliberately conservative: only *transport* failures — the
 * server process exiting, handshake timeouts, failed reconnects — count
 * against a server. A tool result the model dislikes is not the server's
 * fault, and user-initiated aborts are not counted at all.
 */

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { getHooCodeDir } from "../config.js";

export interface McpServerStats {
	/** Completed tool calls (ok + transport-failed; aborts excluded). */
	calls: number;
	/** Tool calls that failed in transport (server exit, timeout, reconnect failure). */
	callTransportFailures: number;
	/** Failed connect/handshake attempts (session start, live activation, lazy reconnect). */
	connectFailures: number;
	/** Epoch ms of the most recent transport or connect failure. */
	lastFailureAt?: number;
}

interface McpStatsFile {
	servers: Record<string, McpServerStats>;
}

export type McpCallOutcome = "ok" | "transport_failure";

const PERSIST_DEBOUNCE_MS = 500;

export interface McpStatsStore {
	recordCall(server: string, outcome: McpCallOutcome): void;
	recordConnectFailure(server: string): void;
	get(server: string): McpServerStats | undefined;
	/** Flush pending writes now (also runs on process exit). */
	flush(): void;
}

/** Create a stats store backed by `filePath`. Loads lazily; all IO is best-effort. */
export function createMcpStatsStore(filePath: string, enabled: () => boolean = () => true): McpStatsStore {
	let data: McpStatsFile | undefined;
	let timer: NodeJS.Timeout | undefined;
	let exitHookInstalled = false;

	function load(): McpStatsFile {
		if (data) return data;
		data = { servers: {} };
		try {
			if (existsSync(filePath)) {
				const raw = JSON.parse(readFileSync(filePath, "utf8")) as McpStatsFile;
				if (raw && typeof raw === "object" && raw.servers && typeof raw.servers === "object") {
					data = { servers: raw.servers };
				}
			}
		} catch {
			// corrupt or unreadable stats are not worth failing anything over
		}
		return data;
	}

	function serverEntry(server: string): McpServerStats {
		const d = load();
		d.servers[server] ??= { calls: 0, callTransportFailures: 0, connectFailures: 0 };
		return d.servers[server];
	}

	function flush(): void {
		if (timer) {
			clearTimeout(timer);
			timer = undefined;
		}
		if (!data) return;
		try {
			writeFileSync(filePath, `${JSON.stringify(data, null, "\t")}\n`);
		} catch {
			// best-effort persistence
		}
	}

	function schedulePersist(): void {
		if (!exitHookInstalled) {
			exitHookInstalled = true;
			process.once("exit", flush);
		}
		if (timer) return;
		timer = setTimeout(() => {
			timer = undefined;
			flush();
		}, PERSIST_DEBOUNCE_MS);
		timer.unref?.();
	}

	return {
		recordCall(server: string, outcome: McpCallOutcome): void {
			if (!enabled()) return;
			const entry = serverEntry(server);
			entry.calls++;
			if (outcome === "transport_failure") {
				entry.callTransportFailures++;
				entry.lastFailureAt = Date.now();
			}
			schedulePersist();
		},
		recordConnectFailure(server: string): void {
			if (!enabled()) return;
			const entry = serverEntry(server);
			entry.connectFailures++;
			entry.lastFailureAt = Date.now();
			schedulePersist();
		},
		get(server: string): McpServerStats | undefined {
			if (!enabled()) return undefined;
			return load().servers[server];
		},
		flush,
	};
}

let statsEnabled = true;

/** Toggle stats collection and surfacing (seeded from the enableMcpStats setting). */
export function setMcpStatsEnabled(enabled: boolean): void {
	statsEnabled = enabled;
}

/** The default store, persisted to `<agentDir>/mcp-stats.json`. */
export const mcpStats: McpStatsStore = createMcpStatsStore(join(getHooCodeDir(), "mcp-stats.json"), () => statsEnabled);

/** Minimum observations before reliability is shown at all. */
const MIN_CALLS_FOR_SIGNAL = 5;
/** Success rate below which a server is flagged in the deferred catalog. */
const UNRELIABLE_THRESHOLD = 0.9;

function successRate(stats: McpServerStats): number {
	if (stats.calls === 0) return 1;
	return (stats.calls - stats.callTransportFailures) / stats.calls;
}

/**
 * Human-readable reliability summary, or undefined when there is not enough
 * history to be meaningful (fewer than {@link MIN_CALLS_FOR_SIGNAL} calls and
 * no connect failures).
 */
export function formatMcpReliability(stats: McpServerStats | undefined): string | undefined {
	if (!stats) return undefined;
	const parts: string[] = [];
	if (stats.calls >= MIN_CALLS_FOR_SIGNAL) {
		parts.push(`${Math.round(successRate(stats) * 100)}% success over ${stats.calls} calls`);
	}
	if (stats.connectFailures > 0) {
		parts.push(`${stats.connectFailures} connect failure${stats.connectFailures === 1 ? "" : "s"}`);
	}
	return parts.length > 0 ? parts.join(", ") : undefined;
}

/**
 * Short warning tag for the deferred catalog, or undefined for servers with no
 * meaningful negative signal. Only transport-level evidence triggers this.
 */
export function formatMcpReliabilityWarning(stats: McpServerStats | undefined): string | undefined {
	if (!stats) return undefined;
	if (stats.calls >= MIN_CALLS_FOR_SIGNAL && successRate(stats) < UNRELIABLE_THRESHOLD) {
		return `[unreliable: ${Math.round(successRate(stats) * 100)}% success over ${stats.calls} calls]`;
	}
	return undefined;
}
