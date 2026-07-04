/**
 * Task tool: delegate a focused task to a specialized subagent.
 *
 * Mirrors the Claude Code `Task` tool. The parent agent decides *when* to
 * delegate based on each agent's `description` (there is no deterministic gate)
 * and selects *which* agent via `subagent_type`. The chosen agent runs in a
 * fresh, isolated child process (SubagentPool) and only its final answer is
 * returned to the parent.
 *
 * Enabled by default (the `enableSubagent` setting defaults to true); disable
 * with `enableSubagent: false`. The `--enable-subagents` flag still force-enables
 * it. Nesting is bounded by the tree-wide depth cap (maxSubagentDepth, default 2:
 * a spawned subagent may itself delegate one more level, and depth-2 grandchildren
 * cannot). See buildSessionOptions in main.ts.
 */

import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import { agentColorFor } from "../../modes/interactive/theme/theme.js";
import { type AgentDefinition, TASK_TOOL_NAME } from "../agent-frontmatter.js";
import { loadAgentRegistry } from "../agent-registry.js";
import type { ToolDefinition } from "../extensions/types.js";
import { defineTool } from "../extensions/types.js";
import { formatDurationSecs } from "../format-duration.js";
import { getProviderExhaustion } from "../provider-health.js";
import { SessionManager } from "../session-manager.js";
import { delegateAllowList, isDelegateAllowed } from "../subagent-depth.js";
import { type InboxRecord, subagentInbox } from "../subagent-inbox.js";
import type { SubagentResult, TaskResult } from "../subagent-pool.js";
import { getSubagentPool } from "../subagent-pool-instance.js";
import type { SubagentResultFile, SubagentTaskNode } from "../subagent-result.js";
import { taskStore } from "../task-store.js";
import { type WarmRunResult, WarmWorkerError } from "../warm-subagent-pool.js";
import { getWarmSubagentPool, warmSubagentsEnabled } from "../warm-subagent-pool-instance.js";

// Re-exported from its home in agent-registry (where formatAgentsForPrompt uses
// it to render the roster) so existing importers keep working without creating a
// tools -> registry -> tools cycle.
export { summarizeAgentDescription } from "../agent-registry.js";

/** System prompt appendix for the main session when the Task tool is enabled.
 *  Instructs the parent agent on when and how to delegate effectively. The
 *  available agents themselves are listed once, authoritatively, in the
 *  `<available_agents>` block the system prompt emits whenever the Task tool is
 *  active (see agent-session `_rebuildSystemPrompt`); this appendix references
 *  that list rather than re-rendering the roster and paying for it twice. */
/**
 * Build the main-session subagent instructions appended to the system prompt.
 *
 * The detailed background/barrier guidance (the three heaviest bullets) is only
 * emitted when the project actually has background-capable agents; otherwise a
 * single concise line covers the per-call `background: true` escape hatch. This
 * keeps the always-on cost down for the common case where nothing runs in the
 * background. `cwd` is used only to detect those agents.
 */
