/**
 * hooteams team client (`--team <url>`).
 *
 * Connects to a running hooteams server, registers every role as a
 * kind="role" agent in the task store, and maps the server's TeamEvent SSE
 * stream onto task-store patches so the task panel's existing "teams" view
 * shows live role state. On top of that mirror the connection exposes
 * steering (POST /steer) and an event subscription used by the attach
 * side-panel — both share the single /events stream; no second SSE
 * connection is ever opened.
 *
 * The connection is best-effort by design — a connect failure or a later
 * drop logs a warning and never blocks (or crashes) the main agent. At most
 * one SSE connection (to /events) is open at any time.
 */

import { type TaskAgentState, type TaskStatus, taskStore } from "./task-store.js";

/** Shape of GET /status: coarse per-role status keyed by role name. */
export type TeamStatusSnapshot = Record<string, { status?: string; lastEventType?: string }>;

/** One frame of GET /events: a hoocode AgentEvent tagged with its producer. */
export interface TeamViewEvent {
	type: string;
	role: string;
	agentId?: string;
	ts?: number;
	toolName?: string;
	args?: unknown;
	isError?: boolean;
	/** Streaming assistant-message delta carried by message_update events. */
	assistantMessageEvent?: { type?: string; delta?: string };
	message?: {
		role?: string;
		errorMessage?: string;
		usage?: { input?: number; output?: number; cost?: { total?: number } };
	};
	/** Task lifecycle fields carried by task_* events from the orchestrator. */
	taskId?: string;
	/** Approval gate carried by task_paused. */
	question?: string;
	options?: string[];
	/** Answer carried by task_resumed. */
	chosenOption?: string;
	/** "done" | "error" on task_finished. */
	status?: string;
	/** Run id carried by dag_complete / dag_failed. */
	runId?: string;
}

/** One unanswered approval gate from GET /tasks/pending. */
export interface TeamPendingApproval {
	taskId: string;
	question: string;
	options: string[];
}

/** hooteams AgentStatus word → task panel agent state. */
function stateFromStatus(status: string | undefined): TaskAgentState {
	switch (status) {
		case "idle":
			return "idle";
		case "thinking":
		case "streaming":
			return "active";
		case "tool":
			return "running";
		case "done":
			return "done";
		case "error":
			return "failed";
		case "paused":
			return "waiting";
		default:
			return "idle";
	}
}

function taskStatusFromState(state: TaskAgentState): TaskStatus {
	switch (state) {
		case "done":
			return "done";
		case "failed":
			return "failed";
		case "idle":
			// Idle is settled, not queued: a "pending" task here would survive every
			// taskStore.reset() and pin the pane at "working" for the whole session.
			return "done";
		default:
			return "in_progress";
	}
}

/** Only these states represent activity worth a task row of its own. */
function stateWarrantsTask(state: TaskAgentState): boolean {
	return state === "active" || state === "running" || state === "failed" || state === "waiting";
}

/**
 * Maps team status snapshots and TeamEvents onto task-store patches.
 *
 * Each role owns one roster entry (id `team:<role>`) and at most one task whose
 * title tracks the role's latest activity. Tasks exist only while a role is
 * actually doing something (active/running, or failed so the error is visible);
 * idle roles keep their roster entry but no task — the panel's teams lens
 * renders them as placeholder groups, so a quiet team reads as an idle roster
 * instead of pinning the pane at "working". Entries are re-created on demand
 * because taskStore.reset() wipes finished tasks between user turns.
 */
export class TeamViewMapper {
	private readonly store: typeof taskStore;
	private readonly taskIds = new Map<string, number>();

	constructor(store: typeof taskStore = taskStore) {
		this.store = store;
	}

	/** Register roles from a GET /status snapshot. */
	applyStatus(snapshot: TeamStatusSnapshot): void {
		for (const [role, info] of Object.entries(snapshot)) {
			const state = stateFromStatus(info?.status);
			this.ensureRole(role, state, info?.lastEventType ?? "connected");
			this.patchRole(role, state);
		}
	}

