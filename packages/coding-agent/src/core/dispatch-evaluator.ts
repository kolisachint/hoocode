/**
 * Subagent dispatch guard + lightweight task telemetry.
 *
 * The parent agent selects which subagent to delegate to (via the ExecuteTask tool),
 * so this module performs NO keyword routing. It survives for two narrow
 * responsibilities, both cheap and LLM-free:
 *   1. Depth guard — a process may only delegate while its depth is below the
 *      tree-wide cap (HOOCODE_SUBAGENT_MAX_DEPTH, default 1). At the default cap
 *      this means a subagent cannot spawn further subagents.
 *   2. Complexity estimate — a heuristic recorded in the dispatch log for
 *      diagnostics only.
 */

import { canSpawnSubagent, resolveMaxSubagentDepth } from "./subagent-depth.js";

export interface TaskAnalysis {
	/** False only when the depth guard blocks delegation. */
	should_delegate: boolean;
	reason: string;
	estimated_complexity: "low" | "medium" | "high";
}

/** Heuristic complexity estimate from file/line/scope mentions in the task. */
function estimateComplexity(task: string): "low" | "medium" | "high" {
	const fileMatches = task.match(/\b[\w/-]+\.(ts|js|tsx|jsx|py|go|rs|java|cpp|c|h|md|json|yaml|yml|toml)\b/g);
	const fileCount = fileMatches ? fileMatches.length : 0;

	const lineMatch = task.match(/(\d+)\s*(lines?|loc)\b/i);
	const lineCount = lineMatch ? Number.parseInt(lineMatch[1], 10) : 0;

	const highScope = /\b(across|multiple|many|several|all files|rearchitect|redesign|migrate|restructure)\b/i.test(
		task,
	);
	const mediumScope = /\b(2|3|4|5)\s*files?\b/i.test(task) || /\b(few|some|couple)\b/i.test(task);

	if (lineCount > 200 || fileCount >= 4 || highScope) return "high";
	if (lineCount > 50 || fileCount >= 2 || mediumScope) return "medium";
	return "low";
}

export class DispatchEvaluator {
	evaluate(task: string): TaskAnalysis {
		if (!canSpawnSubagent()) {
			const maxDepth = resolveMaxSubagentDepth();
			return {
				should_delegate: false,
				// Preserve the original message at the default cap; report the depth
				// reached when nesting has been opted into.
				reason: maxDepth <= 1 ? "Subagents cannot spawn subagents" : `Maximum subagent depth (${maxDepth}) reached`,
				estimated_complexity: "low",
			};
		}

		return {
			should_delegate: true,
			reason: "delegated to subagent",
			estimated_complexity: estimateComplexity(task),
		};
	}
}
