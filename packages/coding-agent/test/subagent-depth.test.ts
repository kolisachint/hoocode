/**
 * subagent-depth tests
 *
 * Covers the env-driven depth contract and the bounded nested-concurrency rule.
 * All helpers accept an explicit env so cases don't touch process.env.
 */

import { describe, expect, it } from "vitest";
import {
	canSpawnSubagent,
	currentSubagentDepth,
	DEFAULT_MAX_SUBAGENT_DEPTH,
	NESTED_SUBAGENT_CONCURRENCY,
	poolConcurrencyForDepth,
	resolveMaxSubagentDepth,
} from "../src/core/subagent-depth.js";

const env = (overrides: Record<string, string>): NodeJS.ProcessEnv => ({ ...overrides });

describe("currentSubagentDepth", () => {
	it("treats missing/zero/garbage as root depth 0", () => {
		expect(currentSubagentDepth(env({}))).toBe(0);
		expect(currentSubagentDepth(env({ HOOCODE_SUBAGENT_DEPTH: "0" }))).toBe(0);
		expect(currentSubagentDepth(env({ HOOCODE_SUBAGENT_DEPTH: "nope" }))).toBe(0);
	});

	it("reads positive depths", () => {
		expect(currentSubagentDepth(env({ HOOCODE_SUBAGENT_DEPTH: "2" }))).toBe(2);
	});
});

describe("resolveMaxSubagentDepth", () => {
	it("defaults to the original cap of 1", () => {
		expect(resolveMaxSubagentDepth(undefined, env({}))).toBe(DEFAULT_MAX_SUBAGENT_DEPTH);
		expect(DEFAULT_MAX_SUBAGENT_DEPTH).toBe(1);
	});

	it("prefers the inherited env value over the setting", () => {
		expect(resolveMaxSubagentDepth(3, env({ HOOCODE_SUBAGENT_MAX_DEPTH: "2" }))).toBe(2);
	});

	it("falls back to the setting when env is absent, clamped to >= 1", () => {
		expect(resolveMaxSubagentDepth(2, env({}))).toBe(2);
		expect(resolveMaxSubagentDepth(0, env({}))).toBe(1);
		expect(resolveMaxSubagentDepth(-5, env({}))).toBe(1);
	});
});

describe("canSpawnSubagent", () => {
	it("blocks at or beyond the cap, allows below it", () => {
		// default cap 1: root delegates, depth-1 child does not
		expect(canSpawnSubagent(undefined, env({}))).toBe(true);
		expect(canSpawnSubagent(undefined, env({ HOOCODE_SUBAGENT_DEPTH: "1" }))).toBe(false);
		// raised cap 2: depth-1 child delegates, depth-2 grandchild does not
		const raised = { HOOCODE_SUBAGENT_MAX_DEPTH: "2" };
		expect(canSpawnSubagent(undefined, env({ ...raised, HOOCODE_SUBAGENT_DEPTH: "1" }))).toBe(true);
		expect(canSpawnSubagent(undefined, env({ ...raised, HOOCODE_SUBAGENT_DEPTH: "2" }))).toBe(false);
	});
});

describe("poolConcurrencyForDepth", () => {
	it("returns undefined at the root (use pool default) and a reduced cap when nested", () => {
		expect(poolConcurrencyForDepth(env({}))).toBeUndefined();
		expect(poolConcurrencyForDepth(env({ HOOCODE_SUBAGENT_DEPTH: "1" }))).toBe(NESTED_SUBAGENT_CONCURRENCY);
		expect(poolConcurrencyForDepth(env({ HOOCODE_SUBAGENT_DEPTH: "2" }))).toBe(NESTED_SUBAGENT_CONCURRENCY);
	});
});
