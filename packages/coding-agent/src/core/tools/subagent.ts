/**
 * Task tool: delegate a focused task to a specialized subagent.
 *
 * Mirrors the Claude Code `Task` tool. The parent agent decides *when* to
 * delegate based on each agent's `description` (there is no deterministic gate)
 * and selects *which* agent via `subagent_type`. The chosen agent runs in a
 * fresh, isolated child process (SubagentPool) and only its final answer is
 * returned to the parent.
 *
 * It is an optional, opt-in tool (enabled via --subagent or the
 * `enableSubagent` setting); see buildSessionOptions in main.ts.
 */

import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { loadAgentRegistry } from "../agent-registry.js";
import { DispatchEvaluator } from "../dispatch-evaluator.js";
import type { ToolDefinition } from "../extensions/types.js";
import { defineTool } from "../extensions/types.js";
import type { SubagentPool, TaskResult } from "../subagent-pool.js";
import { getSubagentPool } from "../subagent-pool-instance.js";
import type { SubagentResultFile } from "../subagent-result.js";
import { taskStore } from "../task-store.js";

/** Render the available agents as a "- name: description" list for prompts. */
function describeAvailableAgents(cwd: string): string {
	const agents = loadAgentRegistry({ cwd }).list();
	if (agents.length === 0) return "(no agents available)";
	return agents.map((a) => `- ${a.name}: ${(a.description.split("\n")[0] ?? "").trim()}`).join("\n");
}

/** System prompt appendix for the main session when the Task tool is enabled.
 *  Instructs the parent agent on when and how to delegate effectively. */
export function buildTaskMainPrompt(cwd: string = process.cwd()): string {
	return `You have access to the **Task** tool. Use it to delegate self-contained tasks to specialized subagents that run in their own isolated context and return only their final answer.

Available agents (choose one via \`subagent_type\`):
${describeAvailableAgents(cwd)}

When to delegate:
1. The work is self-contained and you only need the final result, not intermediate steps.
2. You want to investigate or edit something in parallel without losing your current context or reasoning chain.
3. The task is a discrete unit (explore one module, run one test file, review one PR, fix one isolated bug).
4. You need to run a long command or test suite and wait for its output without blocking your own reasoning.

Guidelines:
- Choose the agent whose description best matches the task.
- Make every task specific and self-contained. The subagent cannot see this conversation; pass all necessary context (files, constraints, prior findings) in \`prompt\`.
- Do NOT delegate tasks that require tight back-and-forth with your current reasoning, or edits to files you are actively reasoning about.
- The subagent returns ONLY its final answer. Its intermediate reasoning, tool calls, and output are hidden from you.
- Default to handling small, quick, or single-file work inline; delegate only self-contained units.
- Some agents are configured to run in the background (non-blocking). For those, Task returns immediately with a task_id; use the **TaskOutput** tool with that task_id to check status and collect the final answer.
- To continue a previous subagent (for example one that returned partial results), call Task again with \`resume_task_id\` set to its task_id; it resumes with its full prior transcript and \`prompt\` is your follow-up.`;
}

const taskParams = Type.Object({
	description: Type.String({
		description: "A short (3-5 word) description of the task, shown in the task panel.",
	}),
	prompt: Type.String({
		description:
			"The full, self-contained task for the subagent. It cannot see this conversation, so include all needed context, files, and constraints.",
	}),
	subagent_type: Type.String({
		description: "The name of the specialized agent to delegate to. Must be one of the available agents.",
	}),
	resume_task_id: Type.Optional(
		Type.String({
			description:
				"Optional. To continue a previous subagent run, pass its task_id (returned by an earlier Task or TaskOutput call). The subagent resumes with its full prior transcript and `prompt` is your follow-up instruction.",
		}),
	),
});

type TaskParams = Static<typeof taskParams>;

export interface TaskToolDetails {
	subagent_type: string;
	ok: boolean;
	error?: string;
	taskId: number;
	/** Pool-level task id usable for resume/polling. */
	poolTaskId?: string;
	/** True when dispatched as a non-blocking background task. */
	background?: boolean;
}

export interface TaskOutputDetails {
	task_id: string;
	status: string;
	ok: boolean;
}

/**
 * A short, human-readable task name for the task panel: the first line limited
 * to ~8 words so it stays glanceable. A character cap guards a single long word.
 */
function summarize(task: string): string {
	const firstLine = (task.trim().split("\n")[0] ?? "").trim();
	if (!firstLine) return "(task)";
	const words = firstLine.split(/\s+/);
	let name = words.length > 8 ? `${words.slice(0, 8).join(" ")}…` : firstLine;
	if (name.length > 60) name = `${name.slice(0, 59)}…`;
	return name;
}

/** Quick advisory check: would the dispatch evaluator delegate this task?
 *  The evaluator is non-blocking; this is exposed for diagnostics/tools only. */
