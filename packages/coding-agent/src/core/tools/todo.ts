/**
 * TodoWrite tool: let the main agent maintain a visible todo list for the
 * current task.
 *
 * hoocode already has all the infrastructure this needs — the task store models
 * `{ title, status }` items with a per-turn lifecycle, and the TUI task panel
 * renders them. The only missing piece was a tool the model can call; this is
 * that thin adapter over `taskStore`.
 *
 * Semantics mirror Claude Code's TodoWrite: each call sends the FULL list and
 * REPLACES the previous one. Because the store is incremental (numeric ids), we
 * reconcile the incoming list against the existing main-agent tasks by position:
 * update items that are still there, create new ones, and drop the tail that was
 * removed. Reconciling (rather than clear-and-recreate) keeps ids stable so the
 * panel does not flicker and in-progress rows stay put.
 *
 * It is an optional, opt-in tool (enabled via the `enableTodoWrite` setting) and
 * is never registered inside a spawned subagent, so a subagent's todos cannot
 * leak into the parent's "main" task group.
 */

import { type Static, Type } from "typebox";
import { TODO_WRITE_TOOL_NAME } from "../agent-frontmatter.js";
import { defineTool, type ToolDefinition } from "../extensions/types.js";
import { type Task, type TaskStatus, taskStore } from "../task-store.js";

const todoStatusSchema = Type.Union([Type.Literal("pending"), Type.Literal("in_progress"), Type.Literal("completed")], {
	description: "pending = not started, in_progress = actively being worked on, completed = finished.",
});

const todoItemSchema = Type.Object(
	{
		content: Type.String({
			description: "The task, in imperative form (e.g. 'Add tests for the parser').",
		}),
		status: todoStatusSchema,
		activeForm: Type.Optional(
			Type.String({
				description:
					"Optional present-tense form shown while the item is in_progress (e.g. 'Adding tests for the parser').",
			}),
		),
		complexity: Type.Optional(
			Type.Union([Type.Literal("fast"), Type.Literal("standard"), Type.Literal("capable")], {
				description:
					"Model category for ExecuteTask dispatch: fast (quick reads), standard (multi-file edits), capable (deep architecture). Optional.",
			}),
		),
	},
	{ additionalProperties: false },
);

const todoWriteParams = Type.Object(
	{
		todos: Type.Array(todoItemSchema, {
			description:
				"The complete todo list. This REPLACES the previous list on every call, so always send every item with its current status — omitting an item removes it.",
		}),
	},
	{ additionalProperties: false },
);

type TodoWriteParams = Static<typeof todoWriteParams>;
type IncomingStatus = TodoWriteParams["todos"][number]["status"];

export interface TodoWriteDetails {
	total: number;
	pending: number;
	inProgress: number;
	completed: number;
}

/** Map the model-facing status vocabulary onto the task store's. */
function toTaskStatus(status: IncomingStatus): TaskStatus {
	return status === "completed" ? "done" : status;
}

const STATUS_GLYPH: Record<TaskStatus, string> = {
	pending: "[ ]",
	in_progress: "[~]",
	done: "[x]",
	failed: "[!]",
};

/** Title to display: the active-form while in progress, otherwise the content. */
function displayTitle(item: TodoWriteParams["todos"][number]): string {
	if (item.status === "in_progress" && item.activeForm?.trim()) return item.activeForm.trim();
	return item.content.trim();
}

/** Current main-agent tasks (source unset, not delegated/MCP), in stable creation order. */
function mainTasks(): Task[] {
	return taskStore
		.list()
		.filter((t) => t.source === undefined && t.agent === undefined && t.parentTaskId === undefined);
}

/** Create the TodoWrite tool definition. Registered as a customTool when enabled. */
export function createTodoWriteToolDefinition(): ToolDefinition {
	return defineTool<typeof todoWriteParams, TodoWriteDetails>({
		name: TODO_WRITE_TOOL_NAME,
		label: TODO_WRITE_TOOL_NAME,
		description: [
			"Maintain a structured todo list for the current task, shown live in the task panel.",
			"Use it PROACTIVELY: at the start of any multi-step or non-trivial task, write the full plan as todos before you begin, then keep it current as you work.",
			"Mark exactly ONE item in_progress at a time, and flip an item to completed immediately after finishing it — do not batch completions or leave finished work marked in_progress.",
			"Each call sends the FULL list and REPLACES the previous one — always include every item with its current status; omitting an item removes it.",
			"Skip it only for trivial, single-step tasks where a list adds no value. When in doubt on multi-step work, use it — it keeps you from losing track of steps.",
		].join("\n"),
		promptSnippet:
			"Plan and track multi-step work as a live todo list (use proactively; replaces the whole list each call)",
		promptGuidelines: [
			"Use TodoWrite proactively for any multi-step or non-trivial task: write the plan as todos up front, keep exactly one item in_progress, and mark items completed immediately as you finish them.",
			"TodoWrite replaces the entire list each call — always send all items with their current status.",
			"Skip TodoWrite for trivial single-step tasks where a checklist adds no value.",
		],
		parameters: todoWriteParams,
		async execute(_toolCallId, params: TodoWriteParams) {
			const todos = params.todos ?? [];
			const existing = mainTasks();

			// Batch all mutations so the TUI renders once, not per-item.
			taskStore.batch(() => {
				// Reconcile by position: update kept items, create new ones, remove the tail.
				for (let i = 0; i < todos.length; i++) {
					const item = todos[i]!;
					const status = toTaskStatus(item.status);
					const title = displayTitle(item);
					const current = existing[i];
					if (current) {
						taskStore.update(current.id, { title, status });
					} else {
						const created = taskStore.create(title);
						taskStore.update(created.id, { status });
					}
				}
				for (let i = todos.length; i < existing.length; i++) {
					taskStore.remove(existing[i]!.id);
				}
			});

			const counts = todos.reduce(
				(acc, t) => {
					if (t.status === "in_progress") acc.inProgress++;
					else if (t.status === "completed") acc.completed++;
					else acc.pending++;
					return acc;
				},
				{ pending: 0, inProgress: 0, completed: 0 },
			);

			const lines = todos.map((t) => `${STATUS_GLYPH[toTaskStatus(t.status)]} ${displayTitle(t)}`);
			const header =
				todos.length === 0
					? "Todo list cleared."
					: `Todos updated (${counts.inProgress} in progress, ${counts.pending} pending, ${counts.completed} completed):`;
			const text = todos.length === 0 ? header : `${header}\n${lines.join("\n")}`;

			return {
				content: [{ type: "text" as const, text }],
				details: {
					total: todos.length,
					pending: counts.pending,
					inProgress: counts.inProgress,
					completed: counts.completed,
				},
			};
		},
	});
}
