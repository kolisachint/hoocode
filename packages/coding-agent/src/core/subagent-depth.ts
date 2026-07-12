/**
 * Subagent nesting depth + tree-wide safety bounds.
 *
 * Nesting is governed entirely by environment variables so every process in a
 * delegation tree agrees without any runtime cross-process coordination (no
 * shared lock files, no slots to leak on crash):
 *
 *   - HOOCODE_SUBAGENT_DEPTH      current process's depth (root unset/0, its
 *                                 children 1, grandchildren 2, ...).
 *   - HOOCODE_SUBAGENT_MAX_DEPTH  the tree-wide cap, seeded once by the root from
 *                                 its `maxSubagentDepth` setting and inherited by
 *                                 every descendant.
 *
 * The default cap is 1 — a subagent may not spawn further subagents — which
 * reproduces the original hard guard exactly. Raising it is an opt-in feature.
 *
 * Fan-out is bounded deterministically rather than by a global counter: pools at
 * depth >= 1 run with NESTED_SUBAGENT_CONCURRENCY instead of the root's default,
 * so the worst-case live process count is a fixed function of depth and the per
 * level caps (e.g. 5 + 5*2 = 15 at depth 2), with nothing to reclaim on crash.
 */

export const SUBAGENT_DEPTH_ENV = "HOOCODE_SUBAGENT_DEPTH";
export const SUBAGENT_MAX_DEPTH_ENV = "HOOCODE_SUBAGENT_MAX_DEPTH";
export const NESTED_CONCURRENCY_ENV = "HOOCODE_NESTED_SUBAGENT_CONCURRENCY";
/** Comma-separated allowlist of subagent types the current process may delegate to. */
export const DELEGATE_ALLOW_ENV = "HOOCODE_DELEGATE_ALLOW";
/**
 * Set by the parent pool when a spawned subagent's tool allowlist contains no MCP
 * tools, telling the child's MCP loader to skip connecting external servers at
 * startup. Connecting them (one ~15s handshake apiece) is pure boot latency for a
 * subagent that can never call them. Only set when the allowlist is explicit and
 * MCP-free; an inherit-all agent leaves it unset and connects as usual.
 */
export const SUBAGENT_SKIP_MCP_ENV = "HOOCODE_SKIP_MCP";

/**
 * When set to "1", the MCP loader defers tool *schemas*: it injects tool names
 * only (via a ResolveMcpTools tool) and materializes each full schema on demand,
 * instead of registering every MCP tool's full JSON schema up front (spec §2).
 *
 * Set on the top-level agent (via the deferMcpSchemas setting, default on) and
 * cleared for subagent children — a child that needs MCP resolves its allowlisted
 * tools eagerly at dispatch (the dispatch ↔ schema interaction), so its scoped
 * tools are immediately callable.
 */
export const DEFER_MCP_SCHEMAS_ENV = "HOOCODE_DEFER_MCP_SCHEMAS";

/** Default tree-wide cap: subagents cannot spawn subagents (original behavior). */
export const DEFAULT_MAX_SUBAGENT_DEPTH = 1;

/** Concurrency cap for pools running at depth >= 1, keeping nested fan-out bounded. */
export const NESTED_SUBAGENT_CONCURRENCY = 2;

/**
 * Hard ceiling on the configurable nesting depth. The worst-case live process
 * count grows geometrically with depth (each level's pool can run
 * NESTED_SUBAGENT_CONCURRENCY children), so an unbounded cap would let a
 * mis-configured setting exhaust the host. At this ceiling the worst case stays
 * modest: 5 * (2^3 - 1) = 35 processes.
 */
export const ABSOLUTE_MAX_SUBAGENT_DEPTH = 3;

/** Clamp a requested cap into the supported range [1, ABSOLUTE_MAX_SUBAGENT_DEPTH]. */
export function clampMaxSubagentDepth(n: number): number {
	if (!Number.isFinite(n)) return DEFAULT_MAX_SUBAGENT_DEPTH;
	return Math.min(Math.max(1, Math.floor(n)), ABSOLUTE_MAX_SUBAGENT_DEPTH);
}

