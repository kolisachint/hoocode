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

/** Get the shared pool for a given working directory, creating it on first use. */
export function getSubagentPool(cwd: string): SubagentPool {
	if (override) return override;
	if (!pool) {
		const { executable, prefixArgs } = getSubagentSpawnCommand();
		pool = new SubagentPool({ executable, prefixArgs, cwd });

		if (!exitHandlerRegistered) {
			exitHandlerRegistered = true;
			process.once("exit", () => pool?.dispose());
		}
	}
	return pool;
}

/** Dispose and clear the shared pool. Intended for test isolation and shutdown. */
export function disposeSubagentPool(): void {
	pool?.dispose();
	pool = undefined;
}

/**
 * Inject a pool instance for tests, bypassing real child-process spawning.
 * Pass `undefined` to clear the override.
 */
export function setSubagentPoolForTesting(testPool: SubagentPool | undefined): void {
	override = testPool;
}
