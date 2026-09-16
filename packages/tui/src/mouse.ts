/**
 * Mouse reporting: turning it on, and reading what comes back.
 *
 * ## Why the app captures the mouse at all
 *
 * Without mouse reporting the wheel belongs to the terminal, which scrolls its
 * own scrollback under the app's feet. That is fine right up until the app
 * repaints — and this renderer repaints the whole transcript whenever a line
 * above the viewport changes — at which point the terminal snaps back to the
 * bottom and the scroll position the reader was holding is gone. The wheel is
 * captured so that scrolling is one thing the app decides, not a race between
 * the app's writes and the terminal's idea of where the view should sit.
 *
 * The cost is real and worth naming: while reporting is on, a plain click-drag
 * is delivered here instead of selecting text. Every terminal worth using keeps
 * a bypass on **shift** (option on iTerm2), so selection is one modifier away
 * rather than gone. `HOOCODE_MOUSE=0` turns the whole thing off for anyone who
 * would rather have the drag back.
 *
 * ## What is enabled, and what deliberately is not
 *
 * - `?1000h` — button press and release, which is what carries wheel events.
 * - `?1006h` — SGR encoding, so coordinates past column 223 survive. Terminals
 *   that ignore it keep sending the X10 form, which `parseMouseEvent` also
 *   reads, so a terminal without SGR degrades to "works up to column 223"
 *   rather than to garbage on screen.
 *
 * `?1002h` (drag tracking) and `?1003h` (any-motion) are **not** enabled. They
 * would deliver a motion event per cell of travel — thousands of wakeups for a
 * gesture this app has no use for — and `?1003h` in particular fires on plain
 * pointer movement over the window, so an idle terminal with the mouse resting
 * over it would never stop waking the render loop.
 */

/** Enable button + wheel reporting, preferring SGR coordinates. */
export const MOUSE_ENABLE = "\x1b[?1000h\x1b[?1006h";

/** Disable in the reverse order it was enabled. */
export const MOUSE_DISABLE = "\x1b[?1006l\x1b[?1000l";

export type MouseEventKind = "wheelUp" | "wheelDown" | "wheelLeft" | "wheelRight" | "press" | "release";

export interface MouseEvent {
	kind: MouseEventKind;
	/** 0 left, 1 middle, 2 right. -1 for wheel events, which name no button. */
	button: number;
	/** 1-based, as the terminal reports it. */
	column: number;
	/** 1-based, as the terminal reports it. */
	row: number;
	shift: boolean;
	alt: boolean;
	ctrl: boolean;
}

const SGR_PATTERN = /^\x1b\[<(\d+);(\d+);(\d+)([Mm])/;
// X10: ESC [ M then three bytes, each its value plus 32.
const X10_PREFIX = "\x1b[M";

/**
 * Whether `data` starts with a mouse report.
 *
 * Used to swallow reports the app does not act on. A mouse sequence that
 * reaches a text field is typed into it — `<35;10;4M` appearing in the prompt
 * is this check having been skipped.
 */
export function isMouseSequence(data: string): boolean {
	if (SGR_PATTERN.test(data)) return true;
	return data.startsWith(X10_PREFIX) && data.length >= X10_PREFIX.length + 3;
}

/**
 * How many bytes the leading mouse report occupies, or 0 if there is none.
 *
 * Reports arrive coalesced — a fast wheel spin delivers several in one read,
 * and a keystroke can ride along behind them — so the caller peels them off one
 * at a time rather than treating the whole chunk as a single event.
 */
export function mouseSequenceLength(data: string): number {
	const sgr = data.match(SGR_PATTERN);
	if (sgr) return sgr[0].length;
	if (data.startsWith(X10_PREFIX) && data.length >= X10_PREFIX.length + 3) {
		return X10_PREFIX.length + 3;
	}
	return 0;
}

function decode(flags: number, column: number, row: number, pressed: boolean): MouseEvent {
	const shift = (flags & 4) !== 0;
	const alt = (flags & 8) !== 0;
	const ctrl = (flags & 16) !== 0;

	// Bit 6 marks the wheel, and the low two bits then say which way it turned
	// rather than which button was hit.
	if ((flags & 64) !== 0) {
		const direction = flags & 3;
		const kind: MouseEventKind =
			direction === 0 ? "wheelUp" : direction === 1 ? "wheelDown" : direction === 2 ? "wheelLeft" : "wheelRight";
		return { kind, button: -1, column, row, shift, alt, ctrl };
	}

	const button = flags & 3;
	// X10 has no separate release code per button: button 3 *is* the release,
	// which is why SGR's trailing `m` exists and why this has to check both.
	if (!pressed || button === 3) {
		return { kind: "release", button: button === 3 ? -1 : button, column, row, shift, alt, ctrl };
	}
	return { kind: "press", button, column, row, shift, alt, ctrl };
}

/**
 * Read the mouse report at the start of `data`, or null if there is not one.
 *
 * Coordinates come back 1-based, exactly as the terminal sends them; callers
 * that index into a line subtract the one themselves rather than having it
 * subtracted here, where it would be invisible.
 */
export function parseMouseEvent(data: string): MouseEvent | null {
	const sgr = data.match(SGR_PATTERN);
	if (sgr) {
		const flags = Number(sgr[1]);
		const column = Number(sgr[2]);
		const row = Number(sgr[3]);
		if (!Number.isFinite(flags) || !Number.isFinite(column) || !Number.isFinite(row)) return null;
		return decode(flags, column, row, sgr[4] === "M");
	}

	if (data.startsWith(X10_PREFIX) && data.length >= X10_PREFIX.length + 3) {
		const flags = data.charCodeAt(X10_PREFIX.length) - 32;
		const column = data.charCodeAt(X10_PREFIX.length + 1) - 32;
		const row = data.charCodeAt(X10_PREFIX.length + 2) - 32;
		if (flags < 0 || column < 0 || row < 0) return null;
		return decode(flags, column, row, true);
	}

	return null;
}
