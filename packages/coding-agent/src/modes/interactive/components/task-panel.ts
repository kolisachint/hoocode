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

/** Sum the token + cost usage reported by the tasks shown this turn. */
function sumTurnUsage(tasks: readonly Task[]): { input: number; output: number; cost: number } | null {
	let input = 0;
	let output = 0;
	let cost = 0;
	for (const task of tasks) {
		if (!task.usage) continue;
		input += task.usage.input;
		output += task.usage.output;
		cost += task.usage.cost;
	}
	if (input === 0 && output === 0 && cost === 0) return null;
	return { input, output, cost };
}

/**
 * Ledger header: a watched/reviewed stamp plus done/total count on the left, and
 * the per-turn token + cost delta (summed across the tasks below) on the right.
 * The panel is an audit trail — the stamp makes the deterministic "every task
 * watched & reviewed" state glanceable, and the delta surfaces what the turn cost.
 */
function formatHeader(tasks: readonly Task[], width: number): string {
	const total = tasks.length;
	const done = tasks.filter((t) => t.status === "done").length;
	const watching = tasks.some((t) => t.status === "in_progress" || t.status === "pending");

	const stampPlain = watching ? "⟳ watching" : "reviewed ✓ · deterministic";
	// Header is quiet audit chrome: the reviewed stamp sits in dim, only the active
	// "watching" state earns a warning tint. (Design: .task-head color = fg-dim.)
	const stamp = watching ? theme.fg("warning", "⟳ watching") : theme.fg("dim", stampPlain);

	const countPlain = `${done}/${total} done`;
	const leftPlain = `${stampPlain}  ${countPlain}`;
	const left = `${stamp}  ${theme.fg("dim", countPlain)}`;

	const turn = sumTurnUsage(tasks);
	if (!turn) {
		return truncateToWidth(left, width, "…");
	}

	// Cost is omitted when zero (e.g. subscription/untracked) — still show tokens.
	const showCost = turn.cost > 0;
	const costPlain = showCost ? ` $${turn.cost.toFixed(3)}` : "";
	const turnPlain = `turn ↑${formatTokens(turn.input)} ↓${formatTokens(turn.output)}${costPlain}`;
	// Turn delta: muted framing with the numbers one step brighter (bold/full fg),
	// matching the design's `.turntok` (fg-muted) / `.turntok b` (fg) hierarchy.
	let turnText =
		theme.fg("muted", "turn ↑") +
		theme.bold(formatTokens(turn.input)) +
		theme.fg("muted", " ↓") +
		theme.bold(formatTokens(turn.output));
	if (showCost) {
		turnText += ` ${theme.bold(`$${turn.cost.toFixed(3)}`)}`;
	}

	if (visibleWidth(leftPlain) + 2 + visibleWidth(turnPlain) > width) {
		// Too narrow for both — keep the stamp + count.
		return truncateToWidth(left, width, "…");
	}
	const pad = Math.max(2, width - visibleWidth(leftPlain) - visibleWidth(turnPlain));
	return left + " ".repeat(pad) + turnText;
}

function formatTokens(count: number): string {
	if (count < 1000) return count.toString();
	if (count < 10000) return `${(count / 1000).toFixed(1)}k`;
	if (count < 1000000) return `${Math.round(count / 1000)}k`;
	return `${(count / 1000000).toFixed(1)}M`;
}

function formatTaskLine(task: Task, width: number): string {
	const icon = theme.fg(taskStatusColor(task.status), TASK_STATUS_ICON[task.status]);
	const idLabel = `#${task.id}`;
	const title = task.title;
	// The id recedes (dim) so the title carries the line. Done tasks fade their
	// title to muted (work that's settled); active/failed keep full foreground.
	// (Design: .task .id = fg-dim, .task .ttitle = fg, .task.is-done .ttitle = fg-muted.)
	const styledId = theme.fg("dim", idLabel);
	const styledTitle = task.status === "done" ? theme.fg("muted", title) : title;

	// Finished tasks carry an audit stamp: total tokens used + elapsed time. The
	// token count sits one step brighter (muted) than the time (dim), per the
	// design's `.cost` (fg-dim) / `.cost b` (fg-muted) split.
	const settled = task.status === "done" || task.status === "failed";
	let rightPlain = "";
	let rightStyled = "";
	if (settled) {
		const parts: string[] = [];
		let tokenText = "";
		if (task.usage) {
			const total = task.usage.input + task.usage.output;
			if (total > 0) tokenText = formatTokens(total);
		}
		const elapsed = formatElapsed(task);
		if (tokenText) {
			parts.push(tokenText, elapsed);
			rightStyled = theme.fg("muted", tokenText) + theme.fg("dim", ` · ${elapsed}`);
		} else {
			parts.push(elapsed);
			rightStyled = theme.fg("dim", elapsed);
		}
		rightPlain = parts.join(" · ");
	}
	const rightWidth = rightPlain ? visibleWidth(rightPlain) + 1 : 0;
	const leftWidth = Math.max(0, width - rightWidth);

	const plainText = `${TASK_STATUS_ICON[task.status]} ${idLabel} ${title}`;
	const available = Math.max(0, leftWidth - visibleWidth(plainText) + visibleWidth(title));
	const left = truncateToWidth(`${icon} ${styledId} ${styledTitle}`, available, "…");

	if (!rightPlain) return left;

	const pad = Math.max(1, width - visibleWidth(left) - visibleWidth(rightPlain));
	return left + " ".repeat(pad) + rightStyled;
}

/**
 * Task panel rendered just above the editor prompt.
 *
 * - A ledger header (watched/reviewed stamp + done/total count) tops the list.
 * - Shows all tasks with all statuses (pending / in_progress / done / failed).
 * - Subagent mode is intentionally NOT shown here (e.g. no "[explore]" tag) — the
 *   task title is the meaningful label; the mode adds noise in the pane.
 * - LIFO within the window: newest tasks appear at the bottom (closest to the prompt).
 * - Finished tasks carry their wall-clock cost and stay visible until the next
 *   user message arrives (see taskStore.reset()), not the moment they finish.
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
