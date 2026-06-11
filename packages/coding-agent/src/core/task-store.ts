/**
 * Minimal in-process task store.
 *
 * Tracks short-lived tasks (e.g. subagent delegations) so the TUI task panel can
 * display active work. It is a process-level singleton because the tool that
 * creates tasks and the footer that renders them live in the same process and
 * there is no cross-process boundary to cross.
 */

export type TaskStatus = "pending" | "in_progress" | "done" | "failed";

/**
 * What kind of background work owns a task, surfaced as a source glyph in the
 * pane. Unset means the main agent. A "team" origin is reserved for the
 * hooteams integration and stays unwired until that lands.
 */
export type TaskSource = "subagent" | "mcp";

/**
 * Kind of agent that can own tasks in the pane's grouped views:
 * the main session (orchestrator), a spawned subagent, or a named
 * team role-agent (e.g. fed by a hooteams bridge).
 */
export type TaskAgentKind = "main" | "subagent" | "role";

/** Lifecycle word shown as the agent's `[state]` tag in grouped views. */
export type TaskAgentState = "active" | "running" | "done" | "queued" | "idle" | "waiting" | "failed";

/**
 * An agent that owns tasks, rendered as a group header in the pane's
 * `subagents` / `teams` views. Subagent dispatches register themselves here;
 * external orchestrators (hooteams) can upsert role-agents with handoffs.
 */
export interface TaskAgent {
	readonly id: string;
	name: string;
	/** Short descriptor after the name: "orchestrator", "subagent", or a team role like "architect". */
	role?: string;
	kind: TaskAgentKind;
	state?: TaskAgentState;
	/** Handoff arrow text for team views (e.g. "→ reviewer", "← builder"). */
	handoff?: string;
	/** Per-agent token + cost totals, shown right-aligned on the group header. */
	stats?: { input: number; output: number; cost: number };
}

export interface Task {
	readonly id: number;
	title: string;
	status: TaskStatus;
	/** Origin of the task (subagent delegation vs MCP tool call; unset = main agent); drives the pane's source glyph. */
	source?: TaskSource;
	/** Subagent mode when this task is owned by a subagent (e.g. "explore"). */
	subagentMode?: string;
	/** Id of the owning TaskAgent; drives grouping in the pane's subagents/teams views. */
	agent?: string;
	/**
	 * Short warning note surfaced as a ⚠ cue in the task pane (e.g. the subagent
	 * fell back to the inherited model, or was skipped because the provider was
	 * exhausted). Kept terse so it fits the row's right column.
	 */
	note?: string;
	readonly createdAt: number;
	updatedAt: number;
	/** Token and cost usage attributed to this task (e.g. from a subagent session). */
	usage?: {
		input: number;
		output: number;
		cacheRead: number;
		cacheWrite: number;
		cost: number;
	};
}

export interface CreateTaskOptions {
	source?: TaskSource;
	subagentMode?: string;
	agent?: string;
}

export type TaskPatch = Partial<
	Pick<Task, "title" | "status" | "source" | "subagentMode" | "agent" | "usage" | "note">
>;

export type TaskAgentPatch = Partial<Omit<TaskAgent, "id">>;

/**
 * Owner group for a task when no explicit agent is set: subagent-sourced work
 * falls into a generic "subagent" group, everything else belongs to main.
 * Shared by the store's reset() and the pane's grouped views so the two never
 * disagree about which agents still own live tasks.
 */
export function taskOwnerId(task: Pick<Task, "agent" | "source">): string {
	if (task.agent) return task.agent;
	return task.source === "subagent" ? "subagent" : "main";
}

type Listener = () => void;

class TaskStore {
	private tasks: Task[] = [];
	private taskAgents: TaskAgent[] = [];
	private nextId = 1;
	private readonly listeners = new Set<Listener>();

	create(title: string, options: CreateTaskOptions = {}): Task {
		const now = Date.now();
		const task: Task = {
			id: this.nextId++,
			title: title.trim() || "(untitled task)",
			status: "pending",
			source: options.source,
			subagentMode: options.subagentMode,
			agent: options.agent,
			createdAt: now,
			updatedAt: now,
		};
		this.tasks.push(task);
		this.emit();
		return task;
	}