	/** Map one TeamEvent from GET /events onto the store. */
	applyEvent(event: TeamViewEvent): void {
		if (!event || typeof event.role !== "string" || event.role.length === 0) return;
		// Dag settlement events are tagged role="orchestrator" — they describe
		// the run, not a member, and must not create a phantom roster entry.
		if (event.type === "dag_complete" || event.type === "dag_failed") return;
		const role = event.role;
		switch (event.type) {
			case "agent_start":
			case "turn_start":
				this.ensureRole(role, "active", "thinking");
				this.patchRole(role, "active", "thinking");
				break;
			case "message_start":
			case "message_update":
			case "message_end":
				this.ensureRole(role, "active", "responding");
				this.patchRole(role, "active");
				break;
			case "tool_execution_start":
				this.ensureRole(role, "running", `tool: ${event.toolName ?? "?"}`);
				this.patchRole(role, "running", `tool: ${event.toolName ?? "?"}`);
				break;
			case "tool_execution_end":
				this.ensureRole(role, "active", "thinking");
				this.patchRole(role, "active", "thinking");
				break;
			case "turn_end":
				if (event.message?.role === "assistant" && event.message.errorMessage) {
					this.ensureRole(role, "failed", "error");
					this.patchRole(role, "failed", "error");
				}
				break;
			case "agent_end": {
				// A failed run stays failed; a paused run (waiting on an approval gate)
				// stays waiting — hooteams mirrors agent_end right after task_paused, so
				// without this guard the gate's "waiting" state would flip to idle and the
				// roster would read as done while the AskOptions pane is still open.
				// agent_end only marks clean completions (mirrors hooteams' own tracking).
				const current = this.store.agents().find((a) => a.id === this.agentId(role))?.state;
				if (current !== "failed" && current !== "waiting") {
					this.ensureRole(role, "done", "idle");
					this.patchRole(role, "done", "idle");
				}
				break;
			}
			case "task_started":
				this.ensureRole(role, "active", `task: ${event.taskId ?? "?"}`);
				this.patchRole(role, "active", `task: ${event.taskId ?? "?"}`);
				break;
			case "task_paused": {
				const title = `awaiting approval: ${event.question ?? "?"}`;
				this.ensureRole(role, "waiting", title);
				this.patchRole(role, "waiting", title);
				break;
			}
			case "task_resumed":
				this.ensureRole(role, "active", `task: ${event.taskId ?? "?"}`);
				this.patchRole(role, "active", `task: ${event.taskId ?? "?"}`);
				break;
			case "task_finished": {
				const state = event.status === "error" ? "failed" : "done";
				this.ensureRole(role, state, state === "failed" ? "error" : "idle");
				this.patchRole(role, state, state === "failed" ? "error" : "idle");
				break;
			}
			default:
				// Unknown event types still prove the role exists.
				this.ensureRole(role, "idle", event.type);
				break;
		}
	}

	private agentId(role: string): string {
		return `team:${role}`;
	}

	/**
	 * Make sure the role's roster entry exists, plus its task when the state
	 * warrants one (reset() may have dropped both). Idle/done states never
	 * create a task — only patch one that live activity already opened.
	 */
	private ensureRole(role: string, state: TaskAgentState, title: string): void {
		const id = this.agentId(role);
		this.store.upsertAgent({ id, name: role, kind: "role", state });
		const taskId = this.taskIds.get(role);
		const existing = taskId !== undefined ? this.store.list().find((task) => task.id === taskId) : undefined;
		if (!existing && stateWarrantsTask(state)) {
			const task = this.store.create(title, { agent: id });
			this.store.update(task.id, { status: taskStatusFromState(state) });
			this.taskIds.set(role, task.id);
		}
	}

	private patchRole(role: string, state: TaskAgentState, title?: string): void {
		this.store.patchAgent(this.agentId(role), { state });
		const taskId = this.taskIds.get(role);
		if (taskId === undefined) return;
		this.store.update(taskId, { status: taskStatusFromState(state), ...(title !== undefined ? { title } : {}) });
	}
}

export interface TeamViewOptions {
	/** Warning sink; defaults to console.error. */
	warn?: (message: string) => void;
	/** Store override for tests. */
	store?: typeof taskStore;
	/** Delay between reconnect attempts in ms (default 5000). */
	retryDelayMs?: number;
}

export interface TeamViewConnection {
	/** Close the SSE connection and stop reconnecting. */
	stop(): void;
	/** POST /steer { role, message }. Rejects on network or HTTP error. */
	steer(role: string, message: string): Promise<void>;
	/**
	 * Answer a paused task: POST /tasks/:taskId/resume { option, feedback? }.
	 * Rejects with "answered elsewhere" on 409 (first answer wins across
	 * surfaces) and with HTTP/network errors otherwise.
	 */
	resume(taskId: string, option: string, feedback?: string): Promise<void>;
	/**
	 * GET /tasks/pending — gates that opened before we attached. Resolves to
	 * [] when the server has no active run (404) so callers need no special
	 * casing.
	 */
	pendingApprovals(): Promise<TeamPendingApproval[]>;
	/**
	 * Subscribe to every TeamEvent delivered by the shared /events stream.
	 * Returns an unsubscribe function. Listeners receive events for all roles;
	 * per-role filtering is the subscriber's job (the attach panel filters).
	 */
	subscribe(listener: (event: TeamViewEvent) => void): () => void;
	/** Number of live event subscribers. Exposed for leak tests. */
	subscriberCount(): number;
}

const STATUS_TIMEOUT_MS = 5000;
const STEER_TIMEOUT_MS = 5000;

/**
 * Start the read-only team view against a hooteams server base URL.
 *
 * Returns immediately; all network work happens in the background and any
 * failure is reported through `warn` without ever throwing.
 */
