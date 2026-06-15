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

/** Concurrency for a pool created in the current process: reduced when nested. */
export function poolConcurrencyForDepth(env: NodeJS.ProcessEnv = process.env): number | undefined {
	return currentSubagentDepth(env) >= 1 ? NESTED_SUBAGENT_CONCURRENCY : undefined;
}
