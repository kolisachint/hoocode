/**
 * The band that tells you what just changed, and then stops telling you.
 *
 * ## The problem it replaces
 *
 * Moving a dial used to write a line into the transcript: `Model: opus-5`,
 * `Chrome: compact`, `Tool output: peek`. Those lines are true for about a
 * second and then they are litter — a session where someone found their
 * thinking level by stepping through it carries five rows of dead settings
 * chatter forever, interleaved with the conversation that transcript is
 * supposed to be a record of. Warnings had the same shape from the other end: a
 * filled block, kept for the life of the session, for something ("No previous
 * directory to return to") that needs to be seen once and never read again.
 *
 * ## The shape
 *
 * One band, one notification at a time, directly above the prompt — which is
 * where the eye already is, and the one place in a bottom-anchored layout that
 * never moves.
 *
 * ## Why a glimpse replaces and a warning waits
 *
 * They are different kinds of message and the queue is where that shows.
 *
 * A glimpse reports *state*: what the model is now, where the thinking dial
 * landed. Only the newest one is true, so a glimpse standing in line replaces
 * itself — holding the dial key down flashes the value it ended on, not five
 * values in sequence three seconds apart, each of them already wrong.
 *
 * A warning reports an *event*: something happened that you did not ask about.
 * Every one of them is still true when the next arrives, so warnings queue and
 * are shown in turn. Collapsing them would mean the last of three startup
 * warnings silently erasing the two before it.
 *
 * ## What comes through here
 *
 * Every dial step, every warning, and everything a command has to say for
 * itself — `Mode set to "build"`, `Cloned to new session`, the plugin
 * catalogue, what `/learn` read. All of those used to write a dimmed row into
 * the conversation, and a session where someone ran five commands carried five
 * rows of dead receipts between the messages the transcript exists to keep.
 *
 * ## What does not
 *
 * Anything you might want to read later. A share URL, an export path, where
 * credentials were saved and every error still go to the transcript
 * (`InteractiveMode.showRecord`), which is the thing that scrolls back. The
 * rule is: if missing it costs you nothing, it belongs here; if missing it
 * costs you the information, it does not.
 */