/** Depth of the current process (0 = root/main session). */
export function currentSubagentDepth(env: NodeJS.ProcessEnv = process.env): number {
	const n = Number.parseInt(env[SUBAGENT_DEPTH_ENV] ?? "0", 10);
	return Number.isFinite(n) && n > 0 ? n : 0;
}

/**
 * Tree-wide max depth. Reads the inherited env value when present (any spawned
 * process), otherwise falls back to the provided setting (the root seeds env
 * from this). Clamped to >= 1 so the cap can never disable delegation entirely.
 */
export function resolveMaxSubagentDepth(settingValue?: number, env: NodeJS.ProcessEnv = process.env): number {
	const raw = env[SUBAGENT_MAX_DEPTH_ENV];
	if (raw !== undefined) {
		const n = Number.parseInt(raw, 10);
		if (Number.isFinite(n) && n >= 1) return clampMaxSubagentDepth(n);
	}
	if (settingValue !== undefined && Number.isFinite(settingValue) && settingValue >= 1) {
		return clampMaxSubagentDepth(settingValue);
	}
	return DEFAULT_MAX_SUBAGENT_DEPTH;
}

/** True when a process at the current depth may still spawn subagents. */
export function canSpawnSubagent(settingValue?: number, env: NodeJS.ProcessEnv = process.env): boolean {
	return currentSubagentDepth(env) < resolveMaxSubagentDepth(settingValue, env);
}

/**
 * Concurrency cap for pools running at depth >= 1. Reads the inherited env value
 * (seeded by the root from the `nestedSubagentConcurrency` setting) when present,
 * else the provided setting, else the default. Clamped to >= 1.
 */
export function resolveNestedConcurrency(settingValue?: number, env: NodeJS.ProcessEnv = process.env): number {
	const raw = env[NESTED_CONCURRENCY_ENV];
	if (raw !== undefined) {
		const n = Number.parseInt(raw, 10);
		if (Number.isFinite(n) && n >= 1) return n;
	}
	if (settingValue !== undefined && Number.isFinite(settingValue) && settingValue >= 1) {
		return Math.floor(settingValue);
	}
	return NESTED_SUBAGENT_CONCURRENCY;
}

/** Concurrency for a pool created in the current process: reduced when nested. */
export function poolConcurrencyForDepth(env: NodeJS.ProcessEnv = process.env): number | undefined {
	return currentSubagentDepth(env) >= 1 ? resolveNestedConcurrency(undefined, env) : undefined;
}

/**
 * Subagent types the current process is restricted to delegating to, or undefined
 * when unrestricted (may delegate to any type). Set per spawned agent from its
 * `delegate: <types>` frontmatter; the root is always unrestricted.
 */
export function delegateAllowList(env: NodeJS.ProcessEnv = process.env): string[] | undefined {
	const raw = env[DELEGATE_ALLOW_ENV];
	if (raw === undefined) return undefined;
	const list = raw
		.split(",")
		.map((s) => s.trim())
		.filter((s) => s.length > 0);
	return list.length > 0 ? list : undefined;
}

/** Whether the current process may delegate to the given subagent type. */
export function isDelegateAllowed(subagentType: string, env: NodeJS.ProcessEnv = process.env): boolean {
	const allow = delegateAllowList(env);
	return !allow || allow.includes(subagentType);
}

/** Whether this process should skip connecting MCP servers at startup (see SUBAGENT_SKIP_MCP_ENV). */
export function subagentSkipMcp(env: NodeJS.ProcessEnv = process.env): boolean {
	return env[SUBAGENT_SKIP_MCP_ENV] === "1";
}

/** Whether this process should defer MCP tool schemas (see DEFER_MCP_SCHEMAS_ENV). */
export function deferMcpSchemas(env: NodeJS.ProcessEnv = process.env): boolean {
	return env[DEFER_MCP_SCHEMAS_ENV] === "1";
}

/**
 * Decide whether a child with the given tool allowlist needs MCP servers. A tool
 * name is MCP-sourced when it carries the `mcp_<server>_<tool>` prefix. An
 * undefined allowlist means "inherit every tool", so MCP must stay available.
 */
export function toolAllowlistNeedsMcp(tools: readonly string[] | undefined): boolean {
	if (!tools) return true;
	return tools.some((t) => /^mcp[_-]?/i.test(t.trim()));
}
