/**
 * Subagent tool: delegate a focused task to a fresh, isolated agent loop.
 *
 * The tool registers a task in the shared task store (visible in the task panel),
 * runs the subagent to completion, and returns ONLY the subagent's final
 * answer. It is an optional, opt-in tool (enabled via --subagent or the
 * `enableSubagent` setting); see buildSessionOptions in main.ts.
 */

/** System prompt appendix for the main session when subagent tooling is enabled.
 *  Instructs the parent agent on when and how to delegate effectively. */
export const SUBAGENT_MAIN_PROMPT = `You have access to the **subagent** tool. Use it to delegate self-contained tasks to isolated subagent loops that run with their own context and return only their final answer.

Available subagent modes:
- explore: read-only investigation (read, grep, find, ls, bash).
- edit: make a focused code change (read, edit, write, grep, find, ls, bash).
- test: run tests and report (read, bash, grep, find, ls).
- fix: diagnose and fix a failure (read, edit, write, bash, grep, find, ls).
- review: read-only code review (read, grep, find, ls, bash).
- doc: write documentation, README, or comments (read, write, edit, grep, find, ls, bash).

When to delegate:
1. The work is self-contained and you only need the final result, not intermediate steps.
2. You want to investigate or edit something in parallel without losing your current context or reasoning chain.
3. The task is a discrete unit (explore one module, run one test file, review one PR, fix one isolated bug).
4. You need to run a long command or test suite and wait for its output without blocking your own reasoning.

Guidelines:
- Make every task specific and self-contained. The subagent cannot see this conversation.
- Pass all necessary context (files, constraints, prior findings) via the \`context\` parameter.
- Do NOT delegate tasks that require tight back-and-forth with your current reasoning.
- Do NOT delegate edits to files you are actively reasoning about.
- The subagent returns ONLY its final answer. Intermediate reasoning, tool calls, and output are hidden from you.

Dispatch evaluator:
- The dispatch evaluator determines if a subagent is needed. Do not spawn subagents directly unless the user explicitly requests it.
- For simple single-file changes (<50 lines, read-only or trivial edit), handle them inline.
- Use force=true to bypass evaluation when you are certain a subagent is required.`;

import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { type AgentType, DispatchEvaluator } from "../dispatch-evaluator.js";
import type { ToolDefinition } from "../extensions/types.js";
import { defineTool } from "../extensions/types.js";
import { runSubagent, type SubagentMode } from "../subagent.js";
import { taskStore } from "../task-store.js";

const subagentParams = Type.Object({
	task: Type.String({
		description:
			"The task to delegate. Make it specific and self-contained; the subagent cannot see this conversation.",
	}),
	context: Type.String({
		description:
			'Context distilled from the conversation the subagent needs (files, constraints, prior findings). Pass "" if none.',
	}),
	mode: Type.Union(
		[
			Type.Literal("explore"),
			Type.Literal("edit"),
			Type.Literal("test"),
			Type.Literal("fix"),
			Type.Literal("review"),
			Type.Literal("doc"),
		],
		{
			description:
				"explore: read-only investigation. edit: make a focused code change. test: run tests and report. fix: diagnose and fix a failure. review: read-only code review. doc: write documentation.",
		},
	),
	force: Type.Boolean({
		description:
			"Bypass dispatch evaluation and spawn the subagent directly. Use when you are certain a subagent is required.",
		default: false,
	}),
});

type SubagentParams = Static<typeof subagentParams>;

export interface SubagentToolDetails {
	mode: SubagentMode;
	ok: boolean;
	error?: string;
	taskId: number;
	/** True when the evaluator handled the task inline instead of delegating. */
	inline?: boolean;
}

/**
 * A short, human-readable task name for the task panel: the first line of the
 * task limited to ~4–8 words so it stays glanceable in the pane. A character cap
 * guards against a single very long word.
 */
function summarize(task: string): string {
	const firstLine = (task.trim().split("\n")[0] ?? "").trim();
	if (!firstLine) return "(task)";
	const words = firstLine.split(/\s+/);
	let name = words.length > 8 ? `${words.slice(0, 8).join(" ")}…` : firstLine;
	if (name.length > 60) name = `${name.slice(0, 59)}…`;
	return name;
}

