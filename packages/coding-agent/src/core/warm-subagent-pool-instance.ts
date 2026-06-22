/**
 * Process-wide WarmSubagentPool singleton + enablement gate.
 *
 * Mirrors subagent-pool-instance: one shared warm pool per process so worker
 * reuse and idle reclaim are coordinated across every dispatch. Created lazily on
 * first use and disposed on exit. Enablement is carried in the environment
 * (HOOCODE_WARM_SUBAGENTS) so it is set once at the root and is trivially
 * readable from the Task tool without threading a setting through every call.
 */

import { getAgentDir } from "../config.js";
import { SettingsManager } from "./settings-manager.js";
import { WarmSubagentPool } from "./warm-subagent-pool.js";

/** Set to "1" at the root when warm subagents are enabled (flag or setting). */
export const WARM_SUBAGENTS_ENV = "HOOCODE_WARM_SUBAGENTS";

let pool: WarmSubagentPool | undefined;
let override: WarmSubagentPool | undefined;
let exitHandlerRegistered = false;
let latestSkillPaths: string[] = [];

/** Whether warm-subagent dispatch is enabled for this process. */
export function warmSubagentsEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
	return env[WARM_SUBAGENTS_ENV] === "1";
}

/** Get the shared warm pool for a working directory, creating it on first use. */
export function getWarmSubagentPool(cwd: string): WarmSubagentPool {
	if (override) return override;
	if (!pool) {
		const settingsManager = SettingsManager.create(cwd, getAgentDir());
		const settings = { ...settingsManager.getGlobalSettings(), ...settingsManager.getProjectSettings() };
		pool = new WarmSubagentPool(cwd, settings, latestSkillPaths);
		if (!exitHandlerRegistered) {
			exitHandlerRegistered = true;
			process.once("exit", () => void pool?.dispose());
		}
	}
	return pool;
}

/** Update the skill paths forwarded to new warm workers (kept in sync with the resource loader). */
export function updateWarmSubagentSkillPaths(paths: string[]): void {
	latestSkillPaths = paths;
	pool?.updateSkillPaths(paths);
}

/** Dispose and clear the shared warm pool. Intended for test isolation and shutdown. */
export function disposeWarmSubagentPool(): void {
	void pool?.dispose();
	pool = undefined;
	latestSkillPaths = [];
}

/** Inject a pool instance for tests, bypassing real child-process spawning. */
export function setWarmSubagentPoolForTesting(testPool: WarmSubagentPool | undefined): void {
	override = testPool;
}
