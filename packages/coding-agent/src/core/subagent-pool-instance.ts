/**
 * Process-wide SubagentPool singleton.
 *
 * The subagent tool and the `/subagent` command both delegate through one pool
 * so concurrency limits, lifeguard monitoring, and token budgets are shared
 * across every delegation in the session. Created lazily on first use and torn
 * down on process exit.
 */

import { getAgentDir, getSubagentSpawnCommand } from "../config.js";
import { SettingsManager } from "./settings-manager.js";
import { poolConcurrencyForDepth } from "./subagent-depth.js";
import { SubagentPool } from "./subagent-pool.js";
import { taskStore } from "./task-store.js";

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
		// Pools created inside a nested subagent (depth >= 1) run with a reduced
		// concurrency cap so deep delegation trees stay bounded; the root keeps the
		// SubagentPool default.
		// Load settings for model category resolution
		const settingsManager = SettingsManager.create(cwd, getAgentDir());
		const globalSettings = settingsManager.getGlobalSettings();
		const projectSettings = settingsManager.getProjectSettings();
		// Merge settings (project overrides global)
		const settings = { ...globalSettings, ...projectSettings };

		pool = new SubagentPool({
			executable,
			prefixArgs,
			cwd,
			skillPaths: latestSkillPaths,
			maxConcurrency: poolConcurrencyForDepth(),
			settings,
		});

		wireProgressToTaskStore(pool);

		if (!exitHandlerRegistered) {
			exitHandlerRegistered = true;
			process.once("exit", () => pool?.dispose());
		}
	}
	return pool;
}

/**
 * Surface live subagent progress on the task panel's agent roster row. The pool
 * forwards only coarse lifecycle events; we map the currently-executing tool onto
 * the run's `activity` and clear it between tools and on completion. Roster rows
 * are keyed per run by the pool task id (see registerSubagentDispatch), so
 * concurrent same-type subagents update their own rows; patching an unknown id
 * is a no-op. This touches only the roster row, never task nodes — so it cannot
 * collide with the end-of-run task-tree merge. Render coalescing is handled by
 * the TUI's `requestRender`, so per-event patches are fine.
 */
function wireProgressToTaskStore(p: SubagentPool): void {
	p.on("task_progress", (data: { task_id: string; event: { type?: string; toolName?: string } }) => {
		const { task_id, event } = data;
		if (event.type === "tool_execution_start") {
			taskStore.patchAgent(task_id, { activity: typeof event.toolName === "string" ? event.toolName : "" });
		} else if (event.type === "turn_end") {
			// Between turns the subagent is reasoning, not idle — mirror the inbox's
			// "thinking" so the panel and TaskOutput agree on what the run is doing.
			taskStore.patchAgent(task_id, { activity: "thinking" });
		} else if (event.type === "tool_execution_end") {
			taskStore.patchAgent(task_id, { activity: "" });
		}
	});
	for (const terminal of ["task_done", "task_failed", "task_stalled", "task_timeout", "task_cancelled"] as const) {
		p.on(terminal, (data: { task_id?: string }) => {
			if (data.task_id) taskStore.patchAgent(data.task_id, { activity: "" });
		});
	}
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