import type { Component } from "@kolisachint/hoocode-tui";
import { applyBackgroundToLine, truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import { type BlockFill, type ThemeColor, theme } from "../theme/theme.js";

/**
 * How long each kind stays up.
 *
 * A glimpse confirms something the user just did on purpose, so it only has to
 * outlast the glance. A warning is telling them something they did not ask
 * about, and is usually longer, so it gets the time to be read.
 */
export const NOTIFICATION_TTL_MS = { info: 3000, warning: 8000 } as const;

export type NotificationKind = keyof typeof NOTIFICATION_TTL_MS;

/**
 * Reading time a row of body earns on top of the headline's.
 *
 * The band carries listings now — the plugin catalogue, what `/learn` read,
 * where a canvas went — and a five-row listing timed like a one-line glimpse is
 * a listing nobody finished. The ceiling is there because the band is still
 * chrome: past half a minute it is not a notification, it is a pane that forgot
 * to close.
 */
const BODY_ROW_MS = 1200;
const MAX_TTL_MS = 30_000;

/** The fill behind each kind. Both are block fills, so the band reads as paper. */
const FILL: Record<NotificationKind, BlockFill> = { info: "customMessageBg", warning: "warningBg" };

/**
 * Rows of body the band will show under the headline, when nobody says.
 *
 * Bounded because this is chrome: a forty-line listing that grows the band
 * pushes the conversation off the screen to say something it is about to erase
 * anyway. What does not fit is dropped rather than scrolled — the band has no
 * way to be scrolled, and a message whose remainder cannot be reached reads as
 * a bug. An owner that knows the screen's height passes `maxBodyRows` and gets
 * a share of it instead of this floor.
 */
const MAX_BODY_ROWS = 3;

/**
 * How many notifications may be waiting.
 *
 * A bound, not a design: warnings arriving faster than eight seconds apart for
 * longer than a minute is a malfunction, and the useful thing to do with it is
 * to stop growing rather than to show a queue nobody will sit through.
 */
const MAX_QUEUE = 8;

/** Leads the headline; filled for a warning, hollow for a glimpse. */
const GLYPH = { info: "◦", warning: "●" } as const;

/**
 * Colour a row, unless it coloured itself.
 *
 * A listing that paints its own columns arrives here already styled, and
 * `theme.fg` closes with `\x1b[39m` — reset foreground, not *restore* — so
 * wrapping it would drop this colour at the first inner span and leave the rest
 * of the row unstyled. Passing a styled line through is the same rule
 * `styleStatusMessage` follows in the transcript, and the reason a listing
 * still reads in its own colours now that listings come through the band.
 */
function ink(color: ThemeColor, text: string): string {
	return text.includes("\x1b[") ? text : theme.fg(color, text);
}

export interface Notification {
	kind: NotificationKind;
	title: string;
	body: string[];
	/** Right-aligned afterword — the key that steps a dial back, usually. */
	note?: string;
	/** Overrides the per-kind default. */
	ttlMs?: number;
}

export class NotificationPanel implements Component {
	/** Shared across every empty frame: identity is all the render caches compare. */
	private static readonly EMPTY: string[] = Object.freeze([]) as unknown as string[];

	/** Head is on screen; the rest are waiting their turn. */
	private queue: Notification[] = [];
	private timer: NodeJS.Timeout | undefined;
	// Keyed on the row budget too: a terminal resized shorter changes what fits
	// without changing the width, and a cache that only watches the width would
	// go on drawing a band the screen no longer has room for.
	private cache?: { width: number; rows: number; lines: string[] };

	/**
	 * @param requestRender - the band appears and disappears on its own clock,
	 *   so it is the one piece of chrome that has to ask for frames rather than
	 *   being drawn into one somebody else asked for.
	 * @param maxBodyRows - how many rows of body the screen can spare right now.
	 *   Asked per frame rather than held: the terminal is resized under a
	 *   notification as readily as under anything else.
	 */
	constructor(
		private readonly requestRender: () => void,
		private readonly maxBodyRows: () => number = () => MAX_BODY_ROWS,
	) {}

	/** What is on the band right now, for tests and for the chrome checks. */
	get showing(): Notification | undefined {
		return this.queue[0];
	}

	/** What is waiting behind it. */
	get pending(): readonly Notification[] {
		return this.queue.slice(1);
	}

	/**
	 * Put a notification up: a glimpse replaces, a warning queues (see above).
	 */
	notify(kind: NotificationKind, title: string, body: string[] = [], note?: string, ttlMs?: number): void {
		const next: Notification = { kind, title, body: body.filter((line) => line.length > 0), note, ttlMs };
		const tail = this.queue.length - 1;
		// A glimpse only replaces another glimpse that has not been seen yet.
		// Overwriting the *head* would cut short a message already on screen, and
		// a notification that can be erased before it is read is not one.
		if (kind === "info" && tail > 0 && this.queue[tail].kind === "info") {
			this.queue[tail] = next;
		} else {
			if (this.queue.length >= MAX_QUEUE) this.queue.splice(1, 1);
			this.queue.push(next);
		}
		this.cache = undefined;
		if (this.queue.length === 1) this.arm(next);
		this.requestRender();
	}

	/** Take the band down now, queue and all; safe when nothing is up. */
	dismiss(): void {
		this.clearTimer();
		if (this.queue.length === 0) return;
		this.queue = [];
		this.cache = undefined;
		this.requestRender();
	}

	/** Drop the pending fade without repainting — for teardown. */
	stop(): void {
		this.clearTimer();
	}

	invalidate(): void {
		// Styled at render time, so a theme swap has to rebuild the cached rows.
		this.cache = undefined;
	}

	/**
	 * The band, painted.
	 *
	 * It carries everything a command has to say now — a mode landing, a plugin
	 * installed, a listing — so it has to read at a glance against a transcript
	 * that is already full of text. A fill is what does that: the same block
	 * fill a message sheet takes, so the band is recognisably the app's own
	 * paper rather than one more line of output, and the eye finds its edges
	 * without anything having to be drawn around it.
	 *
	 * Every painted row runs the full width — a fill that stops short of the
	 * margin is a band with a bite out of it — and the one column of lead-in is
	 * what keeps the glyph off the screen's edge.
	 */
	render(width: number): string[] {
		const current = this.queue[0];
		if (!current || width < 4) return NotificationPanel.EMPTY;
		const rows = this.budget();
		const cached = this.cache;
		if (cached && cached.width === width && cached.rows === rows) return cached.lines;

		const fill = FILL[current.kind];
		const paint = (line: string): string => applyBackgroundToLine(line, width, (text) => theme.bg(fill, text));
		const glyph = theme.fg(current.kind === "warning" ? "warning" : "muted", GLYPH[current.kind]);
		// The leading blank is this block's separator from whatever is above it.
		// The band never pads its own bottom edge: the next row is the prompt's
		// own border, and a blank line next to a rule is a blank line wasted (see
		// "Vertical rhythm" in docs/ui-map.md).
		// One column of page either side of the text: the fill runs edge to edge,
		// what is written on it does not, and a note flush against the screen's
		// last cell reads as text that ran out of room.
		const lines = ["", paint(` ${glyph} ${this.headline(current, width - 4)}`)];
		for (const line of this.bodyRows(current)) {
			lines.push(paint(`   ${ink("muted", truncateToWidth(line, Math.max(1, width - 4)))}`));
		}
		this.cache = { width, rows, lines };
		return lines;
	}

	/** The body rows that fit, at whatever the screen can spare this frame. */
	private bodyRows(current: Notification): string[] {
		return current.body.slice(0, this.budget());
	}

	/** Rows of body this frame can spare — never none, so a body always shows. */
	private budget(): number {
		return Math.max(1, Math.floor(this.maxBodyRows()));
	}

	/**
	 * Title on the left, note flush right, the note dropped before the title is.
	 *
	 * The note is a hint about a key; the title is the thing that happened. On a
	 * terminal too narrow for both, losing the hint costs nothing and losing the
	 * headline costs the whole notification.
	 */
	private headline(current: Notification, width: number): string {
		const color = current.kind === "warning" ? "warning" : "text";
		const note = current.note;
		if (note) {
			const gap = width - visibleWidth(current.title) - visibleWidth(note) - 3;
			if (gap >= 0) {
				return ink(color, current.title) + " ".repeat(gap + 3) + theme.fg("halftone", note);
			}
		}
		return ink(color, truncateToWidth(current.title, Math.max(1, width)));
	}

	/**
	 * How long this one stays up: the headline's time plus the body's.
	 *
	 * An explicit `ttlMs` still wins — a caller that knows its message is read
	 * at a glance, or not at all, is a better judge than this arithmetic.
	 */
	private ttl(current: Notification): number {
		if (current.ttlMs !== undefined) return current.ttlMs;
		const base = NOTIFICATION_TTL_MS[current.kind];
		return Math.min(MAX_TTL_MS, base + this.bodyRows(current).length * BODY_ROW_MS);
	}

	private arm(current: Notification): void {
		this.clearTimer();
		const timer = setTimeout(() => {
			this.timer = undefined;
			this.queue.shift();
			this.cache = undefined;
			const next = this.queue[0];
			if (next) this.arm(next);
			this.requestRender();
		}, this.ttl(current));
		// A band waiting to fade must never be the reason the process is still up.
		timer.unref?.();
		this.timer = timer;
	}

	private clearTimer(): void {
		if (this.timer) clearTimeout(this.timer);
		this.timer = undefined;
	}
}