export function connectTeamView(url: string, options: TeamViewOptions = {}): TeamViewConnection {
	const base = url.replace(/\/+$/, "");
	const warn = options.warn ?? ((message: string) => console.error(message));
	const retryDelayMs = options.retryDelayMs ?? 5000;
	const mapper = new TeamViewMapper(options.store);
	const controller = new AbortController();
	const listeners = new Set<(event: TeamViewEvent) => void>();
	let stopped = false;

	const deliver = (event: TeamViewEvent): void => {
		mapper.applyEvent(event);
		for (const listener of listeners) {
			try {
				listener(event);
			} catch {
				// A broken subscriber must not take down the stream or its peers.
			}
		}
	};

	const run = async (): Promise<void> => {
		// 1. Status snapshot: register the current roles.
		try {
			const response = await fetch(`${base}/status`, {
				signal: AbortSignal.any([controller.signal, AbortSignal.timeout(STATUS_TIMEOUT_MS)]),
			});
			if (!response.ok) throw new Error(`HTTP ${response.status}`);
			mapper.applyStatus((await response.json()) as TeamStatusSnapshot);
		} catch (error) {
			if (stopped) return;
			warn(`team view: failed to fetch ${base}/status (${String(error)}); continuing without the team view`);
		}

		// 2. Single SSE subscription, reconnecting on drops.
		let announcedDrop = false;
		while (!stopped) {
			try {
				const response = await fetch(`${base}/events`, { signal: controller.signal });
				if (!response.ok || !response.body) throw new Error(`HTTP ${response.status}`);
				const reader = response.body.getReader();
				const decoder = new TextDecoder();
				let buffer = "";
				while (true) {
					const { value, done } = await reader.read();
					if (done) break;
					// Only a stream that actually delivers data counts as recovered. A 200
					// that closes immediately (e.g. a server that answers /events without
					// streaming) used to re-arm the warning and repeat it every retry.
					announcedDrop = false;
					buffer += decoder.decode(value, { stream: true });
					let index = buffer.indexOf("\n\n");
					while (index !== -1) {
						const frame = buffer.slice(0, index);
						buffer = buffer.slice(index + 2);
						for (const line of frame.split("\n")) {
							if (!line.startsWith("data:")) continue;
							try {
								deliver(JSON.parse(line.slice(5).trim()) as TeamViewEvent);
							} catch {
								// Malformed frames are dropped; the stream stays up.
							}
						}
						index = buffer.indexOf("\n\n");
					}
				}
				if (stopped) return;
				throw new Error("stream ended");
			} catch (error) {
				if (stopped || controller.signal.aborted) return;
				if (!announcedDrop) {
					announcedDrop = true;
					warn(`team view: lost connection to ${base}/events (${String(error)}); retrying in background`);
				}
				await new Promise((resolve) => setTimeout(resolve, retryDelayMs));
			}
		}
	};

	void run().catch((error) => {
		if (!stopped) warn(`team view: unexpected error (${String(error)})`);
	});

	return {
		stop() {
			stopped = true;
			listeners.clear();
			controller.abort();
		},
		async steer(role: string, message: string): Promise<void> {
			const response = await fetch(`${base}/steer`, {
				method: "POST",
				headers: { "content-type": "application/json" },
				body: JSON.stringify({ role, message }),
				signal: AbortSignal.any([controller.signal, AbortSignal.timeout(STEER_TIMEOUT_MS)]),
			});
			if (!response.ok) throw new Error(`HTTP ${response.status}`);
		},
		async resume(taskId: string, option: string, feedback?: string): Promise<void> {
			const response = await fetch(`${base}/tasks/${encodeURIComponent(taskId)}/resume`, {
				method: "POST",
				headers: { "content-type": "application/json" },
				body: JSON.stringify({ option, ...(feedback !== undefined ? { feedback } : {}) }),
				signal: AbortSignal.any([controller.signal, AbortSignal.timeout(STEER_TIMEOUT_MS)]),
			});
			if (response.status === 409) throw new Error(`task "${taskId}" was answered elsewhere`);
			if (!response.ok) throw new Error(`HTTP ${response.status}`);
		},
		async pendingApprovals(): Promise<TeamPendingApproval[]> {
			const response = await fetch(`${base}/tasks/pending`, {
				signal: AbortSignal.any([controller.signal, AbortSignal.timeout(STATUS_TIMEOUT_MS)]),
			});
			if (response.status === 404) return [];
			if (!response.ok) throw new Error(`HTTP ${response.status}`);
			const body = (await response.json()) as { pending?: TeamPendingApproval[] };
			return Array.isArray(body.pending) ? body.pending : [];
		},
		subscribe(listener: (event: TeamViewEvent) => void): () => void {
			listeners.add(listener);
			return () => {
				listeners.delete(listener);
			};
		},
		subscriberCount(): number {
			return listeners.size;
		},
	};
}
