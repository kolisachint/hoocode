import type { Component, Focusable, TUI } from "@kolisachint/hoocode-tui";
import { getKeybindings, matchesKey, truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import { formatDurationSecs } from "../../../core/format-duration.js";
import type { Task, TaskAgent, TaskAgentKind, TaskAgentState, TaskStatus } from "../../../core/task-store.js";
import { taskOwnerId, taskStore } from "../../../core/task-store.js";
import type { ThemeColor } from "../theme/theme.js";
import { agentColorFor, theme } from "../theme/theme.js";
import { appKeyLabel, matchesAppKey, rawKeyHint } from "./keybinding-hints.js";

const TASK_STATUS_ICON: Record<TaskStatus, string> = {
	// Hollow = not started, matching the hollow-means-lighter logic of ◇ and the
	// selectors' ○/◉ convention; also keeps ● exclusive to the chat's tool
	// status dot so the two never read as the same signal.
	pending: "○",
	in_progress: "◐",
	done: "✓",
	failed: "✗",
	cancelled: "⊘",
};

/**
 * U+26A0 with VS15 (text presentation). Bare ⚠ carries the Unicode Emoji
 * property, and terminals with emoji font fallback render it as a double-width
 * color glyph while the width math counts 1 cell — misaligning exactly the
 * rows that carry a warning. VS15 forces single-cell text rendering and is
 * itself zero-width, so visibleWidth stays correct.
 */
const WARNING_GLYPH = "⚠︎";

/** U+25B6 with VS15 — same emoji-presentation hazard as WARNING_GLYPH. */
const SELECTED_GLYPH = "▶︎";

/**
 * Single-cell marker for MCP-sourced rows, which have no owning agent. Every
 * other row derives its marker from the owner's kind via AGENT_GLYPH (◆ main /
 * ◇ subagent / ▸ team role), so the flat lens attributes a row exactly the way
 * the grouped lenses do. The row also carries a text origin tag before the
 * title (see formatTaskLine).
 */
const MCP_SOURCE_GLYPH = "⧉";

/** Braille spinner frames + cadence, matched to the TUI Loader so the active
 * row animates in step. Every tick re-renders the whole component tree, so the
 * cadence is a direct tax on long sessions — 120ms stays smooth while cutting
 * a third of the render load the old 80ms cadence generated. */
const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_INTERVAL_MS = 120;

/** A thin colored left rail groups the pane without a box, the way the design's `border-left` does. */
const RAIL = "▎";

/** Cells in the deterministic progress bar (matches the design's 14-cell track). */
const PROGRESS_CELLS = 14;

/**
 * How the same task list is presented:
 * - flat       → one ungrouped list (default)
 * - subagents  → grouped by owning agent (◆ main orchestrator + ◇ workers)
 * - teams      → grouped by named role-agent (▸), with handoff arrows
 */
export type TaskPanelView = "flat" | "subagents" | "teams";

const VIEW_LABEL: Record<TaskPanelView, string> = { flat: "tasks", subagents: "subagents", teams: "teams" };

/**
 * A top-level task the main agent owns directly: its own TodoWrite plan. Source
 * is unset (not a subagent/MCP delegation) and it sits at the root of the forest
 * (not a child merged in from a subagent's subtree). These are exactly the rows
 * the flat ("tasks") lens shows.
 */
function isMainTask(task: Task): boolean {
	// Source-unset (not a subagent/MCP delegation — note taskOwnerId folds MCP into
	// "main", so check source directly), not owned by a role agent, and a forest
	// root (not a child merged in from a subagent's subtree).
	return task.source === undefined && task.agent === undefined && task.parentTaskId === undefined;
}

/**
 * Lenses that currently have content, split by ownership:
 * - flat ("tasks")  → only when the main agent has its own TodoWrite plan.
 * - subagents       → when delegated work exists (a registered subagent, or any
 *                     subagent/MCP-sourced task).
 * - teams           → when role agents are registered (hooteams `--team`).
 * The cycle key and the header switcher both skip empty lenses, and an empty
 * flat lens falls through to subagents (see render), so a session that only
 * delegated work opens straight on the task tree with no empty "tasks" view.
 */
function availableViews(tasks: readonly Task[], agents: readonly TaskAgent[]): TaskPanelView[] {
	const views: TaskPanelView[] = [];
	if (tasks.some(isMainTask)) views.push("flat");
	const hasSubagentWork =
		agents.some((a) => a.kind === "subagent") || tasks.some((t) => t.source === "subagent" || t.source === "mcp");
	if (hasSubagentWork) views.push("subagents");
	if (agents.some((a) => a.kind === "role")) views.push("teams");
	return views;
}

/** Owner glyphs: main agent a filled diamond, spawned subagents the hollow counterpart, team roles a triangle. */
const AGENT_GLYPH: Record<TaskAgentKind, string> = { main: "◆", subagent: "◇", role: "▸" };
const AGENT_GLYPH_COLOR: Record<TaskAgentKind, ThemeColor> = {
	main: "accent",
	subagent: "accent",
	role: "borderAccent",
};

/** Color for an agent's lifecycle `[state]` tag (mirrors the design's .ast-* classes). */
const AGENT_STATE_COLOR: Record<TaskAgentState, ThemeColor> = {
	active: "warning",
	running: "warning",
	done: "success",
	queued: "dim",
	idle: "dim",
	waiting: "mdLink",
	failed: "error",
	// User-initiated stop: settled and intentional, so muted rather than red.
	cancelled: "dim",
};

/** Two-cell indent under a group header, with a faint vertical guide. */
const GROUP_INDENT_PLAIN = "│ ";

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
			// pending and cancelled: quiet gray — a user-initiated cancel is not an
			// error and must not paint the row (or the rail) red.
			return "dim";
	}
}

