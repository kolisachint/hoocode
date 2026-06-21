/**
 * Task tool: delegate a focused task to a specialized subagent.
 *
 * Mirrors the Claude Code `Task` tool. The parent agent decides *when* to
 * delegate based on each agent's `description` (there is no deterministic gate)
 * and selects *which* agent via `subagent_type`. The chosen agent runs in a
 * fresh, isolated child process (SubagentPool) and only its final answer is
 * returned to the parent.
 *
 * It is an optional, opt-in tool (enabled via --enable-subagents or the
 * `enableSubagent` setting); see buildSessionOptions in main.ts.
 */

import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { getAgentDir } from "../../config.js";
import { type AgentDefinition, EXECUTE_TASK_TOOL_NAME, MODEL_INHERIT } from "../agent-frontmatter.js";
import { loadAgentRegistry } from "../agent-registry.js";
import type { ToolDefinition } from "../extensions/types.js";
import { defineTool } from "../extensions/types.js";
import { resolveModelReference } from "../model-categories.js";
import { getProviderExhaustion } from "../provider-health.js";
import { SessionManager } from "../session-manager.js";
import { SettingsManager } from "../settings-manager.js";
import { delegateAllowList, isDelegateAllowed } from "../subagent-depth.js";
import type { SubagentResult, TaskResult } from "../subagent-pool.js";
import { getSubagentPool } from "../subagent-pool-instance.js";
import type { SubagentResultFile, SubagentTaskNode } from "../subagent-result.js";
import { taskStore } from "../task-store.js";

/**
 * Condense a (possibly multi-line, bulleted) agent description into a single
 * useful one-liner for the agent picker list.
 *
 * Built-in agent descriptions open with a boilerplate header ("Use this
 * subagent ONLY when:") followed by "when to use" bullets and a "DO NOT use"
 * section. Taking the first line alone yields that identical header for every
 * agent, so instead surface the first meaningful bullets (or the first prose
 * line) from the positive "when to use" region.
 */
