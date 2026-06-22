/**
 * subagent-depth tests
 *
 * Covers the env-driven depth contract and the bounded nested-concurrency rule.
 * All helpers accept an explicit env so cases don't touch process.env.
 */

import { describe, expect, it } from "vitest";
import {
	ABSOLUTE_MAX_SUBAGENT_DEPTH,
	canSpawnSubagent,
	clampMaxSubagentDepth,
	currentSubagentDepth,
	DEFAULT_MAX_SUBAGENT_DEPTH,
	delegateAllowList,
	isDelegateAllowed,
	NESTED_SUBAGENT_CONCURRENCY,
	poolConcurrencyForDepth,
	resolveMaxSubagentDepth,
	resolveNestedConcurrency,
	subagentSkipMcp,
	toolAllowlistNeedsMcp,
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

	it("clamps an over-large cap to the hard ceiling (env and setting paths)", () => {
		expect(resolveMaxSubagentDepth(99, env({}))).toBe(ABSOLUTE_MAX_SUBAGENT_DEPTH);
		expect(resolveMaxSubagentDepth(undefined, env({ HOOCODE_SUBAGENT_MAX_DEPTH: "50" }))).toBe(
			ABSOLUTE_MAX_SUBAGENT_DEPTH,
		);
	});
});

describe("clampMaxSubagentDepth", () => {
	it("keeps values within [1, ABSOLUTE_MAX_SUBAGENT_DEPTH] and floors fractions", () => {
		expect(clampMaxSubagentDepth(0)).toBe(1);
		expect(clampMaxSubagentDepth(2)).toBe(2);
		expect(clampMaxSubagentDepth(2.9)).toBe(2);
		expect(clampMaxSubagentDepth(999)).toBe(ABSOLUTE_MAX_SUBAGENT_DEPTH);
		expect(clampMaxSubagentDepth(Number.NaN)).toBe(DEFAULT_MAX_SUBAGENT_DEPTH);
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

	it("honors a configured nested concurrency from the env", () => {
		expect(
			poolConcurrencyForDepth(env({ HOOCODE_SUBAGENT_DEPTH: "1", HOOCODE_NESTED_SUBAGENT_CONCURRENCY: "4" })),
		).toBe(4);
		// Root still uses the pool default regardless of the nested setting.
		expect(poolConcurrencyForDepth(env({ HOOCODE_NESTED_SUBAGENT_CONCURRENCY: "4" }))).toBeUndefined();
	});
});

describe("delegate scoping", () => {
	it("is unrestricted when the env var is absent", () => {
		expect(isDelegateAllowed("explore", env({}))).toBe(true);
		expect(delegateAllowList(env({}))).toBeUndefined();
	});

	it("restricts to the listed types when set", () => {
		const e = env({ HOOCODE_DELEGATE_ALLOW: "explore, plan" });
		expect(delegateAllowList(e)).toEqual(["explore", "plan"]);
		expect(isDelegateAllowed("explore", e)).toBe(true);
		expect(isDelegateAllowed("general-purpose", e)).toBe(false);
	});
});

describe("MCP skip for subagents", () => {
	it("needs MCP when the allowlist is undefined (inherit all tools)", () => {
		expect(toolAllowlistNeedsMcp(undefined)).toBe(true);
	});

	it("does not need MCP when the explicit allowlist is MCP-free", () => {
		expect(toolAllowlistNeedsMcp(["read", "grep", "find", "ls"])).toBe(false);
		expect(toolAllowlistNeedsMcp([])).toBe(false);
		// Delegating agents get Task/TaskOutput appended — still not MCP.
		expect(toolAllowlistNeedsMcp(["read", "Task", "TaskOutput"])).toBe(false);
	});

	it("needs MCP when the allowlist references an mcp tool (prefix-tolerant)", () => {
		expect(toolAllowlistNeedsMcp(["read", "mcp_github_search"])).toBe(true);
		expect(toolAllowlistNeedsMcp([" mcp-foo "])).toBe(true);
		expect(toolAllowlistNeedsMcp(["MCP_Github_Issue"])).toBe(true);
	});

	it("subagentSkipMcp reads the env flag", () => {
		expect(subagentSkipMcp(env({}))).toBe(false);
		expect(subagentSkipMcp(env({ HOOCODE_SKIP_MCP: "1" }))).toBe(true);
		expect(subagentSkipMcp(env({ HOOCODE_SKIP_MCP: "0" }))).toBe(false);
	});
});

describe("resolveNestedConcurrency", () => {
	it("prefers env, then setting, then default; clamps to >= 1", () => {
		expect(resolveNestedConcurrency(undefined, env({}))).toBe(NESTED_SUBAGENT_CONCURRENCY);
		expect(resolveNestedConcurrency(3, env({}))).toBe(3);
		expect(resolveNestedConcurrency(0, env({}))).toBe(NESTED_SUBAGENT_CONCURRENCY);
		expect(resolveNestedConcurrency(2, env({ HOOCODE_NESTED_SUBAGENT_CONCURRENCY: "5" }))).toBe(5);
	});
});