export function isSubagentRecommended(task: string): boolean {
	return new DispatchEvaluator().evaluate(task).should_delegate;
}

/** Create the Task tool definition. Registered as a customTool when enabled. */
export function createTaskToolDefinition(cwd: string = process.cwd()): ToolDefinition {
	const agentList = describeAvailableAgents(cwd);
	return defineTool<typeof taskParams, TaskToolDetails>({
		name: "Task",
		label: "Task",
		description: [
			"Delegate a focused task to a specialized subagent that runs in a fresh, isolated context (it cannot see this conversation).",
			"Select the agent via `subagent_type`; pass everything it needs via `prompt`. The subagent returns only its final answer.",
			"Available agents:",
			agentList,
			"WHEN TO USE: (1) self-contained work where you only need the final result;",
			"(2) parallel investigation/edits without losing your reasoning chain;",
			"(3) a discrete unit (explore one module, run one test file, review one PR, fix one isolated bug, write docs);",
			"(4) a long command or test suite you want to run without blocking your reasoning.",
			"Do NOT use for tasks needing tight back-and-forth with your current reasoning, or edits to files you are actively reasoning about.",
			"Prefer handling small, quick, or single-file tasks yourself; delegate only self-contained units of work.",
		].join("\n"),
		promptSnippet: "delegate a self-contained task to a specialized subagent (choose via subagent_type)",
		parameters: taskParams,

		async execute(_toolCallId, params: TaskParams, _signal, _onUpdate, ctx) {
			const pool = getSubagentPool(ctx.cwd);

			// Resume path: continue a previously dispatched subagent with a follow-up
			// prompt, reusing its persisted session (full prior transcript).
			const resumeId = params.resume_task_id?.trim();
			if (resumeId) {
				const summary = params.description?.trim() || summarize(params.prompt);
				const task = taskStore.create(summary, { subagentMode: params.subagent_type });
				taskStore.update(task.id, { status: "in_progress" });
				try {
					const dispatchResult = await pool.resume(resumeId, params.prompt, {
						model: ctx.model?.id,
						provider: ctx.model?.provider,
					});
					// The session lives under the original task id; keep it as the resume handle.
					return finalizeForegroundResult(dispatchResult, params.subagent_type, task.id, resumeId);
				} catch (error) {
					taskStore.update(task.id, { status: "failed" });
					throw error;
				}
			}

			// The model has already decided to delegate and which agent to use; honor
			// it. Validate the requested agent against the registry (no routing gate).
			const registry = loadAgentRegistry({ cwd: ctx.cwd });
			const def = registry.get(params.subagent_type);
			if (!def) {
				const available = registry
					.list()
					.map((a) => a.name)
					.join(", ");
				throw new Error(
					`Unknown subagent_type: "${params.subagent_type}". Available agents: ${available || "(none)"}.`,
				);
			}

			const summary = params.description?.trim() || summarize(params.prompt);
			const task = taskStore.create(summary, { subagentMode: params.subagent_type });

			// Background agents: dispatch detached and return a handle immediately so
			// the parent keeps reasoning. The parent polls via the TaskOutput tool.
			if (def.background) {
				taskStore.update(task.id, { status: "in_progress" });
				const dispatched = pool.dispatchDetached(params.prompt, {
					forceAgent: params.subagent_type,
					context: "",
					model: ctx.model?.id,
					provider: ctx.model?.provider,
				});
				const poolTaskId = dispatched.task_id;
				if (poolTaskId) trackBackgroundTask(pool, poolTaskId, task.id);
				return {
					content: [
						{
							type: "text" as const,
							text: `Background subagent (${params.subagent_type}) started with task_id "${poolTaskId}". It runs without blocking you. Call the TaskOutput tool with this task_id to check its status and collect the final answer.`,
						},
					],
					details: {
						subagent_type: params.subagent_type,
						ok: true,
						taskId: task.id,
						poolTaskId,
						background: true,
					},
				};
			}

			taskStore.update(task.id, { status: "in_progress" });
			try {
				const dispatchResult = await pool.dispatch(params.prompt, {
					forceAgent: params.subagent_type,
					context: "",
					model: ctx.model?.id,
					provider: ctx.model?.provider,
				});
				return finalizeForegroundResult(dispatchResult, params.subagent_type, task.id, dispatchResult.task_id);
			} catch (error) {
				taskStore.update(task.id, { status: "failed" });
				throw error;
			}
		},

		renderCall(args, theme) {
			const type = args.subagent_type ?? "agent";
			const preview = summarize(args.description ?? args.prompt ?? "");
			const text =
				theme.fg("toolTitle", theme.bold("Task ")) +
				theme.fg("accent", `[${type}]`) +
				theme.fg("dim", ` ${preview}`);
			return new Text(text, 0, 0);
		},
	});
}

