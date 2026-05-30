/**
 * Deterministic subagent dispatch evaluator.
 *
 * Decides whether a task should be handled inline or delegated to a subagent,
 * which subagent type to use, and whether a task should be split across
 * multiple subagents.  No LLM call — keyword + heuristic only.
 */

export type AgentType = "explore" | "edit" | "test" | "review" | "doc";

export interface TaskAnalysis {
	should_delegate: boolean;
	agent_type: AgentType | null;
	reason: string;
	estimated_complexity: "low" | "medium" | "high";
	parallelizable: boolean;
	context_needed: string[];
}

export interface Subtask {
	agent_type: AgentType;
	prompt: string;
	estimated_files: number;
}

/* ------------------------------------------------------------------ */
// Keyword routing tables

const EXPLORE_KEYWORDS = [
	"explore",
	"understand",
	"scout",
	"investigate",
	"trace",
	"find",
	"where",
	"how does",
	"what is",
	"lookup",
	"search",
	"navigate",
	"discover",
	"map out",
	"get familiar",
];

const EDIT_KEYWORDS = [
	"create",
	"implement",
	"refactor",
	"add",
	"build",
	"change",
	"update",
	"modify",
	"fix",
	"repair",
	"correct",
	"migrate",
	"rename",
	"remove",
	"delete",
	"write",
];

const TEST_KEYWORDS = [
	"test",
	"validate",
	"assert",
	"coverage",
	"jest",
	"vitest",
	"mocha",
	"pytest",
	"unit test",
	"integration test",
	"e2e test",
	"regression test",
];

const REVIEW_KEYWORDS = [
	"review",
	"audit",
	"critique",
	"security",
	"check",
	"inspect",
	"verify",
	"assess",
	"evaluate",
	"analyze for",
	"vulnerab",
	"perf audit",
];

const DOC_KEYWORDS = [
	"readme",
	"documentation",
	"document",
	"comment",
	"explain",
	"docs",
	"guide",
	"tutorial",
	"changelog",
	"api docs",
];

const CROSS_DOMAIN_MARKERS = [
	" and ",
	" as well as ",
	" plus ",
	" then ",
	" after that ",
	" followed by ",
	" in addition ",
	" simultaneously ",
];

/* ------------------------------------------------------------------ */
// Helpers

function countMatches(text: string, keywords: readonly string[]): number {
	const lower = text.toLowerCase();
	return keywords.reduce((count, kw) => count + (lower.includes(kw) ? 1 : 0), 0);
}

function detectAgentType(task: string): AgentType | null {
	const lower = task.toLowerCase();
	const scores: Record<AgentType, number> = {
		explore: countMatches(task, EXPLORE_KEYWORDS),
		edit: countMatches(task, EDIT_KEYWORDS),
		test: countMatches(task, TEST_KEYWORDS),
		review: countMatches(task, REVIEW_KEYWORDS),
		doc: countMatches(task, DOC_KEYWORDS),
	};

	// Boost doc when the task is clearly about documentation
	if (scores.doc > 0 && (lower.includes("readme") || lower.includes("documentation") || lower.includes("document "))) {
		scores.doc += 2;
	}

	// Boost test when the task is clearly about testing
	if (scores.test > 0 && (lower.includes("test") || lower.includes("tests"))) {
		scores.test += 2;
	}

	// Boost review for security-related tasks
	if (scores.review > 0 && lower.includes("security")) {
		scores.review += 2;
	}

	let best: AgentType | null = null;
	let bestScore = 0;
	for (const [type, score] of Object.entries(scores)) {
		if (score > bestScore) {
			bestScore = score;
			best = type as AgentType;
		}
	}
	return bestScore > 0 ? best : null;
}

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

function canHandleInline(task: string): boolean {
	if (estimateComplexity(task) !== "low") return false;

	const fileMatches = task.match(/\b[\w/-]+\.(ts|js|tsx|jsx|py|go|rs|java|cpp|c|h|md|json|yaml|yml|toml)\b/g);
	if (fileMatches && fileMatches.length > 1) return false;

	const hasCrossDomain = CROSS_DOMAIN_MARKERS.some((m) => task.toLowerCase().includes(m));
	if (hasCrossDomain) return false;

	// Exploration: broad tasks delegate; simple lookups can be inline
	const isExplore = detectAgentType(task) === "explore";
	if (isExplore) {
		const broadExplore = /\b(understand|investigate|trace|how does|how is|scout|map out|get familiar)\b/i.test(task);
		return !broadExplore;
	}

	// Documentation tasks always delegate to the doc subagent
	if (detectAgentType(task) === "doc") {
		return false;
	}

	const isReadOnly = countMatches(task, EXPLORE_KEYWORDS) > 0 && countMatches(task, EDIT_KEYWORDS) === 0;
	const isTrivialEdit =
		countMatches(task, EDIT_KEYWORDS) > 0 && !/\b(create|implement|build|refactor|migrate|restructure)\b/i.test(task);

	return isReadOnly || isTrivialEdit;
}

