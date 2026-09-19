/**
 * When a tip is allowed to appear.
 *
 * `tips.ts` decides *what* to say. This decides *when*, and almost all of it is
 * about the times it must stay quiet. A tip is the lowest-priority thing on the
 * screen: it is worth showing only because the moment it uses was going to be
 * spent anyway, which means the instant that stops being true it must not show
 * at all.
 *
 * ## The two moments
 *
 * **Idle.** The prompt is up, nothing is streaming, and the user has not pressed
 * a key for {@link DEFAULT_IDLE_DELAY_MS}. They are reading the last reply, or
 * thinking, or have walked away. A tip here is read or it is not; either way it
 * cost nothing.
 *
 * **Streaming.** A turn has been running for {@link DEFAULT_STREAMING_DELAY_MS}
 * and the user is watching a spinner. This is the better of the two moments,
 * because the time is definitely being spent and the eye is definitely on the
 * bottom of the screen.
 *
 * ## The times it stays quiet
 *
 * - Whenever the band already has something on it. A tip must never delay,
 *   replace, or queue behind a notification the user actually caused — it is
 *   dropped, and the next timer comes round soon enough.
 * - Inside the cooldown after any tip. Two tips in quick succession reads as a
 *   thing talking at you rather than a thing helping you.
 * - For the whole first {@link DEFAULT_GRACE_MS} of a session. Startup already
 *   has a banner, a changelog and possibly warnings; adding a tip to that is
 *   noise on top of noise.
 * - Whenever the user has turned tips off.
 *
 * ## Timers
 *
 * Every timer is `unref`'d. A pending tip must never be the reason the process
 * is still alive — a one-shot `hoocode -p` run that happens to take 30 seconds
 * should exit the moment its work is done, not when a tip timer fires.
 */

import type { Tip, TipMoment } from "./tips.js";
import { TipRotation } from "./tips.js";

/** Keyboard-quiet time at the prompt before a tip is offered. */
export const DEFAULT_IDLE_DELAY_MS = 45_000;

/**
 * Turn duration before a tip is offered mid-stream.
 *
 * Shorter than the idle delay on purpose: a user watching a spinner is already
 * waiting, whereas an idle user may be mid-thought and is more easily
 * interrupted.
 */
export const DEFAULT_STREAMING_DELAY_MS = 20_000;

/** Minimum gap between two tips, whatever moment they come from. */
export const DEFAULT_COOLDOWN_MS = 180_000;

/** Quiet period after startup, while the banner and changelog are still being read. */
export const DEFAULT_GRACE_MS = 60_000;

export interface TipsControllerOptions {
	/** Read fresh every time, so turning tips off in `/settings` takes effect at once. */
	isEnabled: () => boolean;
	/** True when the notification band has nothing on it and nothing queued. */
	bandIsFree: () => boolean;
	/** Put the tip on the band. */
	show: (tip: Tip) => void;
	rotation: TipRotation;
	idleDelayMs?: number;
	streamingDelayMs?: number;
	cooldownMs?: number;
	graceMs?: number;
	/** Clock, injectable for tests. */
	now?: () => number;
	/** Timer factory, injectable for tests. */
	setTimer?: (fn: () => void, ms: number) => unknown;
	clearTimer?: (handle: unknown) => void;
}

export class TipsController {
	private readonly opts: Required<Pick<TipsControllerOptions, "isEnabled" | "bandIsFree" | "show" | "rotation">> & {
		idleDelayMs: number;
		streamingDelayMs: number;
		cooldownMs: number;
		graceMs: number;
		now: () => number;
		setTimer: (fn: () => void, ms: number) => unknown;
		clearTimer: (handle: unknown) => void;
	};

	private idleTimer: unknown;
	private streamTimer: unknown;
	private lastTipAt: number | undefined;
	private readonly startedAt: number;
	private stopped = false;

	constructor(options: TipsControllerOptions) {
		const setTimer =
			options.setTimer ??
			((fn: () => void, ms: number) => {
				const handle = setTimeout(fn, ms);
				// A pending tip must never hold the process open.
				handle.unref?.();
				return handle;
			});
		this.opts = {
			isEnabled: options.isEnabled,
			bandIsFree: options.bandIsFree,
			show: options.show,
			rotation: options.rotation,
			idleDelayMs: options.idleDelayMs ?? DEFAULT_IDLE_DELAY_MS,
			streamingDelayMs: options.streamingDelayMs ?? DEFAULT_STREAMING_DELAY_MS,
			cooldownMs: options.cooldownMs ?? DEFAULT_COOLDOWN_MS,
			graceMs: options.graceMs ?? DEFAULT_GRACE_MS,
			now: options.now ?? Date.now,
			setTimer,
			clearTimer: options.clearTimer ?? ((handle) => clearTimeout(handle as NodeJS.Timeout)),
		};
		this.startedAt = this.opts.now();
	}

	/**
	 * The user did something — a key, a submit, a command.
	 *
	 * Restarts the idle clock. This is deliberately cheap and called from the
	 * input path, so it does nothing but reset a timer.
	 */
	onActivity(): void {
		this.armIdle();
	}

	/** A turn started: the idle moment is over and the streaming one begins. */
	onTurnStart(): void {
		this.clearIdle();
		this.armStreaming();
	}

	/** A turn ended: back to waiting for the user to go quiet. */
	onTurnEnd(): void {
		this.clearStreaming();
		this.armIdle();
	}

	/** Teardown. Safe to call more than once. */
	stop(): void {
		this.stopped = true;
		this.clearIdle();
		this.clearStreaming();
	}

	private armIdle(): void {
		this.clearIdle();
		if (this.stopped) return;
		this.idleTimer = this.opts.setTimer(() => {
			this.idleTimer = undefined;
			this.offer("idle");
		}, this.opts.idleDelayMs);
	}

	private armStreaming(): void {
		this.clearStreaming();
		if (this.stopped) return;
		this.streamTimer = this.opts.setTimer(() => {
			this.streamTimer = undefined;
			this.offer("streaming");
		}, this.opts.streamingDelayMs);
	}

	private clearIdle(): void {
		if (this.idleTimer !== undefined) this.opts.clearTimer(this.idleTimer);
		this.idleTimer = undefined;
	}

	private clearStreaming(): void {
		if (this.streamTimer !== undefined) this.opts.clearTimer(this.streamTimer);
		this.streamTimer = undefined;
	}

	/**
	 * Show a tip, if every reason not to has been ruled out.
	 *
	 * A refused offer is not rescheduled here: the moment that produced it is
	 * over, and the next one will arm its own timer. Retrying would turn "the
	 * band is busy" into a tip that pounces the instant the user's own
	 * notification fades, which is precisely the interruption this avoids.
	 */
	private offer(moment: TipMoment): void {
		if (this.stopped) return;
		if (!this.opts.isEnabled()) return;

		const now = this.opts.now();
		if (now - this.startedAt < this.opts.graceMs) return;
		if (this.lastTipAt !== undefined && now - this.lastTipAt < this.opts.cooldownMs) return;
		if (!this.opts.bandIsFree()) return;

		const tip = this.opts.rotation.next(moment);
		if (!tip) return;

		this.lastTipAt = now;
		this.opts.show(tip);
	}
}

export { TipRotation };