/**
 * Wall-clock time a task occupied. A settled task reads its create→final-update
 * span; a task still in progress is measured against the clock so its elapsed
 * time advances while it runs (updatedAt barely moves during a run, which used
 * to freeze the display at ~0s until completion).
 */
function taskElapsedSecs(task: Task, now: number = Date.now()): number {
	const end = task.status === "in_progress" ? now : task.updatedAt;
	return Math.max(0, (end - task.createdAt) / 1000);
}

/**
 * Wall-clock span of the whole visible batch: earliest creation → now (while
 * anything is still running) or the latest settle time. NOT the sum of
 * per-task spans — summing made the header clock tick at 2× with two
 * concurrent subagents and 3× with three, a nonsense "double time" display.
 * Per-run timers live on the individual rows.
 */
function turnElapsedSecs(tasks: readonly Task[], now: number = Date.now()): number {
	if (tasks.length === 0) return 0;
	let earliest = Number.POSITIVE_INFINITY;
	let latestEnd = 0;
	let anyRunning = false;
	for (const task of tasks) {
		earliest = Math.min(earliest, task.createdAt);
		if (task.status === "in_progress") anyRunning = true;
		else latestEnd = Math.max(latestEnd, task.updatedAt);
	}
	const end = anyRunning ? now : latestEnd;
	return Math.max(0, (end - earliest) / 1000);
}

