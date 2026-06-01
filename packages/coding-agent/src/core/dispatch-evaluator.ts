/**
 * Subagent dispatch guard + lightweight task telemetry.
 *
 * The parent agent selects which subagent to delegate to (via the Task tool),
 * so this module performs NO keyword routing. It survives for two narrow
 * responsibilities, both cheap and LLM-free:
 *   1. Depth guard — a subagent (HOOCODE_SUBAGENT_DEPTH>=1) must not spawn
 *      further subagents.
 *   2. Complexity estimate — a heuristic recorded in the dispatch log for
 *      diagnostics only.
 */

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
		const depth = Number.parseInt(process.env.HOOCODE_SUBAGENT_DEPTH ?? "0", 10);
		if (depth >= 1) {
			return {
				should_delegate: false,
				reason: "Subagents cannot spawn subagents",
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
