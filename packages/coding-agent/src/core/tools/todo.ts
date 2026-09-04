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
	// TodoWrite never produces cancelled items; present for Record exhaustiveness.
	cancelled: "[-]",
};

/** Title to display: the active-form while in progress, otherwise the content. */
function displayTitle(item: TodoWriteParams["todos"][number]): string {
	if (item.status === "in_progress" && item.activeForm?.trim()) return item.activeForm.trim();
	return item.content.trim();
}

/**
 * A root task the main agent itself owns, i.e. a TodoWrite plan item: no
 * `source` (excludes "subagent"/MCP rows), no `agent` (excludes delegated rows),
 * and no `parentTaskId` (excludes merged child trees). `taskOwnerId()` would
 * fold MCP-sourced and delegated rows under "main", so reconciling against it
 * could overwrite or drop those rows when the TodoWrite list is shorter than the
 * combined count.
 */
function isMainPlanTask(task: Task): boolean {
	return task.source === undefined && task.agent === undefined && task.parentTaskId === undefined;
}

/** Current main-agent plan tasks, in stable creation order. */
function mainTasks(): Task[] {
	return taskStore.list().filter(isMainPlanTask);
}

function isActive(task: Task): boolean {
	return task.status === "pending" || task.status === "in_progress";
}

/**
 * Settle plan items the model left pinned at `in_progress` when a request ends.
 *
 * TodoWrite is bookkeeping the model performs by hand, and even strong models
 * routinely drop the final call that flips the last item to completed. Nothing
 * else writes main-plan rows, so without this the panel would keep claiming the
 * work is in flight until the next user message triggers `taskStore.reset()`.
 * The request is over, so the row is wrong either way — settle it to the honest
 * outcome instead of leaving it lying.
 *
 * Scope is deliberately narrow:
 * - Only `in_progress` main-plan items. `pending` rows are left alone: "never
 *   started" is already an accurate reading of an item the model skipped.
 * - Only main-plan items. Subagent- and MCP-sourced rows settle through their
 *   own lifecycles (subagent.ts, mcp-loader.ts) and must not be second-guessed
 *   here.
 * - Nothing settles while any delegated task is still pending/in_progress: a
 *   subagent outliving the parent's agent_end is still working the plan, so its
 *   plan item is genuinely in progress.
 *
 * Returns the number of tasks settled.
 */
export function settleDanglingMainTasks(outcome: Extract<TaskStatus, "done" | "cancelled">): number {
	const all = taskStore.list();
	if (all.some((t) => !isMainPlanTask(t) && isActive(t))) return 0;
	const dangling = all.filter((t) => isMainPlanTask(t) && t.status === "in_progress");
	if (dangling.length === 0) return 0;
	taskStore.batch(() => {
		for (const task of dangling) {
			taskStore.update(task.id, { status: outcome });
		}
	});
	return dangling.length;
}

/** Create the TodoWrite tool definition. Registered as a customTool when enabled. */
export function createTodoWriteToolDefinition(): ToolDefinition {
	return defineTool<typeof todoWriteParams, TodoWriteDetails>({
		name: TODO_WRITE_TOOL_NAME,
		label: TODO_WRITE_TOOL_NAME,
		description: [
			"Maintain a structured todo list for the current task, shown live in the task panel.",
			"Write the full plan as todos before starting multi-step or non-trivial work, and keep it current; skip only trivial single-step tasks.",
			"Mark exactly ONE item in_progress at a time, and flip an item to completed immediately after finishing it.",
			"Each call sends the FULL list and REPLACES the previous one — include every item with its current status; omitting an item removes it.",
		].join("\n"),
		promptSnippet:
			"Plan and track multi-step work as a live todo list (use proactively; replaces the whole list each call)",
		// Only the "reach for it at all" cue lives here — that is the system-prompt's
		// job. The calling contract (one in_progress, full-list replacement, when to
		// skip) is already stated in `description` and in the `todos` schema, both of
		// which ship on every turn alongside this. Same rule three times spent the
		// tokens three times.
		promptGuidelines: [
			"Use TodoWrite proactively for multi-step or non-trivial work; skip trivial single-step tasks.",
		],
		parameters: todoWriteParams,
		async execute(_toolCallId, params: TodoWriteParams) {
			const todos = params.todos ?? [];
			const existing = mainTasks();

			// Reconcile by item identity first, position second, batched so the panel
			// renders once. Each task stores its item's canonical `content`
			// (todoContent) — the display title flips between content and activeForm
			// with status, so it can't identify an item. Matching by content keeps a
			// task's id pinned to the same plan item when the list is reordered or
			// shrunk; a purely positional reconcile re-labeled the surviving slots,
			// which silently re-pointed the subagent runs linked to those ids
			// (linkedTaskId) at the wrong plan items. Unmatched incoming items then
			// consume the leftover slots in order (a rename keeps its id and its
			// linked runs); any remaining leftovers were removed from the plan.
			taskStore.batch(() => {
				const content = (item: TodoWriteParams["todos"][number]) => item.content.trim();
				const matchedExisting = new Set<number>();
				const assigned = new Array<Task | undefined>(todos.length);
				for (let i = 0; i < todos.length; i++) {
					const idx = existing.findIndex(
						(t, j) => !matchedExisting.has(j) && (t.todoContent ?? t.title) === content(todos[i]!),
					);
					if (idx !== -1) {
						matchedExisting.add(idx);
						assigned[i] = existing[idx];
					}
				}
				const leftovers = existing.filter((_, j) => !matchedExisting.has(j));
				let nextLeftover = 0;
				for (let i = 0; i < todos.length; i++) {
					if (!assigned[i]) assigned[i] = leftovers[nextLeftover++];
				}

				const finalIds: number[] = [];
				for (let i = 0; i < todos.length; i++) {
					const item = todos[i]!;
					const status = toTaskStatus(item.status);
					const title = displayTitle(item);
					const current = assigned[i];
					if (current) {
						taskStore.update(current.id, { title, status, todoContent: content(item) });
						finalIds.push(current.id);
					} else {
						const created = taskStore.create(title);
						taskStore.update(created.id, { status, todoContent: content(item) });
						finalIds.push(created.id);
					}
				}
				for (let j = nextLeftover; j < leftovers.length; j++) {
					taskStore.remove(leftovers[j]!.id);
				}
				// Identity matching keeps ids, but the panel must still show the plan
				// in the list's order — permute the plan tasks into it.
				taskStore.arrange(finalIds);
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