/** The agent-type key a roster entry colors by: "explore#2" → "explore". */
function agentTypeOfName(name: string): string {
	const idx = name.indexOf("#");
	return idx > 0 ? name.slice(0, idx) : name;
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
 * View switcher rendered at the right edge of the ledger header: the labels
 * of the lenses that have content joined by `·`, the active one in bold
 * accent. Hidden entirely when only one lens is available. Purely an
 * indicator in the TUI — the bound key cycles it (see app.tasks.cycleView).
 */
function formatViewSwitcher(
	view: TaskPanelView,
	available: readonly TaskPanelView[],
): { plain: string; styled: string } {
	if (available.length < 2) return { plain: "", styled: "" };
	const plain = available.map((v) => VIEW_LABEL[v]).join(" · ");
	const styled = available
		.map((v) => (v === view ? theme.bold(theme.fg("accent", VIEW_LABEL[v])) : theme.fg("dim", VIEW_LABEL[v])))
		.join(theme.fg("dim", " · "));
	return { plain, styled };
}

/**
 * Ledger header: a state stamp (◐ working / ✓ reviewed / ✗ stopped) + a
 * deterministic progress bar and done/total count on the left, and the per-turn
 * token + elapsed + cost delta (summed across the tasks below) plus the view
 * switcher on the right.
 */
function formatHeader(
	tasks: readonly Task[],
	width: number,
	state: PanelState,
	totalSecs: number,
	view: TaskPanelView,
	available: readonly TaskPanelView[],
): string {
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
		const elapsed = formatDurationSecs(totalSecs);
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

	// Right cluster: turn delta, then the view switcher at the far edge. The
	// switcher is the first thing dropped when the terminal narrows; the turn
	// delta next; the stamp/count survive to the end. Either piece may be
	// absent (no usage reported / only one lens available).
	const switcher = formatViewSwitcher(view, available);
	const rightVariants: Array<{ plain: string; styled: string }> = [];
	if (turnPlain && switcher.plain) {
		rightVariants.push({
			plain: `${turnPlain}  ${switcher.plain}`,
			styled: `${turnText}  ${switcher.styled}`,
		});
	}
	if (turnPlain) rightVariants.push({ plain: turnPlain, styled: turnText });
	else if (switcher.plain) rightVariants.push(switcher);

	for (const right of rightVariants) {
		if (visibleWidth(leftFullPlain) + 2 + visibleWidth(right.plain) <= width) {
			const pad = Math.max(2, width - visibleWidth(leftFullPlain) - visibleWidth(right.plain));
			return leftFull + " ".repeat(pad) + right.styled;
		}
		if (visibleWidth(leftMinPlain) + 2 + visibleWidth(right.plain) <= width) {
			const pad = Math.max(2, width - visibleWidth(leftMinPlain) - visibleWidth(right.plain));
			return leftMin + " ".repeat(pad) + right.styled;
		}
	}
	if (visibleWidth(leftFullPlain) <= width) {
		return leftFull + " ".repeat(width - visibleWidth(leftFullPlain));
	}
	return truncateToWidth(leftMin, width, "…");
}

function formatTokens(count: number): string {
	if (count < 1000) return count.toString();
	if (count < 10000) return `${(count / 1000).toFixed(1)}k`;
	if (count < 1000000) return `${Math.round(count / 1000)}k`;
	return `${(count / 1000000).toFixed(1)}M`;
}

function formatTaskLine(
	task: Task,
	width: number,
	frame: number,
	options: { grouped?: boolean; owner?: TaskAgent; treePrefix?: string } = {},
): string {
	const isProgress = task.status === "in_progress";
	const iconGlyph = isProgress
		? (SPINNER_FRAMES[frame] ?? TASK_STATUS_ICON.in_progress)
		: TASK_STATUS_ICON[task.status];
	const icon = theme.fg(taskStatusColor(task.status), iconGlyph);

	// In grouped views the group header already carries the row's origin, so the
	// owner glyph and tag are suppressed; the rows sit on a faint indent guide.
	const grouped = options.grouped === true;
	const indent = grouped ? theme.fg("borderMuted", GROUP_INDENT_PLAIN) : "";

	// Owner marker between the status icon and the id, derived from the owning
	// agent's kind so the flat lens attributes rows the same way the grouped
	// lenses do (a roster-less owner falls back on the task's source). MCP rows
	// have no owning agent and keep their own ⧉ marker. Every row carries one
	// cell, so the id column stays aligned.
	const isMcp = task.source === "mcp";
	const ownerKind = options.owner?.kind ?? (task.source === "subagent" ? "subagent" : "main");
	const sourceGlyph = isMcp ? MCP_SOURCE_GLYPH : AGENT_GLYPH[ownerKind];
	// Subagent rows carry their agent's identity color on the glyph (and tag
	// below) — the same hue as the chat's `Agent [type]` line and TaskOutput —
	// so a user can trace one agent across the whole TUI at a glance.
	const agentTypeName = !isMcp && ownerKind === "subagent" ? (task.subagentMode ?? task.agent) : undefined;
	const styledSource = grouped
		? ""
		: agentTypeName
			? theme.fg(agentColorFor(agentTypeName), sourceGlyph)
			: theme.fg("dim", sourceGlyph);

	// Origin tag prefixed to the title, naming who runs the row: the subagent
	// type ("[explore]"), the team role's name ("[planner]"), or the MCP server
	// ("[github]"; "[MCP]" when no server label was recorded). Drawn in accent,
	// parallel to the chat's `Agent [explore]` / `MCP [server › tool]`. Grouped
	// rows drop it — the group header carries the origin — except MCP rows,
	// which group under main without being main's own work.
	let tag = "";
	let tagColor: ThemeColor = "accent";
	if (isMcp) tag = `[${task.subagentMode ?? "MCP"}]`;
	else if (!grouped) {
		if (task.subagentMode) {
			tag = `[${task.subagentMode}]`;
			tagColor = agentColorFor(task.subagentMode);
		} else if (ownerKind === "role" && options.owner) {
			tag = `[${options.owner.name}]`;
			tagColor = agentColorFor(options.owner.name);
		}
	}
	const styledTag = tag ? `${theme.fg(tagColor, tag)} ` : "";
	const title = task.title;
	// The title carries the line. Done titles fade to muted
	// (settled work), pending dim (not started), active goes bold, failed turns red.
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
		case "cancelled":
			styledTitle = theme.fg("dim", theme.strikethrough(title));
			break;
		case "in_progress":
			styledTitle = theme.bold(title);
			break;
		default:
			styledTitle = title;
	}

	// Right column: settled rows carry their token usage; the
	// active row reads `running…`, pending rows read `queued`.
	let rightPlain = "";
	let rightStyled = "";
	if (task.status === "done" || task.status === "failed") {
		let tokenText = "";
		if (task.usage) {
			const totalTok = task.usage.input + task.usage.output;
			if (totalTok > 0) tokenText = formatTokens(totalTok);
		}
		if (tokenText) {
			rightPlain = tokenText;
			rightStyled = theme.fg("muted", tokenText);
		}
	} else if (task.status === "cancelled") {
		rightPlain = "cancelled";
		rightStyled = theme.fg("dim", rightPlain);
	} else if (task.status === "in_progress") {
		// Surface the owning agent's live activity (the tool it's currently running,
		// fed by the pool's task_progress events) so a delegated row reads "⋯ grep"
		// rather than a static "running…" — the difference between looking busy and
		// looking stuck. Falls back to "running…" between tools (activity cleared) or
		// when no owner is resolved. Each running row carries its own live timer —
		// the per-run clock the header (a single wall-clock span) deliberately
		// doesn't show. Ticks smoothly because active rows re-render per spinner
		// frame anyway.
		const liveActivity = options.owner?.activity;
		const runFor = formatDurationSecs(taskElapsedSecs(task));
		rightPlain = `${liveActivity ? `⋯ ${liveActivity}` : "running…"} · ${runFor}`;
		rightStyled =
			theme.fg("warning", liveActivity ? `⋯ ${liveActivity}` : "running…") + theme.fg("dim", ` · ${runFor}`);
	} else if (task.status === "pending") {
		rightPlain = "queued";
		rightStyled = theme.fg("dim", rightPlain);
	}

	// A warning note (e.g. inherited-model fallback, exhaustion skip) takes over the
	// right column as a ⚠ cue, replacing the usage/status stamp for that row.
	if (task.note) {
		rightPlain = `${WARNING_GLYPH} ${task.note}`;
		rightStyled = theme.fg("warning", rightPlain);
	}

	const rightWidth = rightPlain ? visibleWidth(rightPlain) + 1 : 0;
	const leftWidth = Math.max(0, width - rightWidth);

	// truncateToWidth measures visible width (ANSI-aware), so the styled left can be
	// truncated against the full left budget directly. Subtracting the prefix here
	// (as a prior version did) truncated titles early and unevenly per id width.
	// In the subagents tree, a depth-first connector prefix (└─/├─/│) sits before
	// the row's glyph; roots pass an empty prefix and read exactly like flat rows.
	const treePrefix = options.treePrefix ? theme.fg("borderMuted", options.treePrefix) : "";
	const leftBody = grouped
		? `${indent}${icon} ${styledTag}${styledTitle}`
		: `${treePrefix}${icon} ${styledSource} ${styledTag}${styledTitle}`;
	const left = truncateToWidth(leftBody, leftWidth, "…");

	// Pad every row to the full pane width so rows align regardless of whether they
	// carry a right column (token usage / running… / queued).
	if (!rightPlain) {
		const pad = Math.max(0, width - visibleWidth(left));
		return left + " ".repeat(pad);
	}

	const pad = Math.max(1, width - visibleWidth(left) - visibleWidth(rightPlain));
	return left + " ".repeat(pad) + rightStyled;
}

