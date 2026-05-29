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

function formatTaskLine(task: Task, width: number): string {
	const icon = theme.fg(taskStatusColor(task.status), TASK_STATUS_ICON[task.status]);
	const modeTag = task.subagentMode ? theme.fg("accent", ` [${task.subagentMode}]`) : "";
	const plainModeTag = task.subagentMode ? ` [${task.subagentMode}]` : "";
	const idLabel = `#${task.id}`;
	const title = task.title;
	const plainText = `${TASK_STATUS_ICON[task.status]} ${idLabel} ${title}${plainModeTag}`;
	const available = Math.max(0, width - visibleWidth(plainText) + visibleWidth(title));
	const left = truncateToWidth(`${icon} ${idLabel} ${title}`, available, "...");
	return left + modeTag;
}

/**
 * Task panel rendered just above the editor prompt.
 *
 * - Shows only active (pending / in_progress) tasks.
 * - LIFO within the window: newest tasks appear at the bottom (closest to the prompt).
 * - Completed or failed tasks drop out automatically so the panel only reflects ongoing work.
 * - Collapses to zero lines when there is nothing running.
 */
export class TaskPanelComponent implements Component {
	invalidate(): void {
		// No cached rendering state.
	}

	render(width: number): string[] {
		const tasks = taskStore.list();
		const active = tasks.filter((t) => t.status === "pending" || t.status === "in_progress");
		if (active.length === 0) {
			return [];
		}

		const lines: string[] = [];
		for (const task of active) {
			lines.push(formatTaskLine(task, width));
		}
		return lines;
	}
}
