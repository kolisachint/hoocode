/**
 * When a canvas's `session.send` reaches the agent — the host's half of a canvas
 * talking back.
 *
 * The SDK lets an extension send the agent a message whenever it likes. A canvas
 * sits in front of a person who edits continuously, so taking that literally
 * would turn every burst of clicks into a model turn. The canvas is expected to
 * send only when there is something to ask; this is the backstop for when it
 * does not, and the rules are the host's so every canvas gets the same ones:
 *
 *  - **Visible.** Every message is labelled `[canvas <id>]` and lands in the
 *    transcript as a user message, so the person sees exactly what the agent was
 *    told on their behalf.
 *  - **Latest wins while the agent is busy.** A canvas's messages sent during a
 *    turn are held, and a newer one replaces the older: a canvas describes its
 *    current state, so the last word is the one that is still true. The held
 *    messages are delivered together once the agent is idle.
 *  - **`immediate` steers, sparingly.** At most one steer per canvas per
 *    {@link CANVAS_STEER_INTERVAL_MS}; the rest wait like `enqueue`.
 *  - **No loops without a person.** A canvas may start at most
 *    {@link CANVAS_MAX_UNATTENDED_TURNS} turns in a row without the person
 *    saying anything. After that its message waits for the person's next turn,
 *    and they are told once that it is waiting.
 *
 * Pure policy with injected effects, so it is tested without a session.
 */

import type { CanvasSendMode } from "./protocol.js";

/** Turns canvases may start back to back before a person has to speak. */
export const CANVAS_MAX_UNATTENDED_TURNS = 3;

/** Minimum gap between two steers from one canvas. */
export const CANVAS_STEER_INTERVAL_MS = 10_000;

/** How often, and how many times, a flush retries while the session settles. */
export const CANVAS_FLUSH_RETRY_MS = 200;
export const CANVAS_FLUSH_RETRIES = 25;

/** What happened to a submitted message. */
export type CanvasInboxOutcome = "started" | "steered" | "queued" | "held";

/** The effects the inbox needs, injected so the policy is testable alone. */
export interface CanvasInboxDeps {
	/** Whether the agent is idle right now. */
	isIdle(): boolean;
	/**
	 * Put a message in front of the agent. `followUp` is used when a turn should
	 * start: it starts one when the agent is idle and queues safely when a turn
	 * began in between.
	 */
	deliver(text: string, deliverAs: "steer" | "followUp"): void;
	/** Tell the person something, once. */
	notify(message: string): void;
	now?: () => number;
	/** Timer, injectable so tests do not sleep. */
	schedule?: (callback: () => void, ms: number) => void;
}

/** The label that marks a message as a canvas's, in the transcript and to the model. */
export function canvasMessageLabel(extensionId: string): string {
	return `[canvas ${extensionId}]`;
}

export class CanvasInbox {
	/** Held messages by extension id; a newer one replaces the older. */
	private readonly pending = new Map<string, string>();
	private readonly lastSteer = new Map<string, number>();
	/** Turns started by canvases since the person last spoke. */
	private unattended = 0;
	/** Whether the person has been told a message is waiting for them. */
	private toldWaiting = false;
	private flushing = false;
	private readonly deps: CanvasInboxDeps;
	private readonly now: () => number;
	private readonly schedule: (callback: () => void, ms: number) => void;

	constructor(deps: CanvasInboxDeps) {
		this.deps = deps;
		this.now = deps.now ?? Date.now;
		this.schedule = deps.schedule ?? ((callback, ms) => void setTimeout(callback, ms));
	}

	/** A canvas sent a message. */
	submit(extensionId: string, prompt: string, mode: CanvasSendMode = "enqueue"): CanvasInboxOutcome {
		const text = `${canvasMessageLabel(extensionId)} ${prompt.trim()}`;
		const idle = this.deps.isIdle();
		if (mode === "immediate" && !idle) {
			const last = this.lastSteer.get(extensionId);
			if (last === undefined || this.now() - last >= CANVAS_STEER_INTERVAL_MS) {
				this.lastSteer.set(extensionId, this.now());
				this.pending.delete(extensionId);
				this.deps.deliver(text, "steer");
				return "steered";
			}
		}
		this.pending.set(extensionId, text);
		if (!idle) return "queued";
		return this.flush() ? "started" : "held";
	}

	/** The agent finished a run. Held messages go out once the session settles. */
	agentEnded(): void {
		this.retryFlush(0);
	}

	/** The person said something: canvases may start turns again. */
	personSpoke(): void {
		this.unattended = 0;
		this.toldWaiting = false;
	}

	/** Messages waiting, for display and tests. */
	waiting(): number {
		return this.pending.size;
	}

	/**
	 * Deliver everything held as one message, if a canvas may start a turn now.
	 * Returns whether it did.
	 */
	private flush(): boolean {
		if (this.pending.size === 0) return false;
		if (this.unattended >= CANVAS_MAX_UNATTENDED_TURNS) {
			if (!this.toldWaiting) {
				this.toldWaiting = true;
				const who = [...this.pending.keys()].join(", ");
				this.deps.notify(
					`Canvas ${who} is waiting to ask the agent something. It is sent once you have said something, so a canvas cannot keep the agent busy on its own.`,
				);
			}
			return false;
		}
		const text = [...this.pending.values()].join("\n\n");
		this.pending.clear();
		this.unattended += 1;
		this.deps.deliver(text, "followUp");
		return true;
	}

	/**
	 * `agent_end` fires while the session is still winding down (compaction may
	 * follow), so the flush waits for idle. If a new turn starts meanwhile — the
	 * person typed — it gives up; that turn's end flushes instead.
	 */
	private retryFlush(attempt: number): void {
		if (this.flushing && attempt === 0) return;
		this.flushing = true;
		if (this.pending.size === 0) {
			this.flushing = false;
			return;
		}
		if (this.deps.isIdle()) {
			this.flushing = false;
			this.flush();
			return;
		}
		if (attempt >= CANVAS_FLUSH_RETRIES) {
			this.flushing = false;
			return;
		}
		this.schedule(() => this.retryFlush(attempt + 1), CANVAS_FLUSH_RETRY_MS);
	}
}
