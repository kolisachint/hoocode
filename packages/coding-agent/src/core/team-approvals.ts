/**
 * Approval-gate coordinator for the hooteams bridge (`--team`).
 *
 * The orchestrator pauses a task by emitting a `task_paused` TeamEvent
 * (question + options) and waits for `POST /tasks/:id/resume`. This class
 * turns that wire contract into the TUI's options pane: it queues gates as
 * they arrive (one prompt on screen at a time), answers the server with the
 * chosen option, and dismisses prompts that another surface (hoocanvas,
 * another attached hoocode) answered first — the server enforces
 * first-answer-wins and 409s stale answers.
 *
 * The coordinator is UI-agnostic: the host injects `present` (show a gate,
 * resolve with the answer, undefined on skip, abort signal on dismissal),
 * which interactive mode backs with its AskOptions pane.
 */

import type { TeamPendingApproval, TeamViewEvent } from "./team-view.js";

/** One approval gate, as queued for presentation. */
export interface TeamApproval {
	taskId: string;
	question: string;
	options: string[];
	/** Role that paused; absent for gates fetched from /tasks/pending. */
	role?: string;
}

export interface TeamApprovalHost {
	/**
	 * Show the gate and resolve with the chosen option (free-form answers
	 * allowed), or undefined when the user skips it. The signal aborts when
	 * the gate is dismissed (answered elsewhere); resolve promptly then.
	 */
	present(approval: TeamApproval, signal: AbortSignal): Promise<string | undefined>;
	/** Deliver the answer: POST /tasks/:id/resume. Rejects on stale/HTTP errors. */
	resume(taskId: string, option: string): Promise<void>;
	info(message: string): void;
	warn(message: string): void;
}

export class TeamApprovalCoordinator {
	private readonly queue: TeamApproval[] = [];
	private current: { approval: TeamApproval; abort: AbortController } | undefined;

	constructor(private readonly host: TeamApprovalHost) {}

	/** Feed every TeamEvent from the shared /events subscription. */
	handleEvent(event: TeamViewEvent): void {
		if (event.type === "task_paused" && typeof event.taskId === "string" && typeof event.question === "string") {
			this.enqueue({
				taskId: event.taskId,
				question: event.question,
				options: Array.isArray(event.options) ? event.options : [],
				role: event.role,
			});
		} else if (
			(event.type === "task_resumed" || event.type === "task_finished") &&
			typeof event.taskId === "string"
		) {
			this.dismiss(event.taskId);
		}
	}

	/** Queue gates that opened before we attached (GET /tasks/pending). */
	enqueuePending(pending: TeamPendingApproval): void {
		this.enqueue({ taskId: pending.taskId, question: pending.question, options: pending.options });
	}

	/** Number of gates waiting behind the one on screen. Exposed for tests. */
	queuedCount(): number {
		return this.queue.length;
	}

	/** Task id of the gate currently on screen, if any. Exposed for tests. */
	presentedTaskId(): string | undefined {
		return this.current?.approval.taskId;
	}

	private enqueue(approval: TeamApproval): void {
		if (this.current?.approval.taskId === approval.taskId) return;
		if (this.queue.some((queued) => queued.taskId === approval.taskId)) return;
		this.queue.push(approval);
		this.pump();
	}

	/** Drop a gate another surface settled: silently when queued, with a notice when on screen. */
	private dismiss(taskId: string): void {
		const index = this.queue.findIndex((queued) => queued.taskId === taskId);
		if (index !== -1) this.queue.splice(index, 1);
		if (this.current?.approval.taskId === taskId) {
			this.current.abort.abort();
		}
	}

	private pump(): void {
		if (this.current) return;
		const next = this.queue.shift();
		if (!next) return;
		const abort = new AbortController();
		this.current = { approval: next, abort };
		void this.host.present(next, abort.signal).then(
			(answer) => {
				const dismissed = abort.signal.aborted;
				this.current = undefined;
				if (dismissed) {
					this.host.info(`Approval for "${next.taskId}" was answered from another surface.`);
				} else if (answer !== undefined) {
					void this.host.resume(next.taskId, answer).then(
						() => this.host.info(`Resumed "${next.taskId}" with "${answer}".`),
						(error) => this.host.warn(`Failed to resume "${next.taskId}": ${String(error)}`),
					);
				} else {
					this.host.info(`Left "${next.taskId}" paused — it stays pending on the team server.`);
				}
				this.pump();
			},
			(error) => {
				this.current = undefined;
				this.host.warn(`Approval prompt for "${next.taskId}" failed: ${String(error)}`);
				this.pump();
			},
		);
	}
}