/**
 * Filter tasks and agents to the teams lens: only role agents and the tasks they
 * own. The flat and subagents lenses do their own ownership filtering inline (by
 * source and parentTaskId), so this is teams-specific.
 */
function filterTasksForView(
	tasks: readonly Task[],
	agents: readonly TaskAgent[],
	_view: TaskPanelView,
): { filteredTasks: readonly Task[]; filteredAgents: readonly TaskAgent[] } {
	const roleIds = new Set(agents.filter((a) => a.kind === "role").map((a) => a.id));
	return {
		filteredAgents: agents.filter((a) => a.kind === "role"),
		filteredTasks: tasks.filter((t) => roleIds.has(taskOwnerId(t))),
	};
}

/** One renderable row of the subagents lens: a task plus its tree connector. */
interface SubagentTreeRow {
	task: Task;
	treePrefix: string;
}

/**
 * The subagents lens's rows in render order: a depth-first walk of the
 * delegated forest. Roots are parentless subagent/MCP tasks, plus orphans —
 * children whose parent was dropped from the store (their `parentTaskId` no
 * longer resolves), which used to be counted by the header but never rendered.
 * A visited set guards against `parentTaskId` cycles, so a corrupt link can
 * never hang the render loop. Both the lens filter (header counts) and the
 * renderer consume this list, so count and rendering can never disagree.
 */
function subagentLensRows(tasks: readonly Task[]): SubagentTreeRow[] {
	const ids = new Set<number>();
	for (const task of tasks) ids.add(task.id);
	const childrenByParent = new Map<number, Task[]>();
	for (const task of tasks) {
		if (task.parentTaskId === undefined) continue;
		const siblings = childrenByParent.get(task.parentTaskId);
		if (siblings) siblings.push(task);
		else childrenByParent.set(task.parentTaskId, [task]);
	}
	const roots = tasks.filter(
		(t) =>
			(t.parentTaskId === undefined && (t.source === "subagent" || t.source === "mcp")) ||
			(t.parentTaskId !== undefined && !ids.has(t.parentTaskId)),
	);

	const rows: SubagentTreeRow[] = [];
	const visited = new Set<number>();
	const walk = (task: Task, prefix: string, isLast: boolean, isRoot: boolean): void => {
		if (visited.has(task.id)) return;
		visited.add(task.id);
		rows.push({ task, treePrefix: isRoot ? "" : `${prefix}${isLast ? "└─ " : "├─ "}` });
		const kids = childrenByParent.get(task.id) ?? [];
		const childPrefix = isRoot ? "" : `${prefix}${isLast ? "   " : "│  "}`;
		for (let i = 0; i < kids.length; i++) {
			walk(kids[i] as Task, childPrefix, i === kids.length - 1, false);
		}
	};
	for (const root of roots) walk(root, "", true, true);
	return rows;
}

/**
 * The flat ("tasks") lens's rows in render order: the main agent's TodoWrite
 * plan, with each subagent run nested (└─/├─) under the plan item it was
 * dispatched for (`linkedTaskId`, recorded when exactly one item was
 * in_progress). This is the visual tie between the plan and the agents
 * executing it — previously a spawn was linked in spirit but invisible here,
 * living only in the subagents lens. Runs with no link (or a dangling one —
 * their todo was replaced) stay out of this lens; they remain visible in the
 * subagents tree.
 */
