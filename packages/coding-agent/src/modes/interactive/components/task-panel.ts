import type { Component, TUI } from "@kolisachint/hoocode-tui";
import { truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import type { Task, TaskSource, TaskStatus } from "../../../core/task-store.js";
import { taskStore } from "../../../core/task-store.js";
import { theme } from "../theme/theme.js";

const TASK_STATUS_ICON: Record<TaskStatus, string> = {
	pending: "●",
	in_progress: "◐",
	done: "✓",
	failed: "✗",
};

/**
 * A single-cell source marker placed before the id so a subagent row and an MCP
 * row are distinguishable at a glance. Plain tasks reserve the cell (blank) to keep
 * the id column aligned. The glyph says *where the work came from* at a glance; the
 * row also carries a text origin tag before the title (see formatTaskLine).
 */
const TASK_SOURCE_GLYPH: Record<TaskSource, string> = {
	subagent: "⚙",
	mcp: "⧉",
};

/** Braille spinner frames + cadence, matched to the TUI Loader so the active row animates in step. */
const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_INTERVAL_MS = 80;

/** A thin colored left rail groups the pane without a box, the way the design's `border-left` does. */
const RAIL = "▎";

/** Cells in the deterministic progress bar (matches the design's 14-cell track). */
const PROGRESS_CELLS = 14;

/** Overall pane state, derived from the task statuses. Drives the rail color + header stamp. */
type PanelState = "working" | "reviewed" | "stopped";

interface StatePresentation {
	readonly icon: string;
	readonly label: string;
	readonly color: "warning" | "success" | "error";
}

const STATE_PRESENTATION: Record<PanelState, StatePresentation> = {
	working: { icon: "◐", label: "working", color: "warning" },
	reviewed: { icon: "✓", label: "reviewed", color: "success" },
	stopped: { icon: "✗", label: "stopped", color: "error" },
};

function panelState(tasks: readonly Task[]): PanelState {
	if (tasks.some((t) => t.status === "failed")) return "stopped";
	const active = tasks.some((t) => t.status === "in_progress" || t.status === "pending");
	return active ? "working" : "reviewed";
}

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

/** Format a duration in seconds into a compact, terminal-friendly string. */
function formatDuration(secs: number): string {
	const s = Math.max(0, secs);
	if (s < 10) return `${s.toFixed(1)}s`;
	if (s < 60) return `${Math.round(s)}s`;
	const mins = Math.floor(s / 60);
	const rem = Math.round(s % 60);
	return `${mins}m${rem.toString().padStart(2, "0")}s`;
}

/** Wall-clock time a task occupied, derived from its create/update stamps. */
function taskElapsedSecs(task: Task): number {
	return Math.max(0, (task.updatedAt - task.createdAt) / 1000);
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
 * Deterministic block-glyph progress bar: a heavy run (━) for the completed
 * fraction over a dim track. In-progress tasks count as half, so the bar moves
 * the moment work starts. Fraction is the only input — no animation, no guess.
 */
function progressBar(done: number, active: number, total: number): { plain: string; styled: string } {
	const ratio = total > 0 ? Math.max(0, Math.min(1, (done + active * 0.5) / total)) : 0;
	const filled = Math.round(ratio * PROGRESS_CELLS);
	const fill = "━".repeat(filled);
	const track = "━".repeat(PROGRESS_CELLS - filled);
	return {
		plain: fill + track,
		styled: theme.fg("success", fill) + theme.fg("dim", track),
	};
}

/**
 * Ledger header: a state stamp (◐ working / ✓ reviewed / ✗ stopped) + a
 * deterministic progress bar and done/total count on the left, and the per-turn
 * token + elapsed + cost delta (summed across the tasks below) on the right.
 */
function formatHeader(tasks: readonly Task[], width: number, state: PanelState, totalSecs: number): string {
	const total = tasks.length;
	const done = tasks.filter((t) => t.status === "done").length;
	const active = tasks.filter((t) => t.status === "in_progress").length;

	const { icon, label, color } = STATE_PRESENTATION[state];
	const stampPlain = `${icon} ${label.toUpperCase()}`;
	const stamp = `${theme.fg(color, icon)} ${theme.bold(theme.fg(color, label.toUpperCase()))}`;

	const bar = progressBar(done, active, total);
	const countPlain = `${done}/${total}`;
	const count = theme.fg("muted", `${done}`) + theme.fg("dim", "/") + theme.fg("muted", `${total}`);

	// Left cluster has a full form (stamp · bar · count) and a compact fallback
	// (stamp · count) that drops the bar when the terminal is too narrow.
	const leftFullPlain = `${stampPlain}  ${bar.plain} ${countPlain}`;
	const leftFull = `${stamp}  ${bar.styled} ${count}`;
	const leftMinPlain = `${stampPlain} ${countPlain}`;
	const leftMin = `${stamp} ${count}`;

	const turn = sumTurnUsage(tasks);
	let turnPlain = "";
	let turnText = "";
	if (turn) {
		const inTok = formatTokens(turn.input);
		const outTok = formatTokens(turn.output);
		const elapsed = formatDuration(totalSecs);
		const showCost = turn.cost > 0;
		const costStr = showCost ? `$${turn.cost.toFixed(3)}` : "";
		turnPlain = `turn ↑${inTok} ↓${outTok} · ${elapsed}${showCost ? ` · ${costStr}` : ""}`;
		// Turn delta: muted framing, numbers one step brighter (bold), separators dim.
		turnText =
			theme.fg("muted", "turn ↑") +
			theme.bold(inTok) +
			theme.fg("muted", " ↓") +
			theme.bold(outTok) +
			theme.fg("dim", " · ") +
			theme.fg("muted", elapsed) +
			(showCost ? theme.fg("dim", " · ") + theme.bold(costStr) : "");
	}

	if (turnPlain) {
		if (visibleWidth(leftFullPlain) + 2 + visibleWidth(turnPlain) <= width) {
			const pad = Math.max(2, width - visibleWidth(leftFullPlain) - visibleWidth(turnPlain));
			return leftFull + " ".repeat(pad) + turnText;
		}
		if (visibleWidth(leftMinPlain) + 2 + visibleWidth(turnPlain) <= width) {
			const pad = Math.max(2, width - visibleWidth(leftMinPlain) - visibleWidth(turnPlain));
			return leftMin + " ".repeat(pad) + turnText;
		}
	}
	if (visibleWidth(leftFullPlain) <= width) return leftFull;
	return truncateToWidth(leftMin, width, "…");
}

function formatTokens(count: number): string {
	if (count < 1000) return count.toString();
	if (count < 10000) return `${(count / 1000).toFixed(1)}k`;
	if (count < 1000000) return `${Math.round(count / 1000)}k`;
	return `${(count / 1000000).toFixed(1)}M`;
}

function formatTaskLine(task: Task, width: number, frame: number, idColWidth: number): string {
	const isProgress = task.status === "in_progress";
	const iconGlyph = isProgress
		? (SPINNER_FRAMES[frame] ?? TASK_STATUS_ICON.in_progress)
		: TASK_STATUS_ICON[task.status];
	const icon = theme.fg(taskStatusColor(task.status), iconGlyph);

	// Source marker between the status icon and the id. Reserve the cell (blank) for
	// plain tasks so ids stay column-aligned whether or not a glyph is present.
	const sourceGlyph = task.source ? TASK_SOURCE_GLYPH[task.source] : " ";
	const styledSource = task.source ? theme.fg("dim", sourceGlyph) : " ";

	// Right-pad the id to the shared column width so titles line up across rows even
	// when ids differ in digit count (#1 vs #10). Padding is plain spaces inside the
	// dim styling, so it adds no visible color.
	const idLabel = `#${task.id}`.padEnd(idColWidth);
	// Source tag prefixed to the title: the subagent mode (e.g. "[explore]") for
	// subagent rows, "[MCP]" for MCP rows. Drawn in accent so it reads as the row's
	// origin label, parallel to the chat's `Agent [explore]` / `MCP [server › tool]`.
	const tag = task.source === "mcp" ? "[MCP]" : task.subagentMode ? `[${task.subagentMode}]` : "";
	const styledTag = tag ? `${theme.fg("accent", tag)} ` : "";
	const title = task.title;
	// The id recedes (dim); the title carries the line. Done titles fade to muted
	// (settled work), pending dim (not started), active goes bold, failed turns red.
	const styledId = theme.fg("dim", idLabel);
	let styledTitle: string;
	switch (task.status) {
		case "done":
			styledTitle = theme.fg("muted", title);
			break;
		case "pending":
			styledTitle = theme.fg("dim", title);
			break;
		case "failed":
			styledTitle = theme.fg("error", title);
			break;
		case "in_progress":
			styledTitle = theme.bold(title);
			break;
		default:
			styledTitle = title;
	}

	// Right column: settled rows carry their audit stamp (tokens + elapsed); the
	// active row reads `running…`, pending rows read `queued`.
	let rightPlain = "";
	let rightStyled = "";
	if (task.status === "done" || task.status === "failed") {
		const parts: string[] = [];
		let tokenText = "";
		if (task.usage) {
			const totalTok = task.usage.input + task.usage.output;
			if (totalTok > 0) tokenText = formatTokens(totalTok);
		}
		const elapsed = formatDuration(taskElapsedSecs(task));
		if (tokenText) {
			parts.push(tokenText, elapsed);
			rightStyled = theme.fg("muted", tokenText) + theme.fg("dim", ` · ${elapsed}`);
		} else {
			parts.push(elapsed);
			rightStyled = theme.fg("dim", elapsed);
		}
		rightPlain = parts.join(" · ");
	} else if (task.status === "in_progress") {
		rightPlain = "running…";
		rightStyled = theme.fg("warning", rightPlain);
	} else if (task.status === "pending") {
		rightPlain = "queued";
		rightStyled = theme.fg("dim", rightPlain);
	}

	// A warning note (e.g. inherited-model fallback, exhaustion skip) takes over the
	// right column as a ⚠ cue, replacing the usage/status stamp for that row.
	if (task.note) {
		rightPlain = `⚠ ${task.note}`;
		rightStyled = theme.fg("warning", rightPlain);
	}

	const rightWidth = rightPlain ? visibleWidth(rightPlain) + 1 : 0;
	const leftWidth = Math.max(0, width - rightWidth);

	// truncateToWidth measures visible width (ANSI-aware), so the styled left can be
	// truncated against the full left budget directly. Subtracting the prefix here
	// (as a prior version did) truncated titles early and unevenly per id width.
	const left = truncateToWidth(`${icon} ${styledSource} ${styledId} ${styledTag}${styledTitle}`, leftWidth, "…");

	if (!rightPlain) return left;

	const pad = Math.max(1, width - visibleWidth(left) - visibleWidth(rightPlain));
	return left + " ".repeat(pad) + rightStyled;
}

/**
 * Task panel rendered just above the editor prompt.
 *
 * - A state-colored left rail groups the pane (working=warning, reviewed=success,
 *   stopped=error) without drawing a box.
 * - A ledger header tops the list: a state stamp + deterministic progress bar +
 *   done/total count on the left, the per-turn token/elapsed/cost delta on the right.
 * - Shows all tasks with all statuses (pending / in_progress / done / failed).
 *   The active row animates a braille spinner; pending rows read `queued`.
 * - A single-cell source glyph (⚙ subagent / ⧉ MCP) sits before the id so the two
 *   kinds of background work are distinguishable, and a text origin tag is shown
 *   before the title: the subagent mode (e.g. "[explore]") or "[MCP]".
 * - LIFO within the window: newest tasks appear at the bottom (closest to the prompt).
 * - Finished tasks carry their wall-clock cost and stay visible until the next
 *   user message arrives (see taskStore.reset()), not the moment they finish.
 * - Collapses to zero lines when there are no tasks.
 */
export class TaskPanelComponent implements Component {
	private readonly ui: TUI | null;
	private frame = 0;
	private animationTimer: ReturnType<typeof setInterval> | null = null;

	constructor(ui?: TUI) {
		this.ui = ui ?? null;
	}

	invalidate(): void {
		// No cached rendering state.
	}

	/** Run the spinner timer only while a task is active, ticking re-renders. */
	private ensureAnimation(active: boolean): void {
		if (active && this.ui && !this.animationTimer) {
			this.animationTimer = setInterval(() => {
				this.frame = (this.frame + 1) % SPINNER_FRAMES.length;
				this.ui?.requestRender();
			}, SPINNER_INTERVAL_MS);
			this.animationTimer.unref?.();
		} else if (!active && this.animationTimer) {
			clearInterval(this.animationTimer);
			this.animationTimer = null;
			this.frame = 0;
		}
	}

	/** Stop the spinner timer. Call on teardown. */
	dispose(): void {
		if (this.animationTimer) {
			clearInterval(this.animationTimer);
			this.animationTimer = null;
		}
	}

	render(width: number): string[] {
		const tasks = taskStore.list();
		if (tasks.length === 0) {
			this.ensureAnimation(false);
			return [];
		}

		const hasActive = tasks.some((t) => t.status === "in_progress");
		this.ensureAnimation(hasActive);

		const state = panelState(tasks);
		const totalSecs = tasks.reduce((sum, t) => sum + taskElapsedSecs(t), 0);
		const railColor = STATE_PRESENTATION[state].color;
		const gutter = `${theme.fg(railColor, RAIL)} `;
		const inner = Math.max(0, width - visibleWidth(RAIL) - 1);

		// Width of the id column, sized to the widest id on screen, so every title
		// starts at the same column regardless of digit count (#1 vs #10 vs #100).
		const idColWidth = tasks.reduce((max, t) => Math.max(max, `#${t.id}`.length), 0);

		const lines: string[] = [gutter + formatHeader(tasks, inner, state, totalSecs)];
		for (const task of tasks) {
			lines.push(gutter + formatTaskLine(task, inner, this.frame, idColWidth));
		}
		return lines;
	}
}