/** Extract the final answer from a finished dispatch, updating the task panel. */
function finalizeForegroundResult(
	dispatchResult: TaskResult,
	subagentType: string,
	taskStoreId: number,
	resumeHandle: string | undefined,
): { content: Array<{ type: "text"; text: string }>; details: TaskToolDetails } {
	const result = dispatchResult.result;
	const resultData = result?.result_data as SubagentResultFile | undefined;
	const usage = resultData?.usage;

	if (!result || !result.ok) {
		// Signal failure by throwing: the agent loop derives a tool's error state
		// from a thrown error, not from a returned flag.
		taskStore.update(taskStoreId, { status: "failed", usage });
		const reason = result?.error ?? (result?.status ? `subagent ${result.status}` : "unknown error");
		throw new Error(`Subagent (${subagentType}) failed: ${reason}`);
	}

	// Leave the task in the store with its final status; it stays visible in the
	// task panel until the next user message arrives.
	taskStore.update(taskStoreId, { status: "done", usage });
	let answer = resultData?.summary || "(subagent returned no output)";
	// Partial results are resumable; surface the handle so the parent can continue.
	if (result.status === "partial" && resumeHandle) {
		answer += `\n\n[Partial result. To continue this subagent, call Task again with resume_task_id="${resumeHandle}".]`;
	}
	return {
		content: [{ type: "text", text: answer }],
		details: { subagent_type: subagentType, ok: true, taskId: taskStoreId, poolTaskId: resumeHandle },
	};
}

/**
 * Keep the task panel in sync for a detached background subagent: when the pool
 * reports the task finished, update the stored task's status and detach.
 */
function trackBackgroundTask(pool: SubagentPool, poolTaskId: string, taskStoreId: number): void {
	function finish(status: "done" | "failed"): void {
		taskStore.update(taskStoreId, { status });
		pool.off("task_done", onDone);
		pool.off("task_failed", onFail);
		pool.off("task_stalled", onFail);
		pool.off("task_timeout", onFail);
	}
	function onDone(data: { task_id?: string }): void {
		if (data?.task_id === poolTaskId) finish("done");
	}
	function onFail(data: { task_id?: string }): void {
		if (data?.task_id === poolTaskId) finish("failed");
	}
	pool.on("task_done", onDone);
	pool.on("task_failed", onFail);
	pool.on("task_stalled", onFail);
	pool.on("task_timeout", onFail);
}

const taskOutputParams = Type.Object({
	task_id: Type.String({
		description: "The task_id of a background (or previously dispatched) subagent, as returned by the Task tool.",
	}),
});

type TaskOutputParams = Static<typeof taskOutputParams>;

/**
 * TaskOutput tool: poll a background subagent and collect its final answer.
 * Returns the current status while running, or the subagent's final answer once
 * complete. Registered alongside the Task tool when subagents are enabled.
 */
export function createTaskOutputToolDefinition(): ToolDefinition {
	return defineTool<typeof taskOutputParams, TaskOutputDetails>({
		name: "TaskOutput",
		label: "TaskOutput",
		description: [
			"Check the status of a background subagent and collect its final answer once it finishes.",
			"Pass the task_id returned by a background Task call. While the subagent runs this reports its status; once complete it returns only the subagent's final answer.",
		].join("\n"),
		promptSnippet: "check status / collect the result of a background subagent",
		parameters: taskOutputParams,

		async execute(_toolCallId, params: TaskOutputParams, _signal, _onUpdate, ctx) {
			const pool = getSubagentPool(ctx.cwd);
			const status = pool.get_status(params.task_id);
			if (status === "running" || status === "queued") {
				return {
					content: [
						{
							type: "text" as const,
							text: `Subagent task "${params.task_id}" is ${status}. Call TaskOutput again later to collect its result.`,
						},
					],
					details: { task_id: params.task_id, status, ok: true },
				};
			}

			const result = pool.collect(params.task_id);
			if (!result) {
				throw new Error(
					`No result available for task "${params.task_id}" (status: ${status}). It may not exist or its result was already collected.`,
				);
			}
			if (!result.ok) {
				const reason = result.error ?? (result.status ? `subagent ${result.status}` : status);
				throw new Error(`Background subagent "${params.task_id}" failed: ${reason}`);
			}
			const resultData = result.result_data as SubagentResultFile | undefined;
			const answer = resultData?.summary || "(subagent returned no output)";
			return {
				content: [{ type: "text" as const, text: answer }],
				details: { task_id: params.task_id, status: result.status ?? "complete", ok: true },
			};
		},

		renderCall(args, theme) {
			const text = theme.fg("toolTitle", theme.bold("TaskOutput ")) + theme.fg("dim", String(args.task_id ?? ""));
			return new Text(text, 0, 0);
		},
	});
}
