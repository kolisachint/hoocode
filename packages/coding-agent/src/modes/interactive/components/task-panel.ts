import type { Component } from "@kolisachint/hoocode-tui";
import { truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import type { Task, TaskStatus } from "../../../core/task-store.js";
import { taskStore } from "../../../core/task-store.js";
import { theme } from "../theme/theme.js";

const TASK_STATUS_ICON: Record<TaskStatus, string> = {
	pending: "●",
	in_progress: "◐",
	done: "✓",
	failed: "✗",
};

function taskStatusColor(status: TaskStatus): "dim" | "warning" | "success" | "error" {
	switch (status) {
		case "in_progress":
			return "warning";
		case "done":
			return "success";
		case "failed":
			return "error";
		default:
			return "dim";
	}
}

/** Wall-clock time a task occupied, derived from its create/update stamps. */
function formatElapsed(task: Task): string {
	const secs = Math.max(0, (task.updatedAt - task.createdAt) / 1000);
	if (secs < 10) return `${secs.toFixed(1)}s`;
	if (secs < 60) return `${Math.round(secs)}s`;
	const mins = Math.floor(secs / 60);
	const rem = Math.round(secs % 60);
	return `${mins}m${rem.toString().padStart(2, "0")}s`;
}

/**
 * Ledger header: a watched/reviewed stamp on the left, the done/total count on
 * the right. The panel is an audit trail — the stamp makes the deterministic
 * "every task watched & reviewed" state glanceable.
 */
function formatHeader(tasks: readonly Task[], width: number): string {
	const total = tasks.length;
	const done = tasks.filter((t) => t.status === "done").length;
	const watching = tasks.some((t) => t.status === "in_progress" || t.status === "pending");

	const stampPlain = watching ? "⟳ watching" : "reviewed ✓ · deterministic";
	const stamp = watching
		? theme.fg("warning", "⟳ watching")
		: theme.fg("success", "reviewed ✓") + theme.fg("dim", " · deterministic");

	const countPlain = `${done}/${total} done`;
	const count = theme.fg("dim", countPlain);

	const pad = Math.max(2, width - visibleWidth(stampPlain) - visibleWidth(countPlain));
	if (visibleWidth(stampPlain) + 2 + visibleWidth(countPlain) > width) {
		// Too narrow for both — keep just the stamp.
		return stamp;
	}
	return stamp + " ".repeat(pad) + count;
}

function formatTokens(count: number): string {
	if (count < 1000) return count.toString();
	if (count < 10000) return `${(count / 1000).toFixed(1)}k`;
	if (count < 1000000) return `${Math.round(count / 1000)}k`;
	return `${(count / 1000000).toFixed(1)}M`;
}

function formatTaskLine(task: Task, width: number): string {
	const icon = theme.fg(taskStatusColor(task.status), TASK_STATUS_ICON[task.status]);
	const modeTag = task.subagentMode ? theme.fg("accent", ` [${task.subagentMode}]`) : "";
	const plainModeTag = task.subagentMode ? ` [${task.subagentMode}]` : "";
	const idLabel = `#${task.id}`;
	const title = task.title;

	// Finished tasks carry an audit stamp: tokens + elapsed time.
	const settled = task.status === "done" || task.status === "failed";
	let rightPlain = "";
	if (settled) {
		const parts: string[] = [];
		if (task.usage) {
			if (task.usage.input) parts.push(`↑${formatTokens(task.usage.input)}`);
			if (task.usage.output) parts.push(`↓${formatTokens(task.usage.output)}`);
		}
		parts.push(formatElapsed(task));
		rightPlain = parts.join(" · ");
	}
	const rightWidth = rightPlain ? visibleWidth(rightPlain) + 1 : 0;
	const leftWidth = Math.max(0, width - rightWidth);

	const plainText = `${TASK_STATUS_ICON[task.status]} ${idLabel} ${title}${plainModeTag}`;
	const available = Math.max(0, leftWidth - visibleWidth(plainText) + visibleWidth(title));
	const left = truncateToWidth(`${icon} ${idLabel} ${title}`, available, "...") + modeTag;

	if (!rightPlain) return left;

	const pad = Math.max(1, width - visibleWidth(left) - visibleWidth(rightPlain));
	return left + " ".repeat(pad) + theme.fg("dim", rightPlain);
}

/**
 * Task panel rendered just above the editor prompt.
 *
 * - A ledger header (watched/reviewed stamp + done/total count) tops the list.
 * - Shows all tasks with all statuses (pending / in_progress / done / failed).
 * - LIFO within the window: newest tasks appear at the bottom (closest to the prompt).
 * - Finished tasks carry their wall-clock cost and retire only when the main
 *   agent moves on after a parallel subagent spawn (see taskStore.retireFinished()),
 *   not the moment they finish.
 * - Collapses to zero lines when there are no tasks.
 */
export class TaskPanelComponent implements Component {
	invalidate(): void {
		// No cached rendering state.
	}

	render(width: number): string[] {
		const tasks = taskStore.list();
		if (tasks.length === 0) {
			return [];
		}

		const lines: string[] = [formatHeader(tasks, width)];
		for (const task of tasks) {
			lines.push(formatTaskLine(task, width));
		}
		return lines;
	}
}