function extractSubtasks(task: string): Subtask[] {
	// Split on sentence boundaries and conjunctions, then classify each segment.
	const segments = task
		.split(/(?:[,;]|\.(?:\s+|$))\s*/)
		.map((s) => s.trim())
		.filter((s) => s.length > 10);

	if (segments.length < 2) {
		// No obvious sentence split — try cross-domain markers
		const parts: string[] = [];
		let remaining = task;
		for (const marker of CROSS_DOMAIN_MARKERS) {
			const idx = remaining.toLowerCase().indexOf(marker);
			if (idx !== -1) {
				parts.push(remaining.slice(0, idx).trim());
				remaining = remaining.slice(idx + marker.length).trim();
			}
		}
		if (parts.length > 0) {
			parts.push(remaining);
			return parts
				.map((p) => {
					const type = detectAgentType(p);
					if (!type) return null;
					const est = estimateComplexity(p);
					return {
						agent_type: type,
						prompt: p,
						estimated_files: est === "high" ? 4 : est === "medium" ? 2 : 1,
					};
				})
				.filter((s): s is Subtask => s !== null);
		}
		return [];
	}

	return segments
		.map((segment) => {
			const type = detectAgentType(segment);
			if (!type) return null;
			const est = estimateComplexity(segment);
			return {
				agent_type: type,
				prompt: segment,
				estimated_files: est === "high" ? 4 : est === "medium" ? 2 : 1,
			};
		})
		.filter((s): s is Subtask => s !== null);
}

/* ------------------------------------------------------------------ */
// Evaluator

export class DispatchEvaluator {
	evaluate(task: string): TaskAnalysis {
		const depth = Number.parseInt(process.env.HOOCODE_SUBAGENT_DEPTH ?? "0", 10);
		if (depth >= 1) {
			return {
				should_delegate: false,
				agent_type: null,
				reason: "Subagents cannot spawn subagents",
				estimated_complexity: "low",
				parallelizable: false,
				context_needed: [],
			};
		}

		const agentType = detectAgentType(task);
		const complexity = estimateComplexity(task);
		const inline = canHandleInline(task);
		const subtasks = extractSubtasks(task);
		const parallelizable = subtasks.length > 1 || (complexity === "high" && subtasks.length > 0);

		if (inline) {
			return {
				should_delegate: false,
				agent_type: null,
				reason: `Simple ${agentType ?? "task"} suitable for inline handling (<50 lines, 1 file, no cross-domain)`,
				estimated_complexity: complexity,
				parallelizable: false,
				context_needed: [],
			};
		}

		return {
			should_delegate: true,
			agent_type: agentType,
			reason: `${agentType ?? "general"} task with ${complexity} complexity requires isolated subagent`,
			estimated_complexity: complexity,
			parallelizable,
			context_needed: parallelizable ? subtasks.map((st) => st.prompt) : [task],
		};
	}

	shouldSplit(task: string): { split: boolean; subtasks: Subtask[] } {
		const subtasks = extractSubtasks(task);
		if (subtasks.length >= 2) {
			return { split: true, subtasks };
		}

		// Check for explicit multi-domain keywords even when sentence splitting failed
		const multiDomain =
			/\b(implement|write|create|refactor|fix|test|review|document|explore)\b.*\b(and|also|plus|then|followed by)\b.*\b(test|review|document|explore|implement|write|create|refactor|fix)\b/i.test(
				task,
			);
		if (!multiDomain) return { split: false, subtasks: [] };

		// Force split using cross-domain markers
		const parts: string[] = [];
		let remaining = task;
		for (const marker of CROSS_DOMAIN_MARKERS) {
			const idx = remaining.toLowerCase().indexOf(marker);
			if (idx !== -1) {
				parts.push(remaining.slice(0, idx).trim());
				remaining = remaining.slice(idx + marker.length).trim();
			}
		}
		if (parts.length === 0) return { split: false, subtasks: [] };
		parts.push(remaining);

		const forcedSubtasks = parts
			.map((p) => {
				const type = detectAgentType(p);
				if (!type) return null;
				const est = estimateComplexity(p);
				return {
					agent_type: type,
					prompt: p,
					estimated_files: est === "high" ? 4 : est === "medium" ? 2 : 1,
				};
			})
			.filter((s): s is Subtask => s !== null);

		return forcedSubtasks.length >= 2 ? { split: true, subtasks: forcedSubtasks } : { split: false, subtasks: [] };
	}

	canHandleInline(task: string): boolean {
		return canHandleInline(task);
	}

	getReason(analysis: TaskAnalysis): string {
		return analysis.reason;
	}
}