function flatLensRows(tasks: readonly Task[]): SubagentTreeRow[] {
	const linkedByTodo = new Map<number, Task[]>();
	for (const task of tasks) {
		if (task.linkedTaskId === undefined || task.source !== "subagent") continue;
		const siblings = linkedByTodo.get(task.linkedTaskId);
		if (siblings) siblings.push(task);
		else linkedByTodo.set(task.linkedTaskId, [task]);
	}
	const rows: SubagentTreeRow[] = [];
	for (const task of tasks) {
		if (!isMainTask(task)) continue;
		rows.push({ task, treePrefix: "" });
		const linked = linkedByTodo.get(task.id) ?? [];
		for (let i = 0; i < linked.length; i++) {
			rows.push({ task: linked[i] as Task, treePrefix: i === linked.length - 1 ? "└─ " : "├─ " });
		}
	}
	return rows;
}

/**
 * Scope the full task list to the tasks visible in the given lens. The header,
 * state stamp, and done/total count are derived from this subset so they match
 * exactly what the user sees in the current view.
 */
function filterTasksForLens(
	tasks: readonly Task[],
	agents: readonly TaskAgent[],
	view: TaskPanelView,
): readonly Task[] {
	switch (view) {
		case "flat":
			// Same walk the renderer uses (plan items + their linked runs), so the
			// header's done/total always matches the visible rows.
			return flatLensRows(tasks).map((row) => row.task);
		case "subagents":
			// Exactly the rows the tree renders (same walk), so the header's
			// done/total always matches the visible rows.
			return subagentLensRows(tasks).map((row) => row.task);
		case "teams": {
			const roleIds = new Set<string>();
			for (const a of agents) {
				if (a.kind === "role") roleIds.add(a.id);
			}
			return tasks.filter((t) => roleIds.has(taskOwnerId(t)));
		}
	}
}

/** Fallback group metadata when a task's owner has no roster entry. */
function defaultAgentMeta(id: string): TaskAgent {
	return id === "main"
		? { id, name: "main", role: "orchestrator", kind: "main" }
		: { id, name: id, role: "subagent", kind: "subagent" };
}

/**
 * Partition the flat task list into owner groups. An explicit task.agent wins;
 * otherwise a subagent-sourced task falls into a generic "subagent" group and
 * everything else into "main". Group order is deterministic — main first, then
 * roster order, then stragglers — never reordered by status.
 */
function groupTasks(
	tasks: readonly Task[],
	agents: readonly TaskAgent[],
): Array<{ id: string; meta: TaskAgent; items: Task[] }> {
	const meta = new Map<string, TaskAgent>(agents.map((a) => [a.id, a]));
	const groups = new Map<string, Task[]>();
	for (const task of tasks) {
		const owner = taskOwnerId(task);
		const items = groups.get(owner);
		if (items) items.push(task);
		else groups.set(owner, [task]);
	}
	const order: string[] = [];
	const ordered = new Set<string>();
	const push = (id: string) => {
		if (!ordered.has(id)) {
			ordered.add(id);
			order.push(id);
		}
	};
	if (groups.has("main")) push("main");
	for (const agent of agents) {
		if (groups.has(agent.id)) push(agent.id);
	}
	for (const id of groups.keys()) push(id);
	return order.map((id) => ({
		id,
		meta: meta.get(id) ?? defaultAgentMeta(id),
		items: groups.get(id) ?? [],
	}));
}

/**
 * Group header for the grouped views: owner glyph + bold name + role, the
 * agent's lifecycle `[state]` tag, an optional handoff arrow (teams), then the
 * agent's own token/cost totals + done/total on the right. Mirrors the footer's
 * "every number accounted for" stance, but per agent.
 */