/** Quick check: should this task go to a subagent? */
export function isSubagentRecommended(task: string): boolean {
	const evaluator = new DispatchEvaluator();
	return evaluator.evaluate(task).should_delegate;
}

/** Map evaluator AgentType to SubagentMode (they are compatible strings). */
function toSubagentMode(agentType: AgentType | null): SubagentMode {
	return (agentType as SubagentMode) ?? "explore";
}

/** Create the subagent tool definition. Registered as a customTool when enabled. */
export function createSubagentToolDefinition(): ToolDefinition {
	return defineTool<typeof subagentParams, SubagentToolDetails>({
		name: "subagent",
		label: "Subagent",
		description: [
			"Delegate a focused task to a subagent that runs in a fresh, isolated context (it cannot see this conversation).",
			"Pass everything it needs via `context`. The subagent returns only its final answer.",
			"Modes: explore, edit, test, fix, review, doc.",
			"WHEN TO USE: (1) The work is self-contained and you do not need to see intermediate steps — only the final result.",
			"(2) You want to investigate or edit something in parallel without losing your current context or reasoning chain.",
			"(3) The task is a discrete unit (explore one module, run one test file, review one PR, fix one isolated bug, write docs).",
			"(4) You need to run a long command or test suite and wait for its output without blocking your own reasoning.",
			"Do NOT use for tasks that require tight back-and-forth with your current reasoning or that change files you are actively reasoning about.",
			"Use force=true to bypass dispatch evaluation when you are certain a subagent is required.",
		].join(" "),
		promptSnippet: "delegate a self-contained task to an isolated subagent (modes: explore/edit/test/fix/review/doc)",
		parameters: subagentParams,

		async execute(_toolCallId, params: SubagentParams, signal, _onUpdate, ctx) {
			const forcedMode = params.mode as SubagentMode;

			// Dispatch evaluation
			if (!params.force) {
				const evaluator = new DispatchEvaluator();
				const analysis = evaluator.evaluate(params.task);
				if (!analysis.should_delegate) {
					return {
						content: [
							{
								type: "text",
								text: `Task is simple enough for inline handling. Reason: ${analysis.reason}. Use force=true for subagent override.`,
							},
						],
						details: { mode: forcedMode, ok: true, taskId: 0, inline: true },
					};
				}
			}

			const mode = params.force
				? forcedMode
				: toSubagentMode(new DispatchEvaluator().evaluate(params.task).agent_type);
			const summary = summarize(params.task);

			const task = taskStore.create(summary, { subagentMode: mode });
			taskStore.update(task.id, { status: "in_progress" });
			try {
				const result = await runSubagent({
					task: params.task,
					context: params.context,
					mode,
					cwd: ctx.cwd,
					model: ctx.model,
					modelRegistry: ctx.modelRegistry,
					signal: signal ?? ctx.signal,
				});

				if (!result.ok) {
					// Signal failure by throwing: the agent loop derives a tool's error
					// state from a thrown error, not from a returned flag.
					taskStore.update(task.id, { status: "failed", usage: result.usage });
					throw new Error(`Subagent (${mode}) failed: ${result.error ?? "unknown error"}`);
				}

				// Leave the task in the store with its final status. It stays visible in
				// the task panel until the next user message arrives (retireFinished is
				// called when the user starts the next turn).
				taskStore.update(task.id, { status: "done", usage: result.usage });
				return {
					content: [{ type: "text", text: result.answer || "(subagent returned no output)" }],
					details: { mode, ok: true, taskId: task.id },
				};
			} catch (error) {
				taskStore.update(task.id, { status: "failed" });
				throw error;
			}
		},

		renderCall(args, theme) {
			const mode = args.mode ?? "explore";
			const preview = summarize(args.task ?? "");
			const text =
				theme.fg("toolTitle", theme.bold("subagent ")) +
				theme.fg("accent", `[${mode}]`) +
				theme.fg("dim", ` ${preview}`);
			return new Text(text, 0, 0);
		},
	});
}
