/**
 * Process-wide SubagentPool singleton.
 *
 * The subagent tool and the `/subagent` command both delegate through one pool
 * so concurrency limits, lifeguard monitoring, and token budgets are shared
 * across every delegation in the session. Created lazily on first use and torn
 * down on process exit.
 */

import { getSubagentSpawnCommand } from "../config.js";
import { SubagentPool } from "./subagent-pool.js";

let pool: SubagentPool | undefined;
let override: SubagentPool | undefined;
let exitHandlerRegistered = false;
/** Latest non-default skill paths to forward to subagents, kept in sync with the resource loader. */
let latestSkillPaths: string[] = [];

/** Get the shared pool for a given working directory, creating it on first use. */
export function getSubagentPool(cwd: string): SubagentPool {
	if (override) return override;
	if (!pool) {
		const { executable, prefixArgs } = getSubagentSpawnCommand();
		pool = new SubagentPool({ executable, prefixArgs, cwd, skillPaths: latestSkillPaths });

		if (!exitHandlerRegistered) {
			exitHandlerRegistered = true;
			process.once("exit", () => pool?.dispose());
		}
	}
	return pool;
}

/**
 * Return the shared pool if one already exists, without creating it. Use this for
 * best-effort signaling (e.g. reporting external load) that must not spin up a pool
 * and its lifeguard just because the signal fired before any subagent was dispatched.
 */
export function peekSubagentPool(): SubagentPool | undefined {
	return override ?? pool;
}

/**
 * Update the skill paths forwarded to every subagent.
 * Call this after the resource loader reloads or extends its skill set.
 * If the pool has already been created, updates it immediately.
 * If not, the paths will be passed in when the pool is first created.
 */
export function updateSubagentSkillPaths(paths: string[]): void {
	latestSkillPaths = paths;
	pool?.updateSkillPaths(paths);
}

/** Dispose and clear the shared pool. Intended for test isolation and shutdown. */
export function disposeSubagentPool(): void {
	pool?.dispose();
	pool = undefined;
	latestSkillPaths = [];
}

/**
 * Inject a pool instance for tests, bypassing real child-process spawning.
 * Pass `undefined` to clear the override.
 */
export function setSubagentPoolForTesting(testPool: SubagentPool | undefined): void {
	override = testPool;
}
