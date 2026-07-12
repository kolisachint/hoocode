/**
 * Per-server MCP reliability stats: store round-trip, the enabled gate, and
 * the formatting thresholds (minimum sample size, unreliable warning tag).
 */

import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
	createMcpStatsStore,
	formatMcpReliability,
	formatMcpReliabilityWarning,
	type McpServerStats,
} from "../src/core/mcp-stats.js";

let dir: string;
let file: string;

beforeEach(() => {
	dir = mkdtempSync(join(tmpdir(), "hoo-mcp-stats-"));
	file = join(dir, "mcp-stats.json");
});

afterEach(() => {
	rmSync(dir, { recursive: true, force: true });
});

describe("createMcpStatsStore", () => {
	it("aggregates calls and failures per server", () => {
		const store = createMcpStatsStore(file);
		store.recordCall("alpha", "ok");
		store.recordCall("alpha", "ok");
		store.recordCall("alpha", "transport_failure");
		store.recordConnectFailure("alpha");
		store.recordCall("beta", "ok");

		expect(store.get("alpha")).toMatchObject({ calls: 3, callTransportFailures: 1, connectFailures: 1 });
		expect(store.get("alpha")?.lastFailureAt).toBeTypeOf("number");
		expect(store.get("beta")).toMatchObject({ calls: 1, callTransportFailures: 0, connectFailures: 0 });
		expect(store.get("unknown")).toBeUndefined();
	});

	it("persists on flush and reloads in a fresh store", () => {
		const store = createMcpStatsStore(file);
		store.recordCall("alpha", "ok");
		store.recordCall("alpha", "transport_failure");
		store.flush();

		const reloaded = createMcpStatsStore(file);
		expect(reloaded.get("alpha")).toMatchObject({ calls: 2, callTransportFailures: 1 });
	});

	it("records and reads nothing when disabled", () => {
		const store = createMcpStatsStore(file, () => false);
		store.recordCall("alpha", "ok");
		expect(store.get("alpha")).toBeUndefined();
	});

	it("survives a corrupt stats file", () => {
		writeFileSync(file, "{not json");
		const store = createMcpStatsStore(file);
		store.recordCall("alpha", "ok");
		expect(store.get("alpha")?.calls).toBe(1);
	});
});

function stats(partial: Partial<McpServerStats>): McpServerStats {
	return { calls: 0, callTransportFailures: 0, connectFailures: 0, ...partial };
}

describe("formatMcpReliability", () => {
	it("returns undefined without meaningful history", () => {
		expect(formatMcpReliability(undefined)).toBeUndefined();
		expect(formatMcpReliability(stats({ calls: 3 }))).toBeUndefined();
	});

	it("reports success rate once the sample is large enough", () => {
		expect(formatMcpReliability(stats({ calls: 10, callTransportFailures: 1 }))).toBe("90% success over 10 calls");
	});

	it("reports connect failures even with few calls", () => {
		expect(formatMcpReliability(stats({ connectFailures: 2 }))).toBe("2 connect failures");
		expect(formatMcpReliability(stats({ calls: 10, connectFailures: 1 }))).toBe(
			"100% success over 10 calls, 1 connect failure",
		);
	});
});

describe("formatMcpReliabilityWarning", () => {
	it("stays silent for healthy or under-sampled servers", () => {
		expect(formatMcpReliabilityWarning(undefined)).toBeUndefined();
		expect(formatMcpReliabilityWarning(stats({ calls: 100, callTransportFailures: 2 }))).toBeUndefined();
		expect(formatMcpReliabilityWarning(stats({ calls: 3, callTransportFailures: 3 }))).toBeUndefined();
	});

	it("flags a chronically failing server", () => {
		expect(formatMcpReliabilityWarning(stats({ calls: 10, callTransportFailures: 4 }))).toBe(
			"[unreliable: 60% success over 10 calls]",
		);
	});
});
