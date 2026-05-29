/**
 * Minimal in-process task store.
 *
 * Tracks short-lived tasks (e.g. subagent delegations) so the TUI footer can
 * display active work. It is a process-level singleton because the tool that
 * creates tasks and the footer that renders them live in the same process and
 * there is no cross-process boundary to cross.
 */

export type TaskStatus = "pending" | "in_progress" | "done" | "failed";

export interface Task {
	readonly id: number;
	title: string;
	status: TaskStatus;
	/** Subagent mode when this task is owned by a subagent (e.g. "explore"). */
	subagentMode?: string;
	readonly createdAt: number;
	updatedAt: number;
}

export interface CreateTaskOptions {
	subagentMode?: string;
}

export type TaskPatch = Partial<Pick<Task, "title" | "status" | "subagentMode">>;

type Listener = () => void;

class TaskStore {
	private tasks: Task[] = [];
	private nextId = 1;
	private readonly listeners = new Set<Listener>();

	create(title: string, options: CreateTaskOptions = {}): Task {
		const now = Date.now();
		const task: Task = {
			id: this.nextId++,
			title: title.trim() || "(untitled task)",
			status: "pending",
			subagentMode: options.subagentMode,
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
		if (patch.subagentMode !== undefined) task.subagentMode = patch.subagentMode;
		task.updatedAt = Date.now();
		this.emit();
	}

	list(): readonly Task[] {
		return this.tasks;
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