function formatGroupHeader(meta: TaskAgent, items: readonly Task[], width: number, selected = false): string {
	// A focused role row swaps its ▸ for a filled ▶ in accent — the team-focus
	// selection cursor (the owner-kind glyph mapping itself is unchanged).
	// Spawned/team agents carry their identity color on the glyph and name
	// (hashed from the agent type, "explore#2" → "explore"), matching the same
	// agent's color in the chat and TaskOutput; main keeps the accent.
	const identityColor: ThemeColor =
		meta.kind === "main" ? AGENT_GLYPH_COLOR[meta.kind] : agentColorFor(agentTypeOfName(meta.name));
	const glyph = selected
		? theme.fg("accent", SELECTED_GLYPH)
		: theme.fg(identityColor, AGENT_GLYPH[meta.kind] ?? AGENT_GLYPH.subagent);
	const name = selected
		? theme.bold(theme.fg("accent", meta.name))
		: meta.kind === "main"
			? theme.bold(meta.name)
			: theme.bold(theme.fg(identityColor, meta.name));
	// Roles read as a dim "· role" suffix for spawned/team agents; the main
	// orchestrator's role sits brighter (muted), matching the design's .grp-role.
	const role = meta.role
		? meta.kind === "main"
			? ` ${theme.fg("muted", meta.role)}`
			: theme.fg("dim", ` · ${meta.role}`)
		: "";
	const state = meta.state ? ` ${theme.fg(AGENT_STATE_COLOR[meta.state] ?? "dim", `[${meta.state}]`)}` : "";
	// Live activity for a running subagent (e.g. the tool it's executing). Empty
	// string means idle/between-tools, so it disappears rather than lingering stale.
	const activity = meta.activity ? theme.fg("dim", ` ⋯ ${meta.activity}`) : "";
	const handoff = meta.handoff ? ` ${theme.fg("dim", meta.handoff)}` : "";

	const done = items.filter((t) => t.status === "done").length;
	const countPlain = `${done}/${items.length}`;
	const count = theme.fg("muted", `${done}`) + theme.fg("dim", "/") + theme.fg("muted", `${items.length}`);
	const stats = meta.stats;
	let rightPlain = countPlain;
	let rightStyled = count;
	if (stats && (stats.input > 0 || stats.output > 0 || stats.cost > 0)) {
		const statsPlain = `↑${formatTokens(stats.input)} ↓${formatTokens(stats.output)} · $${stats.cost.toFixed(3)}`;
		rightPlain = `${statsPlain}  ${countPlain}`;
		rightStyled = `${theme.fg("dim", statsPlain)}  ${count}`;
	}

	const rightWidth = visibleWidth(rightPlain) + 1;
	const leftWidth = Math.max(0, width - rightWidth);
	const left = truncateToWidth(`${glyph} ${name}${role}${state}${activity}${handoff}`, leftWidth, "…");
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
 * - A single-cell owner glyph (◆ main / ◇ subagent / ▸ team role / ⧉ MCP) sits
 *   before the id, derived from the owning agent's kind, so every row's origin
 *   is readable at a glance even in the flat lens. A text origin tag before the
 *   title names the owner: the subagent type ("[explore]"), the team role
 *   ("[planner]", fed by `--team <url>`), or the MCP server ("[github]").
 * - Three views split by ownership (cycled via app.tasks.cycleView, shown as a
 *   `tasks · subagents · teams` switcher in the header):
 *     - flat ("tasks") — the main agent's own TodoWrite plan, with each
 *       dispatched subagent run nested under the plan item it was dispatched
 *       for (linkedTaskId), so the plan and the agents executing it read as
 *       one picture;
 *     - subagents — a recursive task tree over delegated work, where a subagent
 *       that spawned a subagent shows its nested tasks (roots are the dispatched
 *       subagents and direct MCP calls; children link via parentTaskId, merged
 *       across the process boundary). Each task is its own node keeping its
 *       [subagentMode]/[server] tag, with depth drawn by └─/├─/│ connectors; a
 *       run with only top-level tasks reads flat (no extra indent);
 *     - teams — grouped by named role-agent with handoff arrows.
 *   The cycle is adaptive: empty lenses are skipped and dropped from the
 *   switcher, which hides entirely when only one lens has content. An empty flat
 *   lens (no main task) falls through to subagents when delegated work exists.
 * - LIFO within the window: newest tasks appear at the bottom (closest to the prompt).
 * - Finished tasks carry their wall-clock cost and stay visible until the next
 *   user message arrives (see taskStore.reset()), not the moment they finish.
 * - Collapses to zero lines when there are no tasks — unless a team roster is
 *   registered (`--team`), in which case the empty flat lens falls through to
 *   teams and every role renders as a placeholder group, so idle roles are
 *   visible from startup.
 */
export class TaskPanelComponent implements Component, Focusable {
	private readonly ui: TUI | null;
	private frame = 0;
	private animationTimer: ReturnType<typeof setInterval> | null = null;
	private view: TaskPanelView = "flat";
	private disposed = false;

	// Render memoization. The spinner re-renders the whole pane every 80ms, but
	// between store mutations only the in-progress rows actually change (spinner
	// frame + live activity). Settled/pending row strings and derived structures
	// (agentById, the subagents tree walk) are cached and invalidated by the
	// store's mutation version + pane width, so a frame tick reuses them instead
	// of rebuilding every string.
	private memoVersion = -1;
	private memoWidth = -1;
	private lineMemo = new Map<string, string>();
	private agentByIdMemo: Map<string, TaskAgent> | null = null;
	private subagentRowsMemo: SubagentTreeRow[] | null = null;
	private flatRowsMemo: SubagentTreeRow[] | null = null;

	/** Drop all memoized render state when the store or width changed. */
	private syncMemo(width: number): void {
		const version = taskStore.version();
		if (version === this.memoVersion && width === this.memoWidth) return;
		this.memoVersion = version;
		this.memoWidth = width;
		this.lineMemo.clear();
		this.agentByIdMemo = null;
		this.subagentRowsMemo = null;
		this.flatRowsMemo = null;
	}

	private agentById(agents: readonly TaskAgent[]): Map<string, TaskAgent> {
		if (!this.agentByIdMemo) {
			this.agentByIdMemo = new Map(agents.map((a) => [a.id, a]));
		}
		return this.agentByIdMemo;
	}

	private subagentRows(tasks: readonly Task[]): SubagentTreeRow[] {
		if (!this.subagentRowsMemo) {
			this.subagentRowsMemo = subagentLensRows(tasks);
		}
		return this.subagentRowsMemo;
	}

	private flatRows(tasks: readonly Task[]): SubagentTreeRow[] {
		if (!this.flatRowsMemo) {
			this.flatRowsMemo = flatLensRows(tasks);
		}
		return this.flatRowsMemo;
	}

	/**
	 * formatTaskLine with caching for rows that don't animate. In-progress rows
	 * depend on the spinner frame and the owner's live activity, so they render
	 * fresh every frame; everything else is stable until the next store mutation
	 * or width change (handled by syncMemo).
	 */
	private taskLine(
		task: Task,
		width: number,
		options: { grouped?: boolean; owner?: TaskAgent; treePrefix?: string } = {},
	): string {
		if (task.status === "in_progress") {
			return formatTaskLine(task, width, this.frame, options);
		}
		const key = `${task.id}:${options.grouped ? 1 : 0}:${options.treePrefix ?? ""}:${options.owner?.kind ?? ""}`;
		const hit = this.lineMemo.get(key);
		if (hit !== undefined) return hit;
		const line = formatTaskLine(task, width, this.frame, options);
		this.lineMemo.set(key, line);
		return line;
	}

	// Team focus mode: when the TUI focuses the panel, role rows become a
	// navigable list (↑/↓ select, n nudge, a attach, q/esc back). The selection
	// is tracked by role name so a roster reorder doesn't move the cursor.
	focused = false;
	private selectedRole: string | undefined;
	/** Open the inline nudge editor for the selected role. */
	onNudge?: (role: string) => void;
	/** Open the attach side panel for the selected role. */
	onAttach?: (role: string) => void;
	/** Leave team focus (focus returns to the main editor). */
	onExitFocus?: () => void;

	constructor(ui?: TUI) {
		this.ui = ui ?? null;
	}

	invalidate(): void {
		// Force the next render to rebuild its memoized rows/structures.
		this.memoVersion = -1;
	}

	private roleAgents(): TaskAgent[] {
		return taskStore.agents().filter((a) => a.kind === "role");
	}

	/** The role the team-focus cursor sits on (clamped to the live roster). */
	focusedRole(): string | undefined {
		const roles = this.roleAgents();
		if (roles.length === 0) return undefined;
		const match = roles.find((a) => a.name === this.selectedRole);
		return (match ?? roles[0]).name;
	}

	handleInput(data: string): void {
		const roles = this.roleAgents();
		if (roles.length === 0) {
			this.onExitFocus?.();
			return;
		}
		const keybindings = getKeybindings();
		const index = Math.max(
			0,
			roles.findIndex((a) => a.name === this.selectedRole),
		);
		if (keybindings.matches(data, "tui.select.up")) {
			this.selectedRole = roles[Math.max(0, index - 1)].name;
		} else if (keybindings.matches(data, "tui.select.down")) {
			this.selectedRole = roles[Math.min(roles.length - 1, index + 1)].name;
		} else if (matchesAppKey(data, "app.team.nudge")) {
			const role = this.focusedRole();
			if (role) this.onNudge?.(role);
		} else if (matchesAppKey(data, "app.team.attach")) {
			const role = this.focusedRole();
			if (role) this.onAttach?.(role);
		} else if (matchesKey(data, "q") || keybindings.matches(data, "tui.select.cancel")) {
			this.onExitFocus?.();
		}
		this.ui?.requestRender();
	}

	getView(): TaskPanelView {
		return this.view;
	}

	setView(view: TaskPanelView): void {
		this.view = view;
		this.ui?.requestRender();
	}

	/**
	 * Advance to the next view lens with content (flat → subagents → teams →
	 * flat), skipping empty lenses. With nothing delegated this is a no-op on
	 * flat; a stale view (its lens emptied since selection) snaps back to flat.
	 */
	cycleView(): TaskPanelView {
		const available = availableViews(taskStore.list(), taskStore.agents());
		const idx = available.indexOf(this.view);
		this.view = available[(idx + 1) % available.length] ?? "flat";
		this.ui?.requestRender();
		return this.view;
	}

	/** Run the spinner timer only while a task is active, ticking re-renders. */
	private ensureAnimation(active: boolean): void {
		if (this.disposed) {
			if (this.animationTimer) {
				clearInterval(this.animationTimer);
				this.animationTimer = null;
				this.frame = 0;
			}
			return;
		}
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
		this.disposed = true;
	}

	render(width: number): string[] {
		if (this.disposed) return [];

		this.syncMemo(width);
		const tasks = taskStore.list();
		const allAgents = taskStore.agents();

		// Resolve the lens: keep the stored view while it still has content, else
		// fall through to the first available lens (flat → subagents → teams). The
		// stored view is untouched so an explicit setView choice resumes once its
		// content returns. This is how an empty flat lens (no main/TodoWrite task)
		// falls through to the subagents tree when only delegated work exists, and
		// how an empty pane falls through to the teams roster at startup.
		const available = availableViews(tasks, allAgents);
		let view: TaskPanelView = available.includes(this.view) ? this.view : (available[0] ?? "flat");

		// Team focus always operates on the teams lens — the focused role list is
		// exactly what the lens renders, so the cursor is never invisible.
		if (this.focused) view = "teams";

		// In teams view the roster itself is content: role agents render as
		// placeholder groups even without tasks (idle roles at startup, queued
		// upcoming work).
		const hasRoleRoster = view === "teams" && allAgents.some((a) => a.kind === "role");

		if (tasks.length === 0 && !hasRoleRoster) {
			this.ensureAnimation(false);
			return [];
		}

		// Scope all visual state to the tasks visible in the current lens so the
		// animation, rail color, and header count match exactly what the user sees.
		// The flat and subagents lenses derive from their memoized row walks so
		// filter and render share one computation (one source of truth for counts).
		const lensTasks =
			view === "subagents"
				? this.subagentRows(tasks).map((row) => row.task)
				: view === "flat"
					? this.flatRows(tasks).map((row) => row.task)
					: filterTasksForLens(tasks, allAgents, view);
		const hasActive = lensTasks.some((t) => t.status === "in_progress");
		this.ensureAnimation(hasActive);

		const state = panelState(lensTasks);
		const totalSecs = turnElapsedSecs(lensTasks);
		const railColor = STATE_PRESENTATION[state].color;
		const gutter = `${theme.fg(railColor, RAIL)} `;
		const inner = Math.max(0, width - visibleWidth(RAIL) - 1);

		const lines: string[] = [gutter + formatHeader(lensTasks, inner, state, totalSecs, view, available)];

		if (view === "flat") {
			// The main agent's TodoWrite plan, with each dispatched run nested under
			// the plan item it executes (see flatLensRows). Resolve each row's owner
			// from the roster so the glyph/tag reflect the owning agent's kind and a
			// nested run shows its live activity + timer.
			const agentById = this.agentById(allAgents);
			for (const { task, treePrefix } of this.flatRows(tasks)) {
				lines.push(gutter + this.taskLine(task, inner, { treePrefix, owner: agentById.get(taskOwnerId(task)) }));
			}
			return lines;
		}

		if (view === "subagents") {
			// A recursive task tree over the delegated forest: roots are the main
			// agent's dispatched subagents (and direct MCP calls), children are the
			// tasks they spawned in turn (linked by parentTaskId, merged across the
			// process boundary), so a subagent that spawned a subagent is visible.
			// Each task is its own node keeping its [subagentMode]/[server] tag; depth
			// is drawn with └─/├─/│ connectors (see subagentLensRows for orphan and
			// cycle handling). With only top-level tasks the roots carry an empty
			// prefix and read exactly like flat rows (no extra indent). Owners are
			// resolved so an in-progress row shows the run's live tool activity
			// (⋯ grep) instead of a static "running…".
			const agentById = this.agentById(allAgents);
			for (const { task, treePrefix } of this.subagentRows(tasks)) {
				lines.push(
					gutter +
						this.taskLine(task, inner, {
							treePrefix,
							owner: agentById.get(taskOwnerId(task)),
						}),
				);
			}
			return lines;
		}

		// teams view: role-agent groups with handoff connectors and queued placeholders.
		const { filteredTasks, filteredAgents } = filterTasksForView(tasks, allAgents, view);
		const groups = groupTasks(filteredTasks, filteredAgents);
		const groupIds = new Set(groups.map((g) => g.id));
		// Role agents with no tasks still get a group header: idle roles are the
		// roster at startup, queued ones upcoming work, done/failed ones the
		// state they settled in after reset() dropped their tasks.
		for (const agent of filteredAgents) {
			if (!groupIds.has(agent.id)) {
				groups.push({ id: agent.id, meta: agent, items: [] });
			}
		}
		const cursorRole = this.focused ? this.focusedRole() : undefined;
		for (const group of groups) {
			const selected = cursorRole !== undefined && group.meta.kind === "role" && group.meta.name === cursorRole;
			lines.push(gutter + formatGroupHeader(group.meta, group.items, inner, selected));
			for (const task of group.items) {
				lines.push(gutter + this.taskLine(task, inner, { grouped: true }));
			}
			// Forward-handoff connector: emit "└──→ name" only for "→ name" arrows
			// (not back-references "← name"), and only when the target exists in the
			// visible role roster.
			const { handoff } = group.meta;
			if (handoff) {
				const arrowIdx = handoff.indexOf("→ ");
				if (arrowIdx !== -1) {
					const nextName = handoff.slice(arrowIdx + 2).trim();
					if (filteredAgents.some((a) => a.name === nextName)) {
						const connectorPrefix = `${GROUP_INDENT_PLAIN}  └──→ `;
						const connectorPad = Math.max(0, inner - visibleWidth(connectorPrefix) - visibleWidth(nextName));
						lines.push(
							gutter +
								theme.fg("borderMuted", connectorPrefix) +
								theme.fg("dim", nextName) +
								" ".repeat(connectorPad),
						);
					}
				}
			}
		}
		if (this.focused) {
			// House hint style (see keybinding-hints): dim key + muted description,
			// muted · separators. Arrows and q/esc are literal by convention; the
			// nudge/attach keys resolve from the live keybinding config.
			const sep = theme.fg("muted", " · ");
			const hint = [
				rawKeyHint("↑/↓", "select"),
				rawKeyHint(appKeyLabel("app.team.nudge"), "nudge"),
				rawKeyHint(appKeyLabel("app.team.attach"), "attach"),
				rawKeyHint("q/esc", "back"),
			].join(sep);
			lines.push(gutter + truncateToWidth(hint, inner, "…"));
		}
		return lines;
	}
}