export function summarizeAgentDescription(description: string): string {
	const lines = description
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.length > 0);
	if (lines.length === 0) return "";

	// Keep only the positive region: everything before a "DO NOT use" section.
	const stop = lines.findIndex((line) => /^(do\s*not|don'?t|avoid)\b/i.test(line));
	const region = stop === -1 ? lines : lines.slice(0, stop);

	// Drop a leading header line (e.g. "Use this subagent ONLY when:").
	const body = region.length > 1 && region[0]!.endsWith(":") ? region.slice(1) : region;

	const stripBullet = (line: string) => line.replace(/^[-*\u2022]\s+/, "").trim();
	const bullets = body
		.filter((line) => /^[-*\u2022]\s+/.test(line))
		.map(stripBullet)
		.filter((line) => line.length > 0);

	const summary = bullets.length > 0 ? bullets.slice(0, 3).join("; ") : (body[0] ?? lines[0] ?? "").replace(/:$/, "");

	const MAX = 200;
	return summary.length > MAX ? `${summary.slice(0, MAX - 1).trimEnd()}\u2026` : summary;
}

/** Render the available agents as a "- name: description" list for prompts. */
function describeAvailableAgents(cwd: string): string {
	const agents = loadAgentRegistry({ cwd }).list();
	if (agents.length === 0) return "(no agents available)";
	return agents.map((a) => `- ${a.name}: ${summarizeAgentDescription(a.description)}`).join("\n");
}

/** System prompt appendix for the main session when the ExecuteTask tool is enabled.
 *  Instructs the parent agent on when and how to delegate effectively. */
export function buildTaskMainPrompt(cwd: string = process.cwd()): string {
	return `You have access to the **ExecuteTask** tool. Use it to delegate self-contained tasks to specialized subagents that run in their own isolated context and return only their final answer.

Available agents (choose one via \`subagent_type\`):
${describeAvailableAgents(cwd)}

When to delegate:
1. The work is self-contained and you only need the final result, not intermediate steps.
2. You want to investigate or edit something in parallel without losing your current context or reasoning chain.
3. The task is a discrete unit (explore one module, run one test file, review one PR, fix one isolated bug).
4. You need to run a long command or test suite and wait for its output without blocking your own reasoning.

Model categories:
- \`complexity: "fast\`\` — quick reads, simple lookups, grep/find operations
- \`complexity: "standard\`\` — multi-file edits, moderate reasoning, test runs
- \`complexity: "capable\`\` — deep architecture changes, complex refactors, full PRs
If omitted, the agent's default model is used.

Guidelines:
- Choose the agent whose description best matches the task.
- Make every task specific and self-contained. The subagent cannot see this conversation; pass all necessary context (files, constraints, prior findings) in \`prompt\`.
- Do NOT delegate tasks that require tight back-and-forth with your current reasoning, or edits to files you are actively reasoning about.
- The subagent returns ONLY its final answer. Its intermediate reasoning, tool calls, and output are hidden from you.
- Delegate proactively when work is self-contained or parallelizable: multi-step investigation, read-only exploration (use \`explore\`), research before changes (use \`plan\`), drafting a standalone file/section, or running a long command/test suite. Dispatch independent subtasks in the same turn. Handle only trivial single-step edits or tightly interactive back-and-forth inline.
- All ExecuteTask calls run as background tasks (non-blocking). The agent loop continues while the subagent runs. Results arrive as follow-up messages automatically. Use TaskOutput to poll for results while a subagent is running.
- To continue a previous subagent (for example one that returned partial results), call ExecuteTask again with \`resume_task_id\` set to its task_id; it resumes with its full prior transcript and \`prompt\` is your follow-up.`;
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
	item_id: Type.Optional(
		Type.Number({
			description:
				"Optional. References a TodoWrite item id to link this dispatch to a plan item. The task panel shows the linkage.",
		}),
	),
	complexity: Type.Optional(
		Type.Union([Type.Literal("fast"), Type.Literal("standard"), Type.Literal("capable")], {
			description:
				"Model category: fast (quick reads/lookups), standard (multi-file edits), capable (deep architecture). Omit to use the agent's default model.",
		}),
	),
	resume_task_id: Type.Optional(
		Type.String({
			description:
				"Optional. To continue a previous subagent run, pass its task_id (returned by an earlier ExecuteTask or TaskOutput call). The subagent resumes with its full prior transcript and `prompt` is your follow-up instruction.",
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

/** Create the ExecuteTask tool definition. Registered as a customTool when enabled. */
export function createExecuteTaskToolDefinition(cwd: string = process.cwd()): ToolDefinition {
	const agentList = describeAvailableAgents(cwd);
	// All ExecuteTask calls run as background tools. The agent loop runs the
	// dispatch detached: the parent keeps reasoning and the subagent's answer is
	// injected as a follow-up message when it finishes. TaskOutput can poll for
	// results while the subagent runs.
	return defineTool<typeof taskParams, TaskToolDetails>({
		name: EXECUTE_TASK_TOOL_NAME,
		label: EXECUTE_TASK_TOOL_NAME,
		background: () => true,
		description: [
			"Delegate a focused task to a specialized subagent that runs in a fresh, isolated context (it cannot see this conversation).",
			"Select the agent via `subagent_type`; pass everything it needs via `prompt`. The subagent returns only its final answer.",
			"Available agents:",
			agentList,
			"Model categories: fast (quick reads/lookups), standard (multi-file edits), capable (deep architecture). Omit to use agent default.",
			"All ExecuteTask calls run as background tasks. The agent loop continues while the subagent runs. Results arrive as follow-up messages.",
			"Use TaskOutput to poll for results while a subagent is running.",
			"WHEN TO USE: (1) self-contained work where you only need the final result;",
			"(2) parallel investigation/edits without losing your reasoning chain;",
			"(3) a discrete unit (explore one module, run one test file, review one PR, fix one isolated bug, write docs);",
			"(4) a long command or test suite you want to run without blocking your reasoning.",
			"Do NOT use for tasks needing tight back-and-forth with your current reasoning, or edits to files you are actively reasoning about.",
			"Delegate proactively for self-contained or parallelizable work; handle only trivial single-step or tightly interactive work inline.",
		].join("\n"),
		promptSnippet: "delegate a self-contained task to a specialized subagent (choose via subagent_type)",
		parameters: taskParams,

		async execute(_toolCallId, params: TaskParams, _signal, _onUpdate, ctx) {
			const pool = getSubagentPool(ctx.cwd);

			// Pre-flight: if the inherited provider recently exhausted its quota (the
			// parent's own turn failed with a usage/rate-limit error that did not
			// recover), skip the spawn. Subagents run on the same provider, so this
			// would only burn another failed attempt. The signal self-expires and is
			// cleared on the next successful response.
			const provider = ctx.model?.provider;
			const exhaustion = provider ? getProviderExhaustion(provider) : undefined;
			if (exhaustion) {
				const skipped = taskStore.create(params.description?.trim() || summarize(params.prompt), {
					source: "subagent",
					subagentMode: params.subagent_type,
					agent: params.subagent_type,
				});
				taskStore.update(skipped.id, { status: "failed", note: `${provider} exhausted` });
				return {
					content: [
						{
							type: "text" as const,
							text:
								`Did not dispatch subagent "${params.subagent_type}": the "${provider}" provider appears ` +
								`exhausted or rate-limited (this session just failed with: ${exhaustion.message}). ` +
								`Subagents run on the same provider, so dispatching would fail too. Wait for the quota to ` +
								`reset or switch model/provider, then retry — or complete the work directly in this session.`,
						},
					],
					details: { subagent_type: params.subagent_type, ok: false, taskId: skipped.id },
				};
			}

			// Scoped delegation: a delegating agent may be restricted to certain subagent
			// types (its `delegate: <types>` frontmatter). The root is unrestricted.
			if (!isDelegateAllowed(params.subagent_type)) {
				const allowed = delegateAllowList()?.join(", ") ?? "";
				throw new Error(
					`This agent may not delegate to "${params.subagent_type}". Allowed subagent types: ${allowed || "(none)"}.`,
				);
			}

			// Resume path: continue a previously dispatched subagent with a follow-up
			// prompt, reusing its persisted session (full prior transcript).
			const resumeId = params.resume_task_id?.trim();
			if (resumeId) {
				const summary = params.description?.trim() || summarize(params.prompt);
				const task = taskStore.create(summary, {
					source: "subagent",
					subagentMode: params.subagent_type,
					agent: params.subagent_type,
				});
				registerSubagentDispatch(params.subagent_type);
				taskStore.update(task.id, { status: "in_progress" });
				try {
					const dispatchResult = await pool.resume(resumeId, params.prompt, {
						model: ctx.model?.id,
						provider: ctx.model?.provider,
					});
					// The session lives under the original task id; keep it as the resume handle.
					return finalizeDispatchResult(dispatchResult, params.subagent_type, task.id, resumeId);
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
			const task = taskStore.create(summary, {
				source: "subagent",
				subagentMode: params.subagent_type,
				agent: params.subagent_type,
			});
			registerSubagentDispatch(params.subagent_type);

			// Always dispatch and await the subagent's full result here. Background
			// agents (def.background) are made non-blocking by the agent loop via this
			// tool's `background` flag: the loop runs this execute() detached, answers
			// the call with a placeholder, and injects the answer below as a follow-up
			// message when it resolves. Foreground agents block the turn as usual.
			taskStore.update(task.id, { status: "in_progress" });

			// Resolve model: complexity category > agent definition default > parent model
			let resolvedModel = ctx.model?.id;
			if (params.complexity) {
				// Model category from ExecuteTask's complexity parameter
				const settingsManager = SettingsManager.create(ctx.cwd, getAgentDir());
				const globalSettings = settingsManager.getGlobalSettings();
				const projectSettings = settingsManager.getProjectSettings();
				const settings = { ...globalSettings, ...projectSettings };
				const categoryModel = resolveModelReference(params.complexity, settings);
				if (categoryModel) resolvedModel = categoryModel;
			} else if (def.model && def.model !== MODEL_INHERIT) {
				// Agent definition's default model (fallback)
				resolvedModel = def.model;
			}

			// Fork agents inherit the parent's conversation via a forked session.
			const forkSessionFile = def.fork
				? resolveForkSessionFile(def, ctx.sessionManager?.getSessionFile(), ctx.cwd)
				: undefined;
			try {
				const dispatchResult = await pool.dispatch(params.prompt, {
					forceAgent: params.subagent_type,
					context: "",
					model: resolvedModel,
					provider: ctx.model?.provider,
					sessionFile: forkSessionFile,
				});
				return finalizeDispatchResult(dispatchResult, params.subagent_type, task.id, dispatchResult.task_id);
			} catch (error) {
				taskStore.update(task.id, { status: "failed" });
				throw error;
			}
		},

		renderCall(args, theme) {
			const type = args.subagent_type ?? "agent";
			const preview = summarize(args.description ?? args.prompt ?? "");
			const text =
				theme.fg("toolTitle", theme.bold("Agent ")) +
				theme.fg("accent", `[${type}]`) +
				theme.fg("dim", ` ${preview}`);
			return new Text(text, 0, 0);
		},
	});
}

/**
 * For a `fork: true` agent, fork the parent's session so the subagent inherits the
 * full parent conversation (and its prompt cache) instead of starting fresh. Returns
 * the forked session file to dispatch the child with, or undefined to fall back to a
 * fresh session (non-fork agent, no parent session, or an empty/invalid source).
 */
export function resolveForkSessionFile(
	def: Pick<AgentDefinition, "fork">,
	parentSessionPath: string | undefined,
	cwd: string,
): string | undefined {
	if (!def.fork || !parentSessionPath) return undefined;
	try {
		return SessionManager.forkFrom(parentSessionPath, cwd).getSessionFile();
	} catch {
		// Empty/invalid parent session: fall back to a fresh subagent session.
		return undefined;
	}
}

/**
 * Register the dispatched agent in the task store's roster so the task pane's
 * grouped views (subagents/teams) can draw a group header for it. Upsert keeps
 * accumulated stats across re-dispatches of the same agent type.
 */
function registerSubagentDispatch(type: string): void {
	taskStore.upsertAgent({ id: type, name: type, role: "subagent", kind: "subagent", state: "running" });
}

/**
 * Merge a child subagent's task subtree into the parent's task store, rooting
 * each top-level node under the dispatching task (`parentTaskId`). Recurses so a
 * subagent that itself delegated shows its nested work — the subtree the child
 * could not surface across the process boundary on its own. Each node is its own
 * task (no key-by-type collapse), preserving the order the child created them.
 */
function mergeChildTaskTree(nodes: readonly SubagentTaskNode[] | undefined, parentTaskId: number): void {
	if (!nodes) return;
	taskStore.batch(() => {
		for (const node of nodes) {
			const created = taskStore.create(node.title, {
				source: node.source,
				subagentMode: node.subagentMode,
				parentTaskId,
			});
			taskStore.update(created.id, { status: node.status, usage: node.usage });
			mergeChildTaskTree(node.children, created.id);
		}
	});
}

/** Extract the final answer from a finished dispatch, updating the task panel. */
function finalizeDispatchResult(
	dispatchResult: TaskResult,
	subagentType: string,
	taskStoreId: number,
	resumeHandle: string | undefined,
): { content: Array<{ type: "text"; text: string }>; details: TaskToolDetails } {
	const result = dispatchResult.result;
	const resultData = result?.result_data as SubagentResultFile | undefined;
	const usage = resultData?.usage;

	// Merge the child's own task subtree under the dispatching task so nested
	// delegation (depth >= 2) is visible in the subagents lens's task tree.
	mergeChildTaskTree(resultData?.task_tree, taskStoreId);

	// Roll the agent's per-run usage into its roster stats so the grouped views'
	// header carries the agent's own token/cost totals.
	if (usage) {
		taskStore.addAgentStats(subagentType, { input: usage.input, output: usage.output, cost: usage.cost });
	}

	if (!result || !result.ok) {
		// Signal failure by throwing: the agent loop derives a tool's error state
		// from a thrown error, not from a returned flag.
		const failNote = result?.usedInheritedModelFallback ? "inherited-model retry failed" : undefined;
		taskStore.update(taskStoreId, { status: "failed", usage, note: failNote });
		taskStore.patchAgent(subagentType, { state: "failed" });
		const reason = result?.error ?? (result?.status ? `subagent ${result.status}` : "unknown error");
		const stderr = result?.stderr?.trim();
		throw new Error(`Subagent (${subagentType}) failed: ${reason}${stderr ? `\nstderr: ${stderr.slice(-500)}` : ""}`);
	}

	// Leave the task in the store with its final status; it stays visible in the
	// task panel until the next user message arrives. Surface a ⚠ cue when the run
	// fell back to the inherited model rather than emitting a chat message.
	const fallbackNote = dispatchResult.result?.usedInheritedModelFallback ? "ran on inherited model" : undefined;
	taskStore.update(taskStoreId, { status: "done", usage, note: fallbackNote });
	// Parallel dispatches share one roster entry per agent type: stay `running`
	// while a sibling task is still live, settle to `done` otherwise.
	const siblingLive = taskStore
		.list()
		.some((t) => t.agent === subagentType && (t.status === "in_progress" || t.status === "pending"));
	taskStore.patchAgent(subagentType, { state: siblingLive ? "running" : "done" });
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
			"Pass the task_id returned by a background ExecuteTask call. While the subagent runs this reports its status; once complete it returns only the subagent's final answer.",
		].join("\n"),
		promptSnippet: "check status / collect the result of a background subagent",
		parameters: taskOutputParams,

		async execute(_toolCallId, params: TaskOutputParams, _signal, _onUpdate, ctx) {
			const pool = getSubagentPool(ctx.cwd);
			const status = pool.get_status(params.task_id);

			// If the task is still running, wait for it to finish (with a timeout)
			// instead of asking the model to poll. This eliminates the extra LLM
			// round-trip that polling would require.
			if (status === "running" || status === "queued") {
				const TASK_OUTPUT_TIMEOUT_MS = 120_000;
				const result = await Promise.race([
					pool.wait_for_completion(params.task_id),
					new Promise<undefined>((resolve) => setTimeout(() => resolve(undefined), TASK_OUTPUT_TIMEOUT_MS)),
				]);
				if (!result) {
					return {
						content: [
							{
								type: "text" as const,
								text: `Subagent task "${params.task_id}" is still ${pool.get_status(params.task_id)}. Timed out after ${TASK_OUTPUT_TIMEOUT_MS / 1000}s.`,
							},
						],
						details: { task_id: params.task_id, status: pool.get_status(params.task_id), ok: true },
					};
				}
				return formatTaskOutputResult(result, params.task_id);
			}

			if (status === "unknown") {
				return {
					content: [
						{
							type: "text" as const,
							text: `No result available for task "${params.task_id}" (status: unknown). It may not exist or its result was already collected.`,
						},
					],
					details: { task_id: params.task_id, status, ok: false },
				};
			}

			const result = pool.collect(params.task_id);
			if (!result) {
				throw new Error(
					`No result available for task "${params.task_id}" (status: ${status}). It may not exist or its result was already collected.`,
				);
			}
			return formatTaskOutputResult(result, params.task_id);
		},

		renderCall(args, theme) {
			const text = theme.fg("toolTitle", theme.bold("TaskOutput ")) + theme.fg("dim", String(args.task_id ?? ""));
			return new Text(text, 0, 0);
		},
	});
}

/** Format a completed subagent result for TaskOutput. */
function formatTaskOutputResult(
	result: SubagentResult,
	taskId: string,
): { content: Array<{ type: "text"; text: string }>; details: TaskOutputDetails } {
	if (!result.ok) {
		const reason = result.error ?? (result.status ? `subagent ${result.status}` : "unknown error");
		throw new Error(`Background subagent "${taskId}" failed: ${reason}`);
	}
	const resultData = result.result_data as SubagentResultFile | undefined;
	const answer = resultData?.summary || "(subagent returned no output)";
	return {
		content: [{ type: "text" as const, text: answer }],
		details: { task_id: taskId, status: result.status ?? "complete", ok: true },
	};
}