export function buildTaskMainPrompt(cwd: string = process.cwd()): string {
	const hasBackgroundAgents = collectBackgroundAgentNames(cwd).size > 0;

	const backgroundGuidance = hasBackgroundAgents
		? `- Background agents run non-blocking (or force per call with \`background: true\`): you get a short "explore#1 finished" notification and pull the full result with \`TaskOutput\` (e.g. \`TaskOutput("explore#1")\`); \`TaskOutput(list: true)\` shows what's running.
- After dispatching, don't idle — keep doing independent work (read/edit unrelated files, draft, dispatch more); barrier with \`TaskOutput(wait: true)\` (a named task, or all outstanding when no id) only when you genuinely can't proceed.`
		: `- You can force any task to run in the background with \`background: true\` (non-blocking): keep working, then pull its result with \`TaskOutput\` (e.g. \`TaskOutput("explore#1")\`) or block on it with \`TaskOutput(wait: true)\`.`;

	return `You have access to the **Task** tool. Use it to delegate self-contained tasks to specialized subagents that run in their own isolated context and return only their final answer. Pick an agent by name from the <available_agents> list in this prompt and pass it as \`subagent_type\`.

When to delegate:
1. The work is self-contained and you only need the final result, not intermediate steps.
2. You want to investigate or edit something in parallel without losing your current context or reasoning chain.
3. The task is a discrete unit (explore one module, run one test file, review one PR, fix one isolated bug).
4. You need to run a long command or test suite and wait for its output without blocking your own reasoning.

Model tier (optional \`complexity\`): set \`fast\` for quick reads/lookups, \`standard\` for multi-file edits, \`capable\` for deep architecture work. It maps to a model from \`settings.modelCategories\`. Omit it to use the agent's default; an agent that pins its own model ignores \`complexity\`.

Guidelines:
- Choose the agent whose description best matches the task.
- Make every task specific and self-contained. The subagent cannot see this conversation; pass all necessary context (files, constraints, prior findings) in \`prompt\`.
- Do NOT delegate tasks that require tight back-and-forth with your current reasoning, or edits to files you are actively reasoning about.
- The subagent returns ONLY its final answer. Its intermediate reasoning, tool calls, and output are hidden from you.
- Delegate proactively when work is self-contained or parallelizable: multi-step investigation, read-only exploration (use \`explore\`), research before changes (use \`plan\`), drafting a standalone file/section, or running a long command/test suite. Dispatch independent subtasks in the same turn. Handle only trivial single-step edits or tightly interactive back-and-forth inline.
${backgroundGuidance}
- When working through a TodoWrite plan, mark the plan item in_progress BEFORE dispatching subagents for it: each dispatch is attributed to the current in_progress item in the user's task panel, so dispatching first (or with several items in_progress) leaves the run unattributed.
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
	complexity: Type.Optional(
		Type.Union([Type.Literal("fast"), Type.Literal("standard"), Type.Literal("capable")], {
			description:
				"Model tier for this dispatch: fast (quick reads/lookups), standard (multi-file edits), capable (deep architecture). Maps to settings.modelCategories. Ignored if the chosen agent pins its own model; omit to use the agent's default.",
		}),
	),
	background: Type.Optional(
		Type.Boolean({
			description:
				"Set true to run non-blocking: you get a short notification when it finishes and pull the full result with TaskOutput; set false to wait and get the answer inline. Defaults to the agent's own background setting.",
		}),
	),
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
	/** The handle queried, when a specific task was named. */
	task_id?: string;
	/** running | done | collected | failed | stalled | timeout | cancelled | list | empty | unknown */
	status: string;
	ok: boolean;
	/** Number of subagents still running, included on roster/list responses. */
	outstanding?: number;
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

/** Create the Task tool definition. Registered as a customTool when enabled. */
export function createTaskToolDefinition(cwd: string = process.cwd()): ToolDefinition {
	// Agents whose definitions opt into background execution. The agent loop reads
	// the tool's `background` flag per call and, for these, runs the dispatch
	// detached: the parent keeps reasoning and the subagent's answer is injected as
	// a follow-up message when it finishes (no polling needed). A per-call
	// `background` argument overrides the agent's default in either direction.
	const backgroundAgents = collectBackgroundAgentNames(cwd);
	return defineTool<typeof taskParams, TaskToolDetails>({
		name: TASK_TOOL_NAME,
		label: TASK_TOOL_NAME,
		background: (toolCall) => {
			const override = toolCall.arguments?.background;
			if (typeof override === "boolean") return override;
			return backgroundAgents.has(String(toolCall.arguments?.subagent_type ?? ""));
		},
		// Kept to mechanics only: the when-to-use / when-not-to guidance lives once in
		// the system-prompt block (buildTaskMainPrompt), the available agents are
		// listed there too, and the `complexity`/`background`/`resume_task_id`
		// semantics live in their parameter descriptions. Repeating any of that here
		// would re-spend those tokens on every turn.
		description:
			"Delegate a focused task to a specialized subagent that runs in a fresh, isolated context (it cannot see this conversation). Choose one of the available agents (listed in the system prompt) via `subagent_type` and pass everything it needs via `prompt`; the subagent returns only its final answer.",
		promptSnippet: "delegate a self-contained task to a specialized subagent (choose via subagent_type)",
		parameters: taskParams,

		async execute(_toolCallId, params: TaskParams, signal, _onUpdate, ctx) {
			const pool = getSubagentPool(ctx.cwd);

			// User-initiated cancel (Esc/abort): kill the dispatched run's whole
			// process tree and let the dispatch settle with status "cancelled" —
			// distinct from "failed" everywhere it surfaces (panel, inbox, result).
			// Cold dispatches only: a warm worker has no cancel surface (it is
			// reused), so warm runs still complete server-side and are discarded.
			const cancelOnAbort = async (poolRunId: string, run: () => Promise<TaskResult>): Promise<TaskResult> => {
				if (!signal) return run();
				const onAbort = () => {
					pool.cancel?.(poolRunId);
				};
				if (signal.aborted) onAbort();
				signal.addEventListener("abort", onAbort, { once: true });
				try {
					return await run();
				} finally {
					signal.removeEventListener("abort", onAbort);
				}
			};

			// Pre-flight: if the inherited provider recently exhausted its quota (the
			// parent's own turn failed with a usage/rate-limit error that did not
			// recover), skip the spawn. Subagents run on the same provider, so this
			// would only burn another failed attempt. The signal self-expires and is
			// cleared on the next successful response.
			const provider = ctx.model?.provider;
			const exhaustion = provider ? getProviderExhaustion(provider) : undefined;
			if (exhaustion) {
				const skippedRunId = newDispatchTaskId();
				const skipped = taskStore.create(params.description?.trim() || summarize(params.prompt), {
					source: "subagent",
					subagentMode: params.subagent_type,
					agent: skippedRunId,
					linkedTaskId: linkedTodoId(),
				});
				registerSubagentDispatch(skippedRunId, subagentInbox.nextLabel(params.subagent_type));
				taskStore.update(skipped.id, { status: "failed", note: `${provider} exhausted` });
				taskStore.patchAgent(skippedRunId, { state: "failed" });
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
				const runId = newDispatchTaskId();
				const label = subagentInbox.nextLabel(params.subagent_type);
				const task = taskStore.create(summary, {
					source: "subagent",
					subagentMode: params.subagent_type,
					agent: runId,
					linkedTaskId: linkedTodoId(),
				});
				registerSubagentDispatch(runId, label);
				taskStore.update(task.id, { status: "in_progress" });
				try {
					const dispatchResult = await cancelOnAbort(runId, () =>
						pool.resume(resumeId, params.prompt, {
							model: ctx.model?.id,
							provider: ctx.model?.provider,
							taskId: runId,
						}),
					);
					// The session lives under the original task id; keep it as the resume handle.
					return finalizeDispatchResult(dispatchResult, params.subagent_type, runId, task.id, resumeId);
				} catch (error) {
					markDispatchFailed(task.id, runId);
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
			// One roster row per dispatch, keyed by the (pre-allocated) pool task id.
			// Keying by agent TYPE made concurrent same-type runs share a single row:
			// their activity, state, and stats collided last-writer-wins. The inbox
			// already keys by task id; this aligns the panel to the same model. The
			// pool's task_progress events carry the task id, so patching by this id
			// routes live activity to the right row.
			const poolTaskId = newDispatchTaskId();
			const label = subagentInbox.nextLabel(params.subagent_type);
			const task = taskStore.create(summary, {
				source: "subagent",
				subagentMode: params.subagent_type,
				agent: poolTaskId,
				// Tie this run to the plan item it executes (the single in_progress
				// TodoWrite task, when unambiguous) so the panel can nest it there.
				linkedTaskId: linkedTodoId(),
			});
			registerSubagentDispatch(poolTaskId, label);
			taskStore.update(task.id, { status: "in_progress" });
			// Fork agents inherit the parent's conversation via a forked session.
			const forkSessionFile = def.fork
				? resolveForkSessionFile(def, ctx.sessionManager?.getSessionFile(), ctx.cwd)
				: undefined;

			// `complexity` is passed as the model: the pool's spawn() already lets a
			// non-`inherit` agent model win, then resolves a category string (fast/
			// standard/capable) via settings.modelCategories. So a pinned-model agent
			// ignores complexity, and an `inherit` agent picks up the requested tier —
			// no settings lookup needed here.
			const dispatchModel = params.complexity ?? ctx.model?.id;

			// Whether this call runs detached. The agent loop reads the tool's
			// `background` flag (the same predicate) to run execute() detached; we
			// recompute it here to choose the notify-and-pull return shape.
			const isBackground = params.background ?? backgroundAgents.has(params.subagent_type);

			if (isBackground) {
				// Notify-and-pull: register the dispatch in the inbox under the
				// pre-allocated id, await it, retain the body in the inbox, and return a
				// compact notification (not the body). The model pulls it with TaskOutput.
				subagentInbox.observe(pool);
				subagentInbox.start(poolTaskId, label, params.subagent_type);

				// Warm path (opt-in): run on a reused RPC worker, retain the body in the
				// inbox, and return the same notify-and-pull shape as the cold path. An
				// infra failure falls through to the cold dispatch below.
				if (warmSubagentsEnabled() && !forkSessionFile) {
					const warm = getWarmSubagentPool(ctx.cwd);
					if (warm.isPoolable(params.subagent_type)) {
						try {
							const warmResult = await warm.dispatch(
								params.prompt,
								{
									agentType: params.subagent_type,
									cwd: ctx.cwd,
									model: dispatchModel,
									provider: ctx.model?.provider,
								},
								(activity) => taskStore.patchAgent(poolTaskId, { activity }),
							);
							const dispatchResult = warmResultToTaskResult(warmResult, params.subagent_type, task.id);
							subagentInbox.finish(poolTaskId, dispatchResult);
							return finalizeDispatchResult(
								dispatchResult,
								params.subagent_type,
								poolTaskId,
								task.id,
								poolTaskId,
								{
									taskId: poolTaskId,
									label,
								},
							);
						} catch (error) {
							if (!(error instanceof WarmWorkerError)) throw error;
							console.error(`[WARM] ${params.subagent_type} fell back to cold spawn: ${error.message}`);
						}
					}
				}

				try {
					const dispatchResult = await cancelOnAbort(poolTaskId, () =>
						pool.dispatch(params.prompt, {
							forceAgent: params.subagent_type,
							context: "",
							model: dispatchModel,
							provider: ctx.model?.provider,
							sessionFile: forkSessionFile,
							taskId: poolTaskId,
						}),
					);
					subagentInbox.finish(poolTaskId, dispatchResult);
					return finalizeDispatchResult(dispatchResult, params.subagent_type, poolTaskId, task.id, poolTaskId, {
						taskId: poolTaskId,
						label,
					});
				} catch (error) {
					const reason = error instanceof Error ? error.message : String(error);
					markDispatchFailed(task.id, poolTaskId);
					subagentInbox.fail(poolTaskId, reason);
					// A background dispatch reports failure as a compact notification, not a
					// thrown tool error — the call was already answered by a placeholder.
					return {
						content: [{ type: "text" as const, text: `${label} failed ✗ — ${reason}` }],
						details: {
							subagent_type: params.subagent_type,
							ok: false,
							error: reason,
							taskId: task.id,
							poolTaskId,
							background: true,
						},
					};
				}
			}

			// Warm path (opt-in): for an eligible foreground dispatch, run on a reused
			// RPC worker to skip the cold-boot. Fork agents (forkSessionFile set) and
			// non-poolable types are excluded. Any infra failure falls through to the
			// cold pool below, so enabling this can only change latency, never whether
			// the task can run.
			if (warmSubagentsEnabled() && !forkSessionFile) {
				const warm = getWarmSubagentPool(ctx.cwd);
				if (warm.isPoolable(params.subagent_type)) {
					try {
						const warmResult = await warm.dispatch(
							params.prompt,
							{
								agentType: params.subagent_type,
								cwd: ctx.cwd,
								model: dispatchModel,
								provider: ctx.model?.provider,
							},
							// Mirror the cold pool's live progress on the task panel roster so a
							// warm dispatch reads as busy (⋯ grep), not stuck.
							(activity) => taskStore.patchAgent(poolTaskId, { activity }),
						);
						const dispatchResult = warmResultToTaskResult(warmResult, params.subagent_type, task.id);
						return finalizeDispatchResult(dispatchResult, params.subagent_type, poolTaskId, task.id, undefined);
					} catch (error) {
						// A genuine infra failure (worker crash/timeout) retries cold; any other
						// error is a real dispatch failure and propagates.
						if (!(error instanceof WarmWorkerError)) {
							taskStore.update(task.id, { status: "failed" });
							taskStore.patchAgent(poolTaskId, { state: "failed", activity: "" });
							throw error;
						}
						console.error(`[WARM] ${params.subagent_type} fell back to cold spawn: ${error.message}`);
					}
				}
			}

			// Foreground: block the turn and return the subagent's full answer inline.
			try {
				const dispatchResult = await cancelOnAbort(poolTaskId, () =>
					pool.dispatch(params.prompt, {
						forceAgent: params.subagent_type,
						context: "",
						model: dispatchModel,
						provider: ctx.model?.provider,
						sessionFile: forkSessionFile,
						taskId: poolTaskId,
					}),
				);
				return finalizeDispatchResult(
					dispatchResult,
					params.subagent_type,
					poolTaskId,
					task.id,
					dispatchResult.task_id,
				);
			} catch (error) {
				markDispatchFailed(task.id, poolTaskId);
				throw error;
			}
		},

		renderCall(args, theme) {
			const type = args.subagent_type ?? "agent";
			// The [type] tag carries the agent's identity color — the same hue this
			// agent has in the task panel and TaskOutput — so a transcript full of
			// concurrent dispatches is scannable by color.
			const text = theme.fg("toolTitle", theme.bold("Agent ")) + theme.fg(agentColorFor(type), `[${type}]`);
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

/** Pre-allocated pool task id for a dispatch (matches the pool's own format). */
function newDispatchTaskId(): string {
	return `dispatch-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

/**
 * The TodoWrite plan item this dispatch is (most plausibly) working on: the
 * single in_progress main-agent task. TodoWrite discipline keeps exactly one
 * item in_progress, so when that holds the link is unambiguous; with zero or
 * several in_progress items no link is recorded rather than guessing. Used to
 * nest the run under its plan item in the task panel's flat lens.
 */
function linkedTodoId(): number | undefined {
	const inProgress = taskStore
		.list()
		.filter(
			(t) =>
				t.source === undefined &&
				t.agent === undefined &&
				t.parentTaskId === undefined &&
				t.status === "in_progress",
		);
	return inProgress.length === 1 ? inProgress[0]?.id : undefined;
}

/**
 * Register this dispatch in the task store's roster, keyed by its run id (the
 * pool task id) so concurrent runs of the same agent type each get their own
 * row, state, activity, and stats. The friendly label ("explore#1") names the
 * row the same way the inbox names background tasks.
 */
function registerSubagentDispatch(runId: string, label: string): void {
	taskStore.upsertAgent({ id: runId, name: label, role: "subagent", kind: "subagent", state: "running" });
}

/**
 * Catch-path bookkeeping: mark the dispatch failed unless it already settled.
 * finalizeDispatchResult records a terminal status (failed/cancelled) *before*
 * throwing, so a blanket "failed" here would clobber a cancellation with a
 * failure the moment the thrown error passed through.
 */
function markDispatchFailed(taskStoreId: number, runId: string): void {
	const current = taskStore.list().find((t) => t.id === taskStoreId);
	if (current && current.status !== "pending" && current.status !== "in_progress") return;
	taskStore.update(taskStoreId, { status: "failed" });
	taskStore.patchAgent(runId, { state: "failed", activity: "" });
}

/** Names of agents configured to run in the background (non-blocking). */
function collectBackgroundAgentNames(cwd: string): Set<string> {
	const names = new Set<string>();
	for (const agent of loadAgentRegistry({ cwd }).list()) {
		if (agent.background) names.add(agent.name);
	}
	return names;
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

/**
 * Adapt a warm-worker run into the TaskResult shape finalizeDispatchResult
 * consumes, so the warm and cold paths share one finish/render path. A warm run
 * returns its answer inline (no result.json), so we synthesize an equivalent
 * SubagentResult with the answer as the summary and the pulled usage. The warm
 * path is clean (no persisted session), so there is no task_tree to merge and no
 * resume handle.
 */
function warmResultToTaskResult(warm: WarmRunResult, agentType: string, taskStoreId: number): TaskResult {
	const resultData: SubagentResultFile = {
		summary: warm.summary || "(subagent returned no output)",
		files_changed: [],
		confidence: warm.ok ? 1 : 0,
		status: warm.status,
		usage: warm.usage,
	};
	const result: SubagentResult = {
		task_id: String(taskStoreId),
		ok: warm.ok,
		stdout: "",
		stderr: "",
		exit_code: warm.ok ? 0 : 1,
		status: warm.status,
		error: warm.error,
		result_data: resultData as unknown as Record<string, unknown>,
	};
	return { handled_inline: false, agent_type: agentType, result };
}

/**
 * Update the task panel from a finished dispatch and shape the tool result.
 *
 * Foreground calls return the subagent's full answer inline and signal a hard
 * failure by throwing (the agent loop derives a tool's error state from a thrown
 * error). A background call passes `background`: the body already lives in the
 * inbox, so it returns a compact, self-contained notification (success or
 * failure) and never throws — the call was already answered by a placeholder.
 */
function finalizeDispatchResult(
	dispatchResult: TaskResult,
	subagentType: string,
	runAgentId: string,
	taskStoreId: number,
	resumeHandle: string | undefined,
	background?: { taskId: string; label: string },
): { content: Array<{ type: "text"; text: string }>; details: TaskToolDetails } {
	const result = dispatchResult.result;
	const resultData = result?.result_data as SubagentResultFile | undefined;
	const usage = resultData?.usage;

	// Merge the child's own task subtree under the dispatching task so nested
	// delegation (depth >= 2) is visible in the subagents lens's task tree.
	mergeChildTaskTree(resultData?.task_tree, taskStoreId);

	// Roll this run's usage into its own roster row (rows are per-dispatch).
	if (usage) {
		taskStore.addAgentStats(runAgentId, { input: usage.input, output: usage.output, cost: usage.cost });
	}

	if (!result || !result.ok) {
		// A user-initiated cancel is not a failure: it gets its own terminal
		// status so the panel shows ⊘ cancelled (dim) instead of ✗ failed (red).
		const cancelled = result?.status === "cancelled";
		const failNote = result?.usedInheritedModelFallback ? "inherited-model retry failed" : undefined;
		taskStore.update(taskStoreId, { status: cancelled ? "cancelled" : "failed", usage, note: failNote });
		taskStore.patchAgent(runAgentId, { state: cancelled ? "cancelled" : "failed", activity: "" });
		const reason = result?.error ?? (result?.status ? `subagent ${result.status}` : "unknown error");
		if (background) {
			const verdict = cancelled ? "cancelled ⊘" : "failed ✗";
			return {
				content: [{ type: "text", text: `${background.label} ${verdict} — ${reason}` }],
				details: {
					subagent_type: subagentType,
					ok: false,
					error: reason,
					taskId: taskStoreId,
					poolTaskId: background.taskId,
					background: true,
				},
			};
		}
		if (cancelled) {
			throw new Error(`Subagent (${subagentType}) cancelled by user.`);
		}
		const stderr = result?.stderr?.trim();
		throw new Error(`Subagent (${subagentType}) failed: ${reason}${stderr ? `\nstderr: ${stderr.slice(-500)}` : ""}`);
	}

	// Leave the task in the store with its final status; it stays visible in the
	// task panel until the next user message arrives. Surface a ⚠ cue when the run
	// fell back to the inherited model rather than emitting a chat message — and
	// clear any stale note otherwise (note is clearable via an explicit undefined).
	const fallbackNote = dispatchResult.result?.usedInheritedModelFallback ? "ran on inherited model" : undefined;
	taskStore.update(taskStoreId, { status: "done", usage, note: fallbackNote });
	// Rows are per-run, so this run settles to done regardless of siblings.
	taskStore.patchAgent(runAgentId, { state: "done", activity: "" });
	let answer = resultData?.summary || "(subagent returned no output)";
	// Partial results are resumable; surface the handle so the parent can continue.
	if (result.status === "partial" && resumeHandle) {
		answer += `\n\n[Partial result. To continue this subagent, call Task again with resume_task_id="${resumeHandle}".]`;
	}

	if (background) {
		// Compact notification: the body is retained in the inbox; the model pulls it
		// with TaskOutput. Keeps a wide swarm from flooding the parent's context.
		const partial = result.status === "partial" ? " (partial — resume to continue)" : "";
		const outstanding = subagentInbox.outstanding().length;
		const tail = outstanding > 0 ? ` ${outstanding} still running.` : "";
		const text =
			`${background.label} finished ✓${partial} — ${summarize(answer)}.${tail}\n` +
			`Read the full result with TaskOutput("${background.label}").`;
		return {
			content: [{ type: "text", text }],
			details: {
				subagent_type: subagentType,
				ok: true,
				taskId: taskStoreId,
				poolTaskId: background.taskId,
				background: true,
			},
		};
	}

	return {
		content: [{ type: "text", text: answer }],
		details: { subagent_type: subagentType, ok: true, taskId: taskStoreId, poolTaskId: resumeHandle },
	};
}

const taskOutputParams = Type.Object({
	task_id: Type.Optional(
		Type.String({
			description:
				'Handle of a background subagent — its task_id or friendly label (e.g. "explore#1") from a Task notification. Omit (or set list:true) to see every background task.',
		}),
	),
	list: Type.Optional(
		Type.Boolean({
			description:
				"List all background subagents with their status (running/done/failed/cancelled) and current activity. No result bodies are returned.",
		}),
	),
	wait: Type.Optional(
		Type.Boolean({
			description:
				"Block until the named task finishes — or, with no task_id, until all outstanding subagents finish (a swarm barrier) — before returning. Bounded by timeout_ms.",
		}),
	),
	timeout_ms: Type.Optional(
		Type.Number({ description: "Maximum time to block in wait mode, in milliseconds (default 120000)." }),
	),
});

type TaskOutputParams = Static<typeof taskOutputParams>;

const TASK_OUTPUT_DEFAULT_TIMEOUT_MS = 120_000;

/**
 * Elapsed time a record has run (so far, or until it settled), in the same
 * format the task panel uses so the two surfaces always agree.
 */
function recordElapsed(rec: InboxRecord): string {
	const end = rec.endedAt ?? Date.now();
	return formatDurationSecs((end - rec.startedAt) / 1000);
}

/** A compact roster of every known background subagent — status + activity, no bodies. */
function formatTaskRoster(): { content: Array<{ type: "text"; text: string }>; details: TaskOutputDetails } {
	const all = subagentInbox.list();
	const outstanding = subagentInbox.outstanding().length;
	if (all.length === 0) {
		return {
			content: [{ type: "text", text: "No background subagents have been dispatched." }],
			details: { status: "empty", ok: true, outstanding: 0 },
		};
	}
	const lines = all.map((r) => {
		const when = recordElapsed(r);
		switch (r.lifecycle) {
			case "running":
				return `- ${r.label}  running  ${when}${r.lastActivity ? `  · ${r.lastActivity}` : ""}`;
			case "done":
				return `- ${r.label}  done (uncollected)  ${when} — ${r.summaryLine ?? ""}`;
			case "collected":
				return `- ${r.label}  collected  ${when} — ${r.summaryLine ?? ""}`;
			case "cancelled":
				return `- ${r.label}  cancelled ⊘  — ${r.error ?? "cancelled by user"}`;
			default:
				return `- ${r.label}  ${r.lifecycle} ✗  — ${r.error ?? "unknown error"}`;
		}
	});
	const header = `${all.length} background subagent${all.length === 1 ? "" : "s"} (${outstanding} running):`;
	const hint = all.some((r) => r.lifecycle === "done") ? '\nRead a finished one with TaskOutput("<label>").' : "";
	return {
		content: [{ type: "text", text: `${header}\n${lines.join("\n")}${hint}` }],
		details: { status: "list", ok: true, outstanding },
	};
}

/**
 * TaskOutput tool: check on background subagents and pull their results.
 *
 * Background `Task` calls don't push their body into the conversation — they
 * leave it in the inbox and post a compact notification. TaskOutput is how the
 * model pulls a body, checks liveness, or waits. It never throws on a valid
 * handle (an error tool result would only confuse the loop): it reports status
 * instead. Modes: `list` (roster), a `task_id` to read/check one, and `wait` to
 * block until one task — or all outstanding tasks — finish.
 */
export function createTaskOutputToolDefinition(): ToolDefinition {
	return defineTool<typeof taskOutputParams, TaskOutputDetails>({
		name: "TaskOutput",
		label: "TaskOutput",
		description: [
			"Check on background subagents dispatched via Task, and pull their results.",
			'Pass a task_id/label (e.g. "explore#1") to read a finished subagent\'s full result, or to see its status while it runs.',
			"Set list:true (or omit task_id) to list every background subagent with its status and current activity.",
			"Set wait:true to block until that task finishes — or, with no task_id, until all outstanding subagents finish (a swarm barrier).",
			"It never errors on a valid handle: a running task reports status, a finished one returns its result, an already-read one says so.",
		].join("\n"),
		promptSnippet: "check status / list / collect the results of background subagents",
		parameters: taskOutputParams,

		async execute(_toolCallId, params: TaskOutputParams, _signal, _onUpdate, ctx) {
			// Touch the pool so the inbox is wired to its progress stream for activity.
			subagentInbox.observe(getSubagentPool(ctx.cwd));
			const handle = params.task_id?.trim();

			// Barrier: wait for the target (or all outstanding) to settle first.
			if (params.wait) {
				const timeout = params.timeout_ms ?? TASK_OUTPUT_DEFAULT_TIMEOUT_MS;
				if (handle) await subagentInbox.waitFor(handle, timeout);
				else await subagentInbox.waitForAll(timeout);
			}

			// Roster when asked, or when no specific task was named.
			if (params.list || !handle) {
				return formatTaskRoster();
			}

			const rec = subagentInbox.get(handle);
			if (!rec) {
				return {
					content: [
						{
							type: "text" as const,
							text: `No background task "${handle}". Call TaskOutput with list:true to see active tasks.`,
						},
					],
					details: { task_id: handle, status: "unknown", ok: false },
				};
			}

			if (rec.lifecycle === "running") {
				const activity = rec.lastActivity ? ` (currently: ${rec.lastActivity})` : "";
				return {
					content: [
						{
							type: "text" as const,
							text: `${rec.label} is still running — ${recordElapsed(rec)} elapsed${activity}. Call TaskOutput again, or with wait:true to block until it finishes.`,
						},
					],
					details: { task_id: handle, status: "running", ok: true },
				};
			}

			if (rec.lifecycle === "done") {
				const collected = subagentInbox.collect(handle);
				const body = collected?.body ?? rec.summaryLine ?? "(subagent returned no output)";
				return {
					content: [{ type: "text" as const, text: body }],
					details: { task_id: handle, status: "done", ok: true },
				};
			}

			if (rec.lifecycle === "collected") {
				return {
					content: [
						{
							type: "text" as const,
							text: `${rec.label} was already delivered — ${rec.summaryLine ?? "(no summary kept)"}.`,
						},
					],
					details: { task_id: handle, status: "collected", ok: true },
				};
			}

			// failed / stalled / timeout / cancelled
			const glyph = rec.lifecycle === "cancelled" ? "⊘" : "✗";
			return {
				content: [
					{
						type: "text" as const,
						text: `${rec.label} ${rec.lifecycle} ${glyph} — ${rec.error ?? "unknown error"}.`,
					},
				],
				details: { task_id: handle, status: rec.lifecycle, ok: false },
			};
		},

		renderCall(args, theme) {
			const target = args.list ? "list" : String(args.task_id ?? "");
			// A friendly label ("explore#1") gets its agent's identity color so the
			// pull visually pairs with the dispatch that produced it; raw ids and
			// "list" stay dim.
			const hashIdx = target.indexOf("#");
			const styledTarget =
				hashIdx > 0 ? theme.fg(agentColorFor(target.slice(0, hashIdx)), target) : theme.fg("dim", target);
			const text =
				theme.fg("toolTitle", theme.bold("TaskOutput ")) +
				styledTarget +
				(args.wait ? theme.fg("dim", " (wait)") : "");
			return new Text(text, 0, 0);
		},

		renderResult(result, _options, theme) {
			// Display-only colorization of the roster/status lines (the text sent to
			// the model stays plain). Lines that aren't roster rows — collected
			// bodies, status sentences — render exactly like the default fallback.
			const text = result.content
				.map((c) => (c.type === "text" ? c.text : ""))
				.filter(Boolean)
				.join("\n");
			if (!text) return new Text("", 0, 0);
			const rosterLine =
				/^- (\S+)\s{2}(running|done \(uncollected\)|collected|failed|stalled|timeout|cancelled)(.*)$/;
			const styled = text
				.split("\n")
				.map((line) => {
					const match = rosterLine.exec(line);
					if (!match) return theme.fg("toolOutput", line);
					const [, label, status, rest] = match as unknown as [string, string, string, string];
					const hashIdx = label.indexOf("#");
					const labelColor = hashIdx > 0 ? agentColorFor(label.slice(0, hashIdx)) : "accent";
					const statusColor =
						status === "running"
							? "warning"
							: status.startsWith("done")
								? "success"
								: status === "collected" || status === "cancelled"
									? "muted"
									: "error";
					return `- ${theme.fg(labelColor, label)}  ${theme.fg(statusColor, status)}${theme.fg("dim", rest)}`;
				})
				.join("\n");
			return new Text(styled, 0, 0);
		},
	});
}