	update(id: number, patch: TaskPatch): void {
		const task = this.tasks.find((t) => t.id === id);
		if (!task) return;
		if (patch.title !== undefined) task.title = patch.title;
		if (patch.status !== undefined) task.status = patch.status;
		if (patch.source !== undefined) task.source = patch.source;
		if (patch.subagentMode !== undefined) task.subagentMode = patch.subagentMode;
		if (patch.agent !== undefined) task.agent = patch.agent;
		if (patch.usage !== undefined) task.usage = patch.usage;
		if (patch.note !== undefined) task.note = patch.note;
		task.updatedAt = Date.now();
		this.emit();
	}

	/**
	 * Register or update an agent for the grouped task views. Merges into an
	 * existing entry with the same id (accumulated stats survive a re-dispatch);
	 * creates it otherwise.
	 */
	upsertAgent(agent: { id: string } & TaskAgentPatch & Pick<TaskAgent, "name" | "kind">): TaskAgent {
		const existing = this.taskAgents.find((a) => a.id === agent.id);
		if (existing) {
			this.applyAgentPatch(existing, agent);
			this.emit();
			return existing;
		}
		const created: TaskAgent = {
			id: agent.id,
			name: agent.name,
			role: agent.role,
			kind: agent.kind,
			state: agent.state,
			handoff: agent.handoff,
			stats: agent.stats,
		};
		this.taskAgents.push(created);
		this.emit();
		return created;
	}

	/** Patch an existing agent (state/handoff/stats…). Unknown ids are ignored. */
	patchAgent(id: string, patch: TaskAgentPatch): void {
		const agent = this.taskAgents.find((a) => a.id === id);
		if (!agent) return;
		this.applyAgentPatch(agent, patch);
		this.emit();
	}

	/** Add a usage delta to an agent's running totals (creating them at zero). */
	addAgentStats(id: string, delta: { input?: number; output?: number; cost?: number }): void {
		const agent = this.taskAgents.find((a) => a.id === id);
		if (!agent) return;
		const stats = agent.stats ?? { input: 0, output: 0, cost: 0 };
		stats.input += delta.input ?? 0;
		stats.output += delta.output ?? 0;
		stats.cost += delta.cost ?? 0;
		agent.stats = stats;
		this.emit();
	}

	agents(): readonly TaskAgent[] {
		return this.taskAgents;
	}

	private applyAgentPatch(agent: TaskAgent, patch: TaskAgentPatch): void {
		if (patch.name !== undefined) agent.name = patch.name;
		if (patch.role !== undefined) agent.role = patch.role;
		if (patch.kind !== undefined) agent.kind = patch.kind;
		if (patch.state !== undefined) agent.state = patch.state;
		if (patch.handoff !== undefined) agent.handoff = patch.handoff;
		if (patch.stats !== undefined) agent.stats = patch.stats;
	}

	remove(id: number): void {
		const idx = this.tasks.findIndex((t) => t.id === id);
		if (idx === -1) return;
		this.tasks.splice(idx, 1);
		this.emit();
	}

	/**
	 * Drop finished tasks and restart numbering from #1 once the pane is empty.
	 *
	 * Called when a new user message arrives: finished tasks from the previous turn
	 * stay visible (with their final status) for the whole turn and are wiped only
	 * when the user starts the next turn, so the next turn opens with an empty pane
	 * and its first task is #1 again. Active (pending/in_progress) tasks are kept —
	 * a follow-up/steer message can arrive while a subagent is still running, and
	 * dropping its task here would orphan the live work (its later status update
	 * would target a removed id and silently vanish). Numbering only restarts once
	 * no active task survives, so ids never collide with a kept task.
	 */
	reset(): void {
		const active = this.tasks.filter((t) => t.status === "pending" || t.status === "in_progress");
		if (active.length === this.tasks.length && this.nextId === 1) return;
		this.tasks = active;
		if (active.length === 0) {
			this.nextId = 1;
			// The roster only matters while it has tasks to group; drop it with them
			// so a fresh turn opens with a clean pane. Agents owning surviving tasks
			// are kept (their group header must not vanish under live rows).
			this.taskAgents = [];
		} else {
			const liveOwners = new Set(active.map(taskOwnerId));
			this.taskAgents = this.taskAgents.filter((a) => liveOwners.has(a.id));
		}
		this.emit();
	}

	list(): readonly Task[] {
		return this.tasks;
	}

	/** Wipe all tasks and restart numbering. Intended for test isolation only. */
	clear(): void {
		this.tasks = [];
		this.taskAgents = [];
		this.nextId = 1;
		this.emit();
	}

	subscribe(listener: Listener): () => void {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	}

	private emit(): void {
		for (const listener of this.listeners) {
			listener();
		}
	}
}

/** Shared, process-wide task store. */
export const taskStore = new TaskStore();
