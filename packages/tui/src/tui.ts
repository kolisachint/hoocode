/**
 * Minimal TUI implementation with differential rendering
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { performance } from "node:perf_hooks";
import { isKeyRelease, matchesKey } from "./keys.js";
import { type MouseEvent, mouseSequenceLength, parseMouseEvent } from "./mouse.js";
import type { Terminal } from "./terminal.js";
import { deleteKittyImage, getCapabilities, isImageLine, setCellDimensions } from "./terminal-image.js";
import {
	extractSegments,
	normalizeTerminalOutput,
	sliceByColumn,
	sliceWithWidth,
	truncateToWidth,
	visibleWidth,
} from "./utils.js";

const KITTY_SEQUENCE_PREFIX = "\x1b_G";

function extractKittyImageIds(line: string): number[] {
	const sequenceStart = line.indexOf(KITTY_SEQUENCE_PREFIX);
	if (sequenceStart === -1) return [];

	const paramsStart = sequenceStart + KITTY_SEQUENCE_PREFIX.length;
	const paramsEnd = line.indexOf(";", paramsStart);
	if (paramsEnd === -1) return [];

	const params = line.slice(paramsStart, paramsEnd);
	for (const param of params.split(",")) {
		const [key, value] = param.split("=", 2);
		if (key !== "i" || value === undefined) continue;
		const id = Number(value);
		if (Number.isInteger(id) && id > 0 && id <= 0xffffffff) {
			return [id];
		}
	}
	return [];
}

/**
 * Component interface - all components must implement this
 */
export interface Component {
	/**
	 * Render the component to lines for the given viewport width
	 * @param width - Current viewport width
	 * @returns Array of strings, each representing a line
	 */
	render(width: number): string[];

	/**
	 * Optional handler for keyboard input when component has focus
	 */
	handleInput?(data: string): void;

	/**
	 * If true, component receives key release events (Kitty protocol).
	 * Default is false - release events are filtered out.
	 */
	wantsKeyRelease?: boolean;

	/**
	 * Invalidate any cached rendering state.
	 * Called when theme changes or when component needs to re-render from scratch.
	 */
	invalidate(): void;
}

type InputListenerResult = { consume?: boolean; data?: string } | undefined;
type InputListener = (data: string) => InputListenerResult;

/**
 * Interface for components that can receive focus and display a hardware cursor.
 * When focused, the component should emit CURSOR_MARKER at the cursor position
 * in its render output. TUI will find this marker and position the hardware
 * cursor there for proper IME candidate window positioning.
 */
export interface Focusable {
	/** Set by TUI when focus changes. Component should emit CURSOR_MARKER when true. */
	focused: boolean;
}

/** Type guard to check if a component implements Focusable */
export function isFocusable(component: Component | null): component is Component & Focusable {
	return component !== null && "focused" in component;
}

/**
 * Cursor position marker - APC (Application Program Command) sequence.
 * This is a zero-width escape sequence that terminals ignore.
 * Components emit this at the cursor position when focused.
 * TUI finds and strips this marker, then positions the hardware cursor there.
 */
export const CURSOR_MARKER = "\x1b_pi:c\x07";

/**
 * DECTCEM cursor visibility, as strings rather than Terminal calls so they can
 * be folded into a frame's synchronized-output buffer instead of racing it as a
 * separate write.
 */
const HIDE_CURSOR = "\x1b[?25l";
const SHOW_CURSOR = "\x1b[?25h";

/**
 * How far one wheel notch moves the pinned view.
 *
 * Three lines is what terminals, pagers and browsers have settled on, and the
 * agreement is the point: a wheel that moves a different distance here than in
 * every other window is the kind of wrongness people feel without being able to
 * name it.
 */
const WHEEL_LINES = 3;

/** What the scroll indicator is told about the pinned view. */
export interface ScrollStatus {
	/** 1-based transcript row at the top of the view. */
	top: number;
	/** 1-based transcript row at the bottom of the view. */
	bottom: number;
	/** Rows in the whole transcript. */
	total: number;
	/** Rows the view shows at once. */
	viewHeight: number;
	atTop: boolean;
	/** True only when the very last row is in view — the point where the pin lets go. */
	atBottom: boolean;
	/** Columns the indicator may fill. */
	width: number;
}

export type ScrollStatusFormatter = (status: ScrollStatus) => string;

/**
 * The indicator the tui draws when the app has not supplied its own.
 *
 * Reverse video rather than a colour, because this package has no theme and a
 * hard-coded colour is the one thing guaranteed to clash with whichever one the
 * app is using. It leads with the position — the question a pinned reader
 * actually has — and spends what is left on the keys, dropping them on a narrow
 * terminal rather than truncating the numbers.
 */
function defaultScrollStatus(status: ScrollStatus): string {
	const position = `${status.top}–${status.bottom}/${status.total}`;
	const where = status.atTop ? " top" : "";
	const keys = "↑↓ line · PgUp/PgDn page · esc live";
	const left = ` ${position}${where} `;
	// Measured in columns, not characters: the arrows and the separator are one
	// cell each but a rebind could put anything in here, and a row that is one
	// cell too wide wraps into the window above it.
	const body = visibleWidth(left) + visibleWidth(keys) + 1 <= status.width ? `${left}${keys} ` : left;
	return `\x1b[7m${truncateToWidth(body, status.width, "", true)}\x1b[0m`;
}

/**
 * Anchor position for overlays
 */
export type OverlayAnchor =
	| "center"
	| "top-left"
	| "top-right"
	| "bottom-left"
	| "bottom-right"
	| "top-center"
	| "bottom-center"
	| "left-center"
	| "right-center";

/**
 * Margin configuration for overlays
 */
export interface OverlayMargin {
	top?: number;
	right?: number;
	bottom?: number;
	left?: number;
}

/** Value that can be absolute (number) or percentage (string like "50%") */
export type SizeValue = number | `${number}%`;

/** Parse a SizeValue into absolute value given a reference size */
function parseSizeValue(value: SizeValue | undefined, referenceSize: number): number | undefined {
	if (value === undefined) return undefined;
	if (typeof value === "number") return value;
	// Parse percentage string like "50%"
	const match = value.match(/^(\d+(?:\.\d+)?)%$/);
	if (match) {
		return Math.floor((referenceSize * parseFloat(match[1])) / 100);
	}
	return undefined;
}

function isTermuxSession(): boolean {
	return Boolean(process.env.TERMUX_VERSION);
}

/**
 * Options for overlay positioning and sizing.
 * Values can be absolute numbers or percentage strings (e.g., "50%").
 */
export interface OverlayOptions {
	// === Sizing ===
	/** Width in columns, or percentage of terminal width (e.g., "50%") */
	width?: SizeValue;
	/** Minimum width in columns */
	minWidth?: number;
	/** Maximum height in rows, or percentage of terminal height (e.g., "50%") */
	maxHeight?: SizeValue;

	// === Positioning - anchor-based ===
	/** Anchor point for positioning (default: 'center') */
	anchor?: OverlayAnchor;
	/** Horizontal offset from anchor position (positive = right) */
	offsetX?: number;
	/** Vertical offset from anchor position (positive = down) */
	offsetY?: number;

	// === Positioning - percentage or absolute ===
	/** Row position: absolute number, or percentage (e.g., "25%" = 25% from top) */
	row?: SizeValue;
	/** Column position: absolute number, or percentage (e.g., "50%" = centered horizontally) */
	col?: SizeValue;

	// === Margin from terminal edges ===
	/** Margin from terminal edges. Number applies to all sides. */
	margin?: OverlayMargin | number;

	// === Visibility ===
	/**
	 * Control overlay visibility based on terminal dimensions.
	 * If provided, overlay is only rendered when this returns true.
	 * Called each render cycle with current terminal dimensions.
	 */
	visible?: (termWidth: number, termHeight: number) => boolean;
	/** If true, don't capture keyboard focus when shown */
	nonCapturing?: boolean;
}

/**
 * Handle returned by showOverlay for controlling the overlay
 */
export interface OverlayHandle {
	/** Permanently remove the overlay (cannot be shown again) */
	hide(): void;
	/** Temporarily hide or show the overlay */
	setHidden(hidden: boolean): void;
	/** Check if overlay is temporarily hidden */
	isHidden(): boolean;
	/** Focus this overlay and bring it to the visual front */
	focus(): void;
	/** Release focus to the previous target */
	unfocus(): void;
	/** Check if this overlay currently has focus */
	isFocused(): boolean;
}

/**
 * Container - a component that contains other components
 */
export class Container implements Component {
	children: Component[] = [];
	// Flatten memo: children are always render()ed (side effects and their own
	// caches must run), but when every child returns the same array reference as
	// last time, the previously flattened array is returned as-is. Unchanged
	// subtrees thus stay reference-stable all the way up, which lets the TUI
	// root diff whole regions by identity instead of re-flattening the world.
	private renderMemo?: { width: number; refs: string[][]; lines: string[] };

	addChild(component: Component): void {
		this.children.push(component);
		this.renderMemo = undefined;
	}

	removeChild(component: Component): void {
		const index = this.children.indexOf(component);
		if (index !== -1) {
			this.children.splice(index, 1);
			this.renderMemo = undefined;
		}
	}

	clear(): void {
		this.children = [];
		this.renderMemo = undefined;
	}

	invalidate(): void {
		this.renderMemo = undefined;
		for (const child of this.children) {
			child.invalidate?.();
		}
	}

	render(width: number): string[] {
		const n = this.children.length;
		const memo = this.renderMemo;
		const refs: string[][] = new Array(n);
		let unchanged = memo !== undefined && memo.width === width && memo.refs.length === n;
		for (let i = 0; i < n; i++) {
			refs[i] = this.children[i].render(width);
			if (unchanged && refs[i] !== (memo as { refs: string[][] }).refs[i]) {
				unchanged = false;
			}
		}
		if (unchanged) {
			return (memo as { lines: string[] }).lines;
		}
		const lines: string[] = [];
		for (const childLines of refs) {
			for (const line of childLines) {
				lines.push(line);
			}
		}
		this.renderMemo = { width, refs, lines };
		return lines;
	}
}

/**
 * A child that can be taken off screen without disturbing the diff.
 *
 * Hiding a component naively — returning `[]` from its render — is one of the
 * more expensive things you can do to this renderer. `Container.render` and the
 * root's flat cache both decide "did this subtree change" by **array identity**,
 * so a fresh `[]` every frame reads as a change every frame: the memo is
 * dropped, the buffer is re-flattened, and a dirty range is reported for a
 * component that is not even drawn. One frozen array, returned every time,
 * makes a hidden slot free instead.
 *
 * The child is not rendered at all while hidden, which is the other half of the
 * saving — a hidden footer costs nothing to keep hidden. That means a child
 * whose `render` advances an animation or maintains a cache will be paused, not
 * merely invisible; it catches up when shown again. Chrome (footers, panels,
 * status rows) is fine with that. A spinner is not, so do not wrap one.
 */
export class Slot implements Component {
	/** Shared across every hidden slot: identity is all the caches compare. */
	private static readonly EMPTY: string[] = Object.freeze([]) as unknown as string[];
	private hidden = false;

	constructor(private component: Component) {}

	/** Whoever is in the slot right now. */
	get child(): Component {
		return this.component;
	}

	/**
	 * Swap the occupant, keeping the slot itself in place.
	 *
	 * An extension replacing the footer used to remove one root child and append
	 * another, which both moved the footer to the end of the tree — behind the
	 * widgets meant to sit below it — and handed the root's per-child cache a
	 * changed child list every time. The slot is the stable root child; only what
	 * is inside it changes.
	 */
	setChild(component: Component): boolean {
		if (this.component === component) return false;
		this.component = component;
		return true;
	}

	get visible(): boolean {
		return !this.hidden;
	}

	/** Returns whether this changed anything, so callers can skip a render. */
	setVisible(visible: boolean): boolean {
		const hidden = !visible;
		if (this.hidden === hidden) return false;
		this.hidden = hidden;
		return true;
	}

	invalidate(): void {
		this.child.invalidate?.();
	}

	render(width: number): string[] {
		if (this.hidden) return Slot.EMPTY;
		return this.child.render(width);
	}
}

/**
 * TUI - Main class for managing terminal UI with differential rendering
 */
export class TUI extends Container {
	public terminal: Terminal;
	private previousLines: string[] = [];
	// Root flat-line cache (see the render() override): per-child line arrays,
	// their offsets into the flat buffer, and the flat buffer itself. Active
	// only when no overlays are up and no image has been drawn; otherwise the
	// legacy full-flatten + full-diff path runs.
	private flatCache?: { width: number; refs: string[][]; offsets: number[] };
	private flatLines?: string[];
	/** What the last render() call changed: "full" = unknown (legacy diff must
	 * scan), null = nothing, otherwise the dirty row range + previous length. */
	private lastPatch: { low: number; high: number; prevLength: number } | null | "full" = "full";
	/** Cursor position extracted on the last frame; reused when the dirty range
	 * shows the marker's row untouched (the marker was already stripped). */
	private lastCursorPos: { row: number; col: number } | null | undefined = undefined;
	/** Set by render() when this frame's patches invalidate lastCursorPos: the
	 * marker's row was overwritten, or a patched-in line carries a marker. */
	private cursorRowOverwritten = false;
	private previousKittyImageIds = new Set<number>();
	/** Flips true the first time an image line is emitted. While false no image
	 * has ever been drawn, so there are no kitty ids on screen to track and the
	 * per-frame full-buffer scan (collectKittyImageIds) is skipped entirely —
	 * the common case for a pure-text session. */
	private sawImageLine = false;
	private static readonly EMPTY_KITTY_IDS: ReadonlySet<number> = new Set<number>();
	private previousWidth = 0;
	private previousHeight = 0;
	private focusedComponent: Component | null = null;
	private inputListeners = new Set<InputListener>();

	/** Global callback for debug key (Shift+Ctrl+D). Called before input is forwarded to focused component. */
	public onDebug?: () => void;
	private renderRequested = false;
	private renderTimer: NodeJS.Timeout | undefined;
	private lastRenderAt = 0;
	private static readonly MIN_RENDER_INTERVAL_MS = 16;
	private cursorRow = 0; // Logical cursor row (end of rendered content)
	private hardwareCursorRow = 0; // Actual terminal cursor row (may differ due to IME positioning)
	private showHardwareCursor = process.env.HOOCODE_HARDWARE_CURSOR === "1";
	private clearOnShrink = process.env.HOOCODE_CLEAR_ON_SHRINK === "1"; // Clear empty rows when content shrinks (default: off)
	private maxLinesRendered = 0; // Track terminal's working area (max lines ever rendered)
	private previousViewportTop = 0; // Track previous viewport top for resize-aware cursor moves
	private fullRedrawCount = 0;
	private stopped = false;

	/**
	 * The pinned viewport.
	 *
	 * ## What is wrong with letting the terminal do it
	 *
	 * This renderer keeps the entire transcript in its line buffer and writes it
	 * to the normal screen, so "scrolling" has always meant the terminal's own
	 * scrollback. That works exactly as long as the app does not repaint — and
	 * this one repaints the whole buffer whenever a line *above* the viewport
	 * changes, because a positional diff cannot address a row that has scrolled
	 * out of reach. The repaint is `\x1b[2J\x1b[H\x1b[3J` followed by the
	 * transcript again, and the `\x1b[3J` throws away the scrollback the reader
	 * was sitting in. From the reader's side the screen simply jumps to the
	 * bottom, for no reason they can see, at a moment they did not choose.
	 *
	 * ## What this does instead
	 *
	 * `scrollOffset` is the transcript row drawn at the top of the screen, and
	 * `null` means "follow the tail", which is the normal live behaviour and the
	 * path everything else in this file was written for. The moment it is a
	 * number the TUI switches to the alternate screen and paints a window of the
	 * buffer itself: a fixed grid, addressed row by row, with no scrollback for
	 * anything to fight over. New output still arrives and still lands in the
	 * buffer — it just does not move the window, which is the whole point. The
	 * indicator on the last row says how far down the transcript the window is,
	 * because a view that cannot move on its own needs to say where it stopped.
	 *
	 * Going back to live leaves the alternate screen, which restores the normal
	 * screen *and its scrollback* exactly as they were, and the next frame is an
	 * ordinary differential one that writes only what arrived while the reader
	 * was away — see `scrollToLive` for what has to be true for that to be safe.
	 * Not a clear-and-replay: on a long session that is a visible flash, a burst
	 * of output, and the loss of the scrollback that had just been handed back.
	 */
	private scrollOffset: number | null = null;
	/** Transcript length measured by the last pinned paint; what clamping uses. */
	private scrollTotalLines = 0;
	private scrollStatusFormatter: ScrollStatusFormatter = defaultScrollStatus;
	/**
	 * Whether the view may pin right now.
	 *
	 * The wheel is answered wherever it is turned, including with a picker on
	 * screen — and a picker that lost its arrow keys to a pinned view it did not
	 * know about would be far worse than a wheel that did nothing. The app sets
	 * this because only the app knows which of its surfaces is asking a question.
	 * Unset means always.
	 */
	public canPinScroll?: () => boolean;

	// Overlay stack for modal components rendered on top of base content
	private focusOrderCounter = 0;
	private overlayStack: {
		component: Component;
		options?: OverlayOptions;
		preFocus: Component | null;
		hidden: boolean;
		focusOrder: number;
	}[] = [];

	constructor(terminal: Terminal, showHardwareCursor?: boolean) {
		super();
		this.terminal = terminal;
		if (showHardwareCursor !== undefined) {
			this.showHardwareCursor = showHardwareCursor;
		}
	}

	get fullRedraws(): number {
		return this.fullRedrawCount;
	}

	// ── The pinned viewport ─────────────────────────────────────────────────

	/** True while the view is pinned rather than following the tail. */
	get scrollPinned(): boolean {
		return this.scrollOffset !== null;
	}

	/** Where the pinned window sits, or null while live. */
	getScrollPosition(): { top: number; total: number; viewHeight: number } | null {
		if (this.scrollOffset === null) return null;
		return { top: this.scrollOffset, total: this.scrollTotalLines, viewHeight: this.scrollViewHeight() };
	}

	/** Let the app paint the indicator in its own theme. */
	setScrollStatusFormatter(formatter: ScrollStatusFormatter): void {
		this.scrollStatusFormatter = formatter;
	}

	/**
	 * The screen rows a pinned window shows, the last one being the indicator.
	 *
	 * The indicator is not optional: a pinned view looks exactly like a live one
	 * that has gone quiet, and a reader who cannot tell the two apart will wait
	 * for output that is arriving perfectly well just out of sight.
	 */
	private scrollViewHeight(): number {
		return Math.max(1, this.terminal.rows - 1);
	}

	/** Rows available to scroll through — the live buffer while live, the
	 * measured one while pinned. */
	private transcriptLength(): number {
		return this.scrollOffset === null ? this.previousLines.length : this.scrollTotalLines;
	}

	/**
	 * Move the view by `delta` rows; negative is towards the start.
	 *
	 * Returns whether anything moved, so a caller can let the key fall through
	 * to whatever else wants it when there is nothing to scroll.
	 */
	scrollByLines(delta: number): boolean {
		if (delta === 0) return false;
		// Scrolling down while already live is not "scroll to somewhere", it is a
		// request for content that does not exist yet. Doing nothing is right, and
		// cheap: treating it as a move would drop out of scroll mode and force a
		// full repaint on every wheel notch at the bottom of the transcript.
		if (this.scrollOffset === null && delta > 0) return false;

		const viewHeight = this.scrollViewHeight();
		const maxOffset = Math.max(0, this.transcriptLength() - viewHeight);
		if (maxOffset === 0) return false;

		const next = (this.scrollOffset ?? maxOffset) + delta;
		// Reaching the end is how the pin lets go: the reader has caught up, so
		// give them the live screen back rather than a pinned view of the tail
		// that silently stops following.
		if (next >= maxOffset) return this.scrollToLive();
		this.setScrollOffset(next);
		return true;
	}

	/**
	 * Move by pages, keeping two rows of overlap.
	 *
	 * A page that moves a full screen leaves nothing in common between before
	 * and after, and the reader has to find their place again on every press.
	 * The two kept rows are what makes the jump readable.
	 */
	scrollByPages(delta: number): boolean {
		const page = Math.max(1, this.scrollViewHeight() - 2);
		return this.scrollByLines(delta * page);
	}

	/** Pin the view to the very start of the transcript. */
	scrollToTop(): boolean {
		const maxOffset = Math.max(0, this.transcriptLength() - this.scrollViewHeight());
		if (maxOffset === 0) return false;
		if (this.scrollOffset === 0) return false;
		this.setScrollOffset(0);
		return true;
	}

	/** Release the pin and follow the tail again. */
	scrollToLive(): boolean {
		if (this.scrollOffset === null) return false;
		this.scrollOffset = null;
		this.terminal.setAlternateScreen(false);
		// `?1049l` restores the normal screen, its scrollback and the cursor
		// exactly as they were at `?1049h`, and the snapshot taken on the way in
		// says what that screen holds — so the next frame can be an ordinary
		// differential one that writes only what arrived while we were reading.
		// Dropping the flat cache is what makes it honest: the cache has been
		// patched on every pinned frame, and a patch report describing rows that
		// were painted to the *alternate* screen would leave the diff addressing
		// the wrong ones.
		this.flatCache = undefined;
		this.lastCursorPos = undefined;
		this.requestRender();
		return true;
	}

	private setScrollOffset(offset: number): void {
		const entering = this.scrollOffset === null;
		// Only entry is gated. A view that is already pinned keeps responding, so
		// a surface opening underneath cannot strand the reader somewhere they
		// have no key to leave.
		if (entering && this.canPinScroll && !this.canPinScroll()) return;
		this.scrollOffset = Math.max(0, offset);
		if (entering) {
			// What the normal screen is left showing, frozen. Without the copy this
			// stays the same array the patching render() mutates in place, so on the
			// way back out it would be diffed against itself and report that nothing
			// arrived while the reader was away.
			this.previousLines = this.previousLines.slice();
			this.terminal.setAlternateScreen(true);
			this.terminal.hideCursor();
		}
		this.requestRender();
		// Scrolling is a direct manipulation: the view has to move under the
		// gesture, not one animation frame behind it.
		this.expediteRender();
	}

	getShowHardwareCursor(): boolean {
		return this.showHardwareCursor;
	}

	setShowHardwareCursor(enabled: boolean): void {
		if (this.showHardwareCursor === enabled) return;
		this.showHardwareCursor = enabled;
		if (!enabled) {
			this.terminal.hideCursor();
		}
		this.requestRender();
	}

	getClearOnShrink(): boolean {
		return this.clearOnShrink;
	}

	/**
	 * Set whether to trigger full re-render when content shrinks.
	 * When true (default), empty rows are cleared when content shrinks.
	 * When false, empty rows remain (reduces redraws on slower terminals).
	 */
	setClearOnShrink(enabled: boolean): void {
		this.clearOnShrink = enabled;
	}

	/** The component keystrokes are currently going to. */
	get focused(): Component | null {
		return this.focusedComponent;
	}

	setFocus(component: Component | null): void {
		// Clear focused flag on old component
		if (isFocusable(this.focusedComponent)) {
			this.focusedComponent.focused = false;
		}

		this.focusedComponent = component;

		// Set focused flag on new component
		if (isFocusable(component)) {
			component.focused = true;
		}
	}

	/**
	 * Show an overlay component with configurable positioning and sizing.
	 * Returns a handle to control the overlay's visibility.
	 */
	showOverlay(component: Component, options?: OverlayOptions): OverlayHandle {
		const entry = {
			component,
			options,
			preFocus: this.focusedComponent,
			hidden: false,
			focusOrder: ++this.focusOrderCounter,
		};
		this.overlayStack.push(entry);
		// Only focus if overlay is actually visible
		if (!options?.nonCapturing && this.isOverlayVisible(entry)) {
			this.setFocus(component);
		}
		this.terminal.hideCursor();
		this.requestRender();

		// Return handle for controlling this overlay
		return {
			hide: () => {
				const index = this.overlayStack.indexOf(entry);
				if (index !== -1) {
					this.overlayStack.splice(index, 1);
					// Restore focus if this overlay had focus
					if (this.focusedComponent === component) {
						const topVisible = this.getTopmostVisibleOverlay();
						this.setFocus(topVisible?.component ?? entry.preFocus);
					}
					if (this.overlayStack.length === 0) this.terminal.hideCursor();
					this.requestRender();
				}
			},
			setHidden: (hidden: boolean) => {
				if (entry.hidden === hidden) return;
				entry.hidden = hidden;
				// Update focus when hiding/showing
				if (hidden) {
					// If this overlay had focus, move focus to next visible or preFocus
					if (this.focusedComponent === component) {
						const topVisible = this.getTopmostVisibleOverlay();
						this.setFocus(topVisible?.component ?? entry.preFocus);
					}
				} else {
					// Restore focus to this overlay when showing (if it's actually visible)
					if (!options?.nonCapturing && this.isOverlayVisible(entry)) {
						entry.focusOrder = ++this.focusOrderCounter;
						this.setFocus(component);
					}
				}
				this.requestRender();
			},
			isHidden: () => entry.hidden,
			focus: () => {
				if (!this.overlayStack.includes(entry) || !this.isOverlayVisible(entry)) return;
				if (this.focusedComponent !== component) {
					this.setFocus(component);
				}
				entry.focusOrder = ++this.focusOrderCounter;
				this.requestRender();
			},
			unfocus: () => {
				if (this.focusedComponent !== component) return;
				const topVisible = this.getTopmostVisibleOverlay();
				this.setFocus(topVisible && topVisible !== entry ? topVisible.component : entry.preFocus);
				this.requestRender();
			},
			isFocused: () => this.focusedComponent === component,
		};
	}

	/** Hide the topmost overlay and restore previous focus. */
	hideOverlay(): void {
		const overlay = this.overlayStack.pop();
		if (!overlay) return;
		if (this.focusedComponent === overlay.component) {
			// Find topmost visible overlay, or fall back to preFocus
			const topVisible = this.getTopmostVisibleOverlay();
			this.setFocus(topVisible?.component ?? overlay.preFocus);
		}
		if (this.overlayStack.length === 0) this.terminal.hideCursor();
		this.requestRender();
	}

	/** Check if there are any visible overlays */
	hasOverlay(): boolean {
		return this.overlayStack.some((o) => this.isOverlayVisible(o));
	}

	/** Check if an overlay entry is currently visible */
	private isOverlayVisible(entry: (typeof this.overlayStack)[number]): boolean {
		if (entry.hidden) return false;
		if (entry.options?.visible) {
			return entry.options.visible(this.terminal.columns, this.terminal.rows);
		}
		return true;
	}

	/** Find the topmost visible capturing overlay, if any */
	private getTopmostVisibleOverlay(): (typeof this.overlayStack)[number] | undefined {
		for (let i = this.overlayStack.length - 1; i >= 0; i--) {
			if (this.overlayStack[i].options?.nonCapturing) continue;
			if (this.isOverlayVisible(this.overlayStack[i])) {
				return this.overlayStack[i];
			}
		}
		return undefined;
	}

	override invalidate(): void {
		super.invalidate();
		for (const overlay of this.overlayStack) overlay.component.invalidate?.();
	}

	start(): void {
		this.stopped = false;
		this.terminal.start(
			(data) => this.handleInput(data),
			() => this.requestRender(),
		);
		this.terminal.hideCursor();
		this.queryCellSize();
		this.requestRender();
	}

	addInputListener(listener: InputListener): () => void {
		this.inputListeners.add(listener);
		return () => {
			this.inputListeners.delete(listener);
		};
	}

	removeInputListener(listener: InputListener): void {
		this.inputListeners.delete(listener);
	}

	private queryCellSize(): void {
		// Only query if terminal supports images (cell size is only used for image rendering)
		if (!getCapabilities().images) {
			return;
		}
		// Query terminal for cell size in pixels: CSI 16 t
		// Response format: CSI 6 ; height ; width t
		this.terminal.write("\x1b[16t");
	}

	stop(): void {
		// Off the alternate screen before the exit bookkeeping below, which moves
		// the cursor relative to content that lives on the normal screen.
		if (this.scrollOffset !== null) {
			this.scrollOffset = null;
			this.terminal.setAlternateScreen(false);
		}
		this.stopped = true;
		if (this.renderTimer) {
			clearTimeout(this.renderTimer);
			this.renderTimer = undefined;
		}
		// Move cursor to the end of the content to prevent overwriting/artifacts on exit
		if (this.previousLines.length > 0) {
			const targetRow = this.previousLines.length; // Line after the last content
			const lineDiff = targetRow - this.hardwareCursorRow;
			if (lineDiff > 0) {
				this.terminal.write(`\x1b[${lineDiff}B`);
			} else if (lineDiff < 0) {
				this.terminal.write(`\x1b[${-lineDiff}A`);
			}
			this.terminal.write("\r\n");
		}

		this.terminal.showCursor();
		this.terminal.stop();
	}

	requestRender(force = false): void {
		if (force) {
			this.previousLines = [];
			this.previousWidth = -1; // -1 triggers widthChanged, forcing a full clear
			this.previousHeight = -1; // -1 triggers heightChanged, forcing a full clear
			this.cursorRow = 0;
			this.hardwareCursorRow = 0;
			this.maxLinesRendered = 0;
			this.previousViewportTop = 0;
			if (this.renderTimer) {
				clearTimeout(this.renderTimer);
				this.renderTimer = undefined;
			}
			this.renderRequested = true;
			process.nextTick(() => {
				if (this.stopped || !this.renderRequested) {
					return;
				}
				this.renderRequested = false;
				this.lastRenderAt = performance.now();
				this.doRender();
			});
			return;
		}
		if (this.renderRequested) return;
		this.renderRequested = true;
		process.nextTick(() => this.scheduleRender());
	}

	private scheduleRender(): void {
		if (this.stopped || this.renderTimer || !this.renderRequested) {
			return;
		}
		const elapsed = performance.now() - this.lastRenderAt;
		const delay = Math.max(0, TUI.MIN_RENDER_INTERVAL_MS - elapsed);
		this.renderTimer = setTimeout(() => {
			this.renderTimer = undefined;
			if (this.stopped || !this.renderRequested) {
				return;
			}
			this.renderRequested = false;
			this.lastRenderAt = performance.now();
			this.doRender();
			if (this.renderRequested) {
				this.scheduleRender();
			}
		}, delay);
	}

	private handleInput(data: string): void {
		// Ahead of the listeners: a mouse report that reaches a text field is
		// typed into it, and a paste-detecting listener has no reason to see one.
		if (this.terminal.mouseReporting) {
			const remaining = this.consumeMouseReports(data);
			if (remaining === null) return;
			data = remaining;
		}

		if (this.inputListeners.size > 0) {
			let current = data;
			for (const listener of this.inputListeners) {
				const result = listener(current);
				if (result?.consume) {
					return;
				}
				if (result?.data !== undefined) {
					current = result.data;
				}
			}
			if (current.length === 0) {
				return;
			}
			data = current;
		}

		// Consume terminal cell size responses without blocking unrelated input.
		if (this.consumeCellSizeResponse(data)) {
			return;
		}

		// Global debug key handler (Shift+Ctrl+D)
		if (matchesKey(data, "shift+ctrl+d") && this.onDebug) {
			this.onDebug();
			return;
		}

		// If focused component is an overlay, verify it's still visible
		// (visibility can change due to terminal resize or visible() callback)
		const focusedOverlay = this.overlayStack.find((o) => o.component === this.focusedComponent);
		if (focusedOverlay && !this.isOverlayVisible(focusedOverlay)) {
			// Focused overlay is no longer visible, redirect to topmost visible overlay
			const topVisible = this.getTopmostVisibleOverlay();
			if (topVisible) {
				this.setFocus(topVisible.component);
			} else {
				// No visible overlays, restore to preFocus
				this.setFocus(focusedOverlay.preFocus);
			}
		}

		// Pass input to focused component (including Ctrl+C)
		// The focused component can decide how to handle Ctrl+C
		if (this.focusedComponent?.handleInput) {
			// Filter out key release events unless component opts in
			if (isKeyRelease(data) && !this.focusedComponent.wantsKeyRelease) {
				return;
			}
			this.focusedComponent.handleInput(data);
			this.requestRender();
			// Keystroke echo should not queue behind the animation coalescing
			// window: render the input's effect immediately instead of waiting out
			// MIN_RENDER_INTERVAL_MS behind spinner/streaming frames.
			this.expediteRender();
		}
	}

	/**
	 * Act on every mouse report in `data` and return what is left of it.
	 *
	 * Returns null when the chunk was nothing but reports. Reports arrive
	 * coalesced — a flick of the wheel delivers a run of them in one read, and a
	 * keystroke pressed during the flick rides along behind — so they are peeled
	 * off one at a time instead of the chunk being classified as a whole.
	 */
	private consumeMouseReports(data: string): string | null {
		// Neither introducer present is the overwhelmingly common case (every
		// ordinary keystroke), and it costs one scan of a very short string.
		if (!data.includes("\x1b[<") && !data.includes("\x1b[M")) return data;

		let rest = data;
		let out = "";
		let sawReport = false;
		while (rest.length > 0) {
			const length = mouseSequenceLength(rest);
			if (length === 0) {
				out += rest[0];
				rest = rest.slice(1);
				continue;
			}
			const event = parseMouseEvent(rest.slice(0, length));
			if (event) this.handleMouseEvent(event);
			sawReport = true;
			rest = rest.slice(length);
		}

		if (!sawReport) return data;
		return out.length > 0 ? out : null;
	}

	/**
	 * What the mouse does.
	 *
	 * Only the wheel is acted on. Clicks are swallowed rather than handled:
	 * reporting is on for the wheel's sake, and a click that fell through to the
	 * focused component would arrive as the raw report text in whatever field
	 * has focus.
	 */
	private handleMouseEvent(event: MouseEvent): void {
		if (event.kind === "wheelUp") {
			this.scrollByLines(-WHEEL_LINES);
			return;
		}
		if (event.kind === "wheelDown") {
			this.scrollByLines(WHEEL_LINES);
		}
	}

	/** Run a requested render now, bypassing the coalescing delay. Used for
	 * input-driven frames where echo latency matters more than batching. */
	private expediteRender(): void {
		if (this.stopped || !this.renderRequested) return;
		if (this.renderTimer) {
			clearTimeout(this.renderTimer);
			this.renderTimer = undefined;
		}
		this.renderRequested = false;
		this.lastRenderAt = performance.now();
		this.doRender();
	}

	private consumeCellSizeResponse(data: string): boolean {
		// Response format: ESC [ 6 ; height ; width t
		const match = data.match(/^\x1b\[6;(\d+);(\d+)t$/);
		if (!match) {
			return false;
		}

		const heightPx = parseInt(match[1], 10);
		const widthPx = parseInt(match[2], 10);
		if (heightPx <= 0 || widthPx <= 0) {
			return true;
		}

		setCellDimensions({ widthPx, heightPx });
		// Invalidate all components so images re-render with correct dimensions.
		this.invalidate();
		this.requestRender();
		return true;
	}

	/**
	 * Resolve overlay layout from options.
	 * Returns { width, row, col, maxHeight } for rendering.
	 */
	private resolveOverlayLayout(
		options: OverlayOptions | undefined,
		overlayHeight: number,
		termWidth: number,
		termHeight: number,
	): { width: number; row: number; col: number; maxHeight: number | undefined } {
		const opt = options ?? {};

		// Parse margin (clamp to non-negative)
		const margin =
			typeof opt.margin === "number"
				? { top: opt.margin, right: opt.margin, bottom: opt.margin, left: opt.margin }
				: (opt.margin ?? {});
		const marginTop = Math.max(0, margin.top ?? 0);
		const marginRight = Math.max(0, margin.right ?? 0);
		const marginBottom = Math.max(0, margin.bottom ?? 0);
		const marginLeft = Math.max(0, margin.left ?? 0);

		// Available space after margins
		const availWidth = Math.max(1, termWidth - marginLeft - marginRight);
		const availHeight = Math.max(1, termHeight - marginTop - marginBottom);

		// === Resolve width ===
		let width = parseSizeValue(opt.width, termWidth) ?? Math.min(80, availWidth);
		// Apply minWidth
		if (opt.minWidth !== undefined) {
			width = Math.max(width, opt.minWidth);
		}
		// Clamp to available space
		width = Math.max(1, Math.min(width, availWidth));

		// === Resolve maxHeight ===
		let maxHeight = parseSizeValue(opt.maxHeight, termHeight);
		// Clamp to available space
		if (maxHeight !== undefined) {
			maxHeight = Math.max(1, Math.min(maxHeight, availHeight));
		}

		// Effective overlay height (may be clamped by maxHeight)
		const effectiveHeight = maxHeight !== undefined ? Math.min(overlayHeight, maxHeight) : overlayHeight;

		// === Resolve position ===
		let row: number;
		let col: number;

		if (opt.row !== undefined) {
			if (typeof opt.row === "string") {
				// Percentage: 0% = top, 100% = bottom (overlay stays within bounds)
				const match = opt.row.match(/^(\d+(?:\.\d+)?)%$/);
				if (match) {
					const maxRow = Math.max(0, availHeight - effectiveHeight);
					const percent = parseFloat(match[1]) / 100;
					row = marginTop + Math.floor(maxRow * percent);
				} else {
					// Invalid format, fall back to center
					row = this.resolveAnchorRow("center", effectiveHeight, availHeight, marginTop);
				}
			} else {
				// Absolute row position
				row = opt.row;
			}
		} else {
			// Anchor-based (default: center)
			const anchor = opt.anchor ?? "center";
			row = this.resolveAnchorRow(anchor, effectiveHeight, availHeight, marginTop);
		}

		if (opt.col !== undefined) {
			if (typeof opt.col === "string") {
				// Percentage: 0% = left, 100% = right (overlay stays within bounds)
				const match = opt.col.match(/^(\d+(?:\.\d+)?)%$/);
				if (match) {
					const maxCol = Math.max(0, availWidth - width);
					const percent = parseFloat(match[1]) / 100;
					col = marginLeft + Math.floor(maxCol * percent);
				} else {
					// Invalid format, fall back to center
					col = this.resolveAnchorCol("center", width, availWidth, marginLeft);
				}
			} else {
				// Absolute column position
				col = opt.col;
			}
		} else {
			// Anchor-based (default: center)
			const anchor = opt.anchor ?? "center";
			col = this.resolveAnchorCol(anchor, width, availWidth, marginLeft);
		}

		// Apply offsets
		if (opt.offsetY !== undefined) row += opt.offsetY;
		if (opt.offsetX !== undefined) col += opt.offsetX;

		// Clamp to terminal bounds (respecting margins)
		row = Math.max(marginTop, Math.min(row, termHeight - marginBottom - effectiveHeight));
		col = Math.max(marginLeft, Math.min(col, termWidth - marginRight - width));

		return { width, row, col, maxHeight };
	}

	private resolveAnchorRow(anchor: OverlayAnchor, height: number, availHeight: number, marginTop: number): number {
		switch (anchor) {
			case "top-left":
			case "top-center":
			case "top-right":
				return marginTop;
			case "bottom-left":
			case "bottom-center":
			case "bottom-right":
				return marginTop + availHeight - height;
			case "left-center":
			case "center":
			case "right-center":
				return marginTop + Math.floor((availHeight - height) / 2);
		}
	}

	private resolveAnchorCol(anchor: OverlayAnchor, width: number, availWidth: number, marginLeft: number): number {
		switch (anchor) {
			case "top-left":
			case "left-center":
			case "bottom-left":
				return marginLeft;
			case "top-right":
			case "right-center":
			case "bottom-right":
				return marginLeft + availWidth - width;
			case "top-center":
			case "center":
			case "bottom-center":
				return marginLeft + Math.floor((availWidth - width) / 2);
		}
	}

	/** Composite all overlays into content lines (sorted by focusOrder, higher = on top). */
	private compositeOverlays(lines: string[], termWidth: number, termHeight: number): string[] {
		if (this.overlayStack.length === 0) return lines;
		const result = [...lines];

		// Pre-render all visible overlays and calculate positions
		const rendered: { overlayLines: string[]; row: number; col: number; w: number }[] = [];
		let minLinesNeeded = result.length;

		const visibleEntries = this.overlayStack.filter((e) => this.isOverlayVisible(e));
		visibleEntries.sort((a, b) => a.focusOrder - b.focusOrder);
		for (const entry of visibleEntries) {
			const { component, options } = entry;

			// Get layout with height=0 first to determine width and maxHeight
			// (width and maxHeight don't depend on overlay height)
			const { width, maxHeight } = this.resolveOverlayLayout(options, 0, termWidth, termHeight);

			// Render component at calculated width
			let overlayLines = component.render(width);

			// Apply maxHeight if specified
			if (maxHeight !== undefined && overlayLines.length > maxHeight) {
				overlayLines = overlayLines.slice(0, maxHeight);
			}

			// Get final row/col with actual overlay height
			const { row, col } = this.resolveOverlayLayout(options, overlayLines.length, termWidth, termHeight);

			rendered.push({ overlayLines, row, col, w: width });
			minLinesNeeded = Math.max(minLinesNeeded, row + overlayLines.length);
		}

		// Pad to at least terminal height so overlays have screen-relative positions.
		// Excludes maxLinesRendered: the historical high-water mark caused self-reinforcing
		// inflation that pushed content into scrollback on terminal widen.
		const workingHeight = Math.max(result.length, termHeight, minLinesNeeded);

		// Extend result with empty lines if content is too short for overlay placement or working area
		while (result.length < workingHeight) {
			result.push("");
		}

		const viewportStart = Math.max(0, workingHeight - termHeight);

		// Composite each overlay
		for (const { overlayLines, row, col, w } of rendered) {
			for (let i = 0; i < overlayLines.length; i++) {
				const idx = viewportStart + row + i;
				if (idx >= 0 && idx < result.length) {
					// Defensive: truncate overlay line to declared width before compositing
					// (components should already respect width, but this ensures it)
					const truncatedOverlayLine =
						visibleWidth(overlayLines[i]) > w ? sliceByColumn(overlayLines[i], 0, w, true) : overlayLines[i];
					result[idx] = this.compositeLineAt(result[idx], truncatedOverlayLine, col, w, termWidth);
				}
			}
		}

		return result;
	}

	private static readonly SEGMENT_RESET = "\x1b[0m\x1b]8;;\x07";

	/**
	 * Append the per-line style/hyperlink reset (and normalize Thai/Lao AM
	 * vowels) at the moment a line is written to the terminal. This is
	 * deliberately kept OFF the cached/diffed line arrays: leaf components cache
	 * their lines without the reset, so leaving `newLines`/`previousLines`
	 * un-reset keeps unchanged lines reference-stable frame to frame. The
	 * differential compare then short-circuits on identity for every unchanged
	 * line instead of allocating a fresh reset-appended string per line and
	 * doing a full content compare across the whole transcript every frame.
	 * Image lines carry no trailing style and are emitted verbatim.
	 */
	private emitLine(line: string): string {
		if (isImageLine(line)) {
			this.sawImageLine = true;
			return line;
		}
		return normalizeTerminalOutput(line) + TUI.SEGMENT_RESET;
	}

	private collectKittyImageIds(lines: string[]): Set<number> {
		// No image has ever been drawn: nothing on screen carries a kitty id, so
		// skip the full-buffer scan and the Set allocation.
		if (!this.sawImageLine) return TUI.EMPTY_KITTY_IDS as Set<number>;
		const ids = new Set<number>();
		for (const line of lines) {
			for (const id of extractKittyImageIds(line)) {
				ids.add(id);
			}
		}
		return ids;
	}

	private deleteKittyImages(ids: Iterable<number>): string {
		let buffer = "";
		for (const id of ids) {
			buffer += deleteKittyImage(id);
		}
		return buffer;
	}

	private expandLastChangedForKittyImages(firstChanged: number, lastChanged: number): number {
		// No image ever drawn: nothing to expand over, skip the scan (also,
		// on patched frames previousLines is not the previous content).
		if (!this.sawImageLine) return lastChanged;
		let expandedLastChanged = lastChanged;
		for (let i = firstChanged; i < this.previousLines.length; i++) {
			if (extractKittyImageIds(this.previousLines[i]).length > 0) {
				expandedLastChanged = Math.max(expandedLastChanged, i);
			}
		}
		return expandedLastChanged;
	}

	private deleteChangedKittyImages(firstChanged: number, lastChanged: number): string {
		if (firstChanged < 0 || lastChanged < firstChanged) return "";

		const ids = new Set<number>();
		const maxLine = Math.min(lastChanged, this.previousLines.length - 1);
		for (let i = firstChanged; i <= maxLine; i++) {
			for (const id of extractKittyImageIds(this.previousLines[i] ?? "")) {
				ids.add(id);
			}
		}

		return this.deleteKittyImages(ids);
	}

	/** Splice overlay content into a base line at a specific column. Single-pass optimized. */
	private compositeLineAt(
		baseLine: string,
		overlayLine: string,
		startCol: number,
		overlayWidth: number,
		totalWidth: number,
	): string {
		if (isImageLine(baseLine)) return baseLine;

		// Single pass through baseLine extracts both before and after segments
		const afterStart = startCol + overlayWidth;
		const base = extractSegments(baseLine, startCol, afterStart, totalWidth - afterStart, true);

		// Extract overlay with width tracking (strict=true to exclude wide chars at boundary)
		const overlay = sliceWithWidth(overlayLine, 0, overlayWidth, true);

		// Pad segments to target widths
		const beforePad = Math.max(0, startCol - base.beforeWidth);
		const overlayPad = Math.max(0, overlayWidth - overlay.width);
		const actualBeforeWidth = Math.max(startCol, base.beforeWidth);
		const actualOverlayWidth = Math.max(overlayWidth, overlay.width);
		const afterTarget = Math.max(0, totalWidth - actualBeforeWidth - actualOverlayWidth);
		const afterPad = Math.max(0, afterTarget - base.afterWidth);

		// Compose result
		const r = TUI.SEGMENT_RESET;
		const result =
			base.before +
			" ".repeat(beforePad) +
			r +
			overlay.text +
			" ".repeat(overlayPad) +
			r +
			base.after +
			" ".repeat(afterPad);

		// CRITICAL: Always verify and truncate to terminal width.
		// This is the final safeguard against width overflow which would crash the TUI.
		// Width tracking can drift from actual visible width due to:
		// - Complex ANSI/OSC sequences (hyperlinks, colors)
		// - Wide characters at segment boundaries
		// - Edge cases in segment extraction
		const resultWidth = visibleWidth(result);
		if (resultWidth <= totalWidth) {
			return result;
		}
		// Truncate with strict=true to ensure we don't exceed totalWidth
		return sliceByColumn(result, 0, totalWidth, true);
	}

	/**
	 * Find and extract cursor position from rendered lines.
	 * Searches for CURSOR_MARKER, calculates its position, and strips it from the output.
	 * Only scans the bottom terminal height lines (visible viewport).
	 * @param lines - Rendered lines to search
	 * @param height - Terminal height (visible viewport size)
	 * @returns Cursor position { row, col } or null if no marker found
	 */
	private extractCursorPosition(lines: string[], height: number): { row: number; col: number } | null {
		// Only scan the bottom `height` lines (visible viewport)
		const viewportTop = Math.max(0, lines.length - height);
		for (let row = lines.length - 1; row >= viewportTop; row--) {
			const line = lines[row];
			const markerIndex = line.indexOf(CURSOR_MARKER);
			if (markerIndex !== -1) {
				// Calculate visual column (width of text before marker)
				const beforeMarker = line.slice(0, markerIndex);
				const col = visibleWidth(beforeMarker);

				// Strip marker from the line
				lines[row] = line.slice(0, markerIndex) + line.slice(markerIndex + CURSOR_MARKER.length);

				return { row, col };
			}
		}
		return null;
	}

	/**
	 * Root flatten with patch tracking. Children stay memoized (Container), so
	 * a frame where only one region changed patches that region into the
	 * persistent flat buffer and reports the dirty row range via lastPatch —
	 * doRender then skips the whole-transcript diff. Falls back to a fresh
	 * flatten (lastPatch = "full") when overlays are up, an image has been
	 * drawn (kitty bookkeeping needs true previous content), the width changed,
	 * or the child list changed.
	 */
	override render(width: number): string[] {
		const cacheAllowed = this.overlayStack.length === 0 && !this.sawImageLine;
		const cache = this.flatCache;
		if (!cacheAllowed || !cache || cache.width !== width || cache.refs.length !== this.children.length) {
			const n = this.children.length;
			const refs: string[][] = new Array(n);
			const offsets: number[] = new Array(n);
			const flat: string[] = [];
			for (let i = 0; i < n; i++) {
				refs[i] = this.children[i].render(width);
				offsets[i] = flat.length;
				for (const line of refs[i]) flat.push(line);
			}
			if (cacheAllowed) {
				this.flatCache = { width, refs, offsets };
				this.flatLines = flat;
			} else {
				this.flatCache = undefined;
				this.flatLines = undefined;
			}
			this.lastPatch = "full";
			return flat;
		}

		let flat = this.flatLines as string[];
		const prevLength = flat.length;
		let low = Infinity;
		let high = -1;
		let delta = 0;
		// Cursor bookkeeping: the marker was stripped out of the persistent flat
		// when last extracted, so the cached position stays valid until the row
		// it lives on is overwritten by re-imported child content — and it
		// shifts when content above it grows or shrinks.
		let cp = this.lastCursorPos ?? null;
		this.cursorRowOverwritten = false;
		for (let i = 0; i < this.children.length; i++) {
			const r = this.children[i].render(width);
			const old = cache.refs[i];
			if (r === old) continue;
			const off = cache.offsets[i] + delta;
			if (r.length === old.length) {
				for (let k = 0; k < r.length; k++) {
					if (old[k] !== r[k]) {
						const row = off + k;
						flat[row] = r[k];
						if (row < low) low = row;
						if (row > high) high = row;
						// Overwrote the marker's row, or imported a line carrying a
						// (possibly relocated) marker: position must be re-extracted.
						if (cp && cp.row === row) this.cursorRowOverwritten = true;
						if (r[k].includes(CURSOR_MARKER)) this.cursorRowOverwritten = true;
					}
				}
			} else {
				// Length changed: find the first differing line, then splice the
				// child's new lines in. Everything from there down shifts rows, so
				// the dirty range extends to the end (positional diff semantics).
				let p = 0;
				const minLen = Math.min(old.length, r.length);
				while (p < minLen && old[p] === r[p]) p++;
				flat = flat.slice(0, off + p).concat(r.slice(p), flat.slice(off + old.length));
				if (off + p < low) low = off + p;
				if (cp) {
					if (cp.row >= off + old.length) {
						// Below the replaced region: shifts with it.
						cp = { row: cp.row + (r.length - old.length), col: cp.col };
					} else if (cp.row >= off + p) {
						// Inside the replaced region: fresh content, re-extract.
						this.cursorRowOverwritten = true;
					}
				}
				if (!this.cursorRowOverwritten) {
					for (let k = p; k < r.length; k++) {
						if (r[k].includes(CURSOR_MARKER)) {
							this.cursorRowOverwritten = true;
							break;
						}
					}
				}
				delta += r.length - old.length;
			}
			cache.refs[i] = r;
		}
		this.lastCursorPos = cp;
		const spliced = flat !== this.flatLines;
		if (spliced) {
			let acc = 0;
			for (let i = 0; i < cache.refs.length; i++) {
				cache.offsets[i] = acc;
				acc += cache.refs[i].length;
			}
			this.flatLines = flat;
			// Rows below the first splice all shifted; positional diff semantics
			// mean everything from there to the end must be treated as dirty.
			high = Math.max(prevLength - 1, flat.length - 1);
		}
		this.lastPatch = low === Infinity && high === -1 ? null : { low: low === Infinity ? 0 : low, high, prevLength };
		return flat;
	}

	/**
	 * Paint the pinned window onto the alternate screen.
	 *
	 * Deliberately not differential. The alternate screen is `rows` tall and
	 * nothing else writes to it, so a whole frame is at most a screenful of
	 * cells inside one synchronized-output pair — cheaper to emit than the
	 * bookkeeping a diff would need, and with nothing to get out of step with.
	 * The differential renderer's state is left exactly as the last live frame
	 * left it, because `scrollToLive` throws it away rather than resuming from it.
	 */
	private renderScrollView(): void {
		const width = this.terminal.columns;
		const height = this.terminal.rows;

		let lines = this.render(width);
		// A patch computed while pinned describes rows nothing painted to the
		// normal screen, so the live path must never be handed it.
		this.lastPatch = "full";
		if (this.overlayStack.length > 0) {
			lines = this.compositeOverlays(lines, width, height);
		}

		const viewHeight = this.scrollViewHeight();
		this.scrollTotalLines = lines.length;
		const maxOffset = Math.max(0, lines.length - viewHeight);
		// The transcript can shrink under a pinned view — a pane closing, a tool
		// block collapsing — so the offset is re-clamped every frame rather than
		// only where it is set.
		const top = Math.min(Math.max(0, this.scrollOffset ?? 0), maxOffset);
		this.scrollOffset = top;

		let buffer = "\x1b[?2026h"; // Begin synchronized output
		buffer += HIDE_CURSOR;
		// Autowrap off for the paint: a full-width row would otherwise wrap into
		// the row below it and shift the rest of the window down by one.
		buffer += "\x1b[?7l";

		for (let row = 0; row < viewHeight; row++) {
			buffer += `\x1b[${row + 1};1H\x1b[2K`;
			const line = lines[top + row];
			if (line !== undefined) buffer += this.emitScrollLine(line);
		}

		buffer += `\x1b[${height};1H\x1b[2K`;
		buffer += this.scrollStatusFormatter({
			top: top + 1,
			bottom: Math.min(top + viewHeight, lines.length),
			total: lines.length,
			viewHeight,
			atTop: top === 0,
			atBottom: top >= maxOffset,
			width,
		});

		buffer += "\x1b[?7h";
		buffer += "\x1b[?2026l"; // End synchronized output
		this.terminal.write(buffer);

		// `previousWidth` / `previousHeight` are deliberately left describing the
		// last *live* frame. If the terminal was resized while pinned they will
		// disagree with the real size on the way out, and the live path will take
		// its full-redraw branch — which is exactly right, because the normal
		// screen `?1049l` restored was drawn at the old size.
	}

	/**
	 * One transcript row, ready for the pinned window.
	 *
	 * Images are named rather than drawn. A kitty or iTerm image is placed by
	 * the cursor and sized in pixels, so the same escape replayed at a different
	 * screen row lands somewhere the window did not ask for and survives the
	 * frame that was supposed to replace it — a smear across the view that no
	 * later repaint can clear.
	 */
	private emitScrollLine(line: string): string {
		if (isImageLine(line)) return "\x1b[2m[image]\x1b[0m";
		const marker = line.indexOf(CURSOR_MARKER);
		const text = marker === -1 ? line : line.slice(0, marker) + line.slice(marker + CURSOR_MARKER.length);
		return normalizeTerminalOutput(text) + TUI.SEGMENT_RESET;
	}

	private doRender(): void {
		if (this.stopped) return;
		if (this.scrollOffset !== null) {
			this.renderScrollView();
			return;
		}
		const width = this.terminal.columns;
		const height = this.terminal.rows;
		const widthChanged = this.previousWidth !== 0 && this.previousWidth !== width;
		const heightChanged = this.previousHeight !== 0 && this.previousHeight !== height;
		const previousBufferLength = this.previousHeight > 0 ? this.previousViewportTop + this.previousHeight : height;
		let prevViewportTop = heightChanged ? Math.max(0, previousBufferLength - height) : this.previousViewportTop;
		let viewportTop = prevViewportTop;
		let hardwareCursorRow = this.hardwareCursorRow;
		const computeLineDiff = (targetRow: number): number => {
			const currentScreenRow = hardwareCursorRow - prevViewportTop;
			const targetScreenRow = targetRow - viewportTop;
			return targetScreenRow - currentScreenRow;
		};

		// Render all components to get new lines. The root render() reports what
		// it changed via lastPatch; consume it here (it is per-frame state).
		let newLines = this.render(width);
		const patch = this.lastPatch;
		this.lastPatch = "full";

		// Composite overlays into the rendered lines (before differential compare)
		if (this.overlayStack.length > 0) {
			newLines = this.compositeOverlays(newLines, width, height);
		}

		// Extract cursor position before the marker could be obscured. The reset
		// is applied per-line at write time (see emitLine), so newLines stays the
		// un-reset, reference-stable output of the component tree from here on.
		// On patched frames the persistent flat buffer already had the marker
		// stripped; the cached position (row-shifted by render()) stays valid
		// unless its row was overwritten by re-imported child content, or a
		// marker could have newly appeared in changed content.
		let cursorPos: { row: number; col: number } | null;
		if (patch !== "full" && this.lastCursorPos !== undefined) {
			const cp = this.lastCursorPos;
			if (cp !== null && !this.cursorRowOverwritten) {
				cursorPos = cp;
			} else if (patch === null) {
				cursorPos = cp;
			} else {
				cursorPos = this.extractCursorPosition(newLines, height);
			}
		} else {
			cursorPos = this.extractCursorPosition(newLines, height);
		}
		this.lastCursorPos = cursorPos;

		// Helper to clear scrollback and viewport and render all new lines
		const fullRender = (clear: boolean): void => {
			this.fullRedrawCount += 1;
			let buffer = "\x1b[?2026h"; // Begin synchronized output
			if (clear) {
				buffer += this.deleteKittyImages(this.previousKittyImageIds);
				buffer += "\x1b[2J\x1b[H\x1b[3J"; // Clear screen, home, then clear scrollback
			}
			for (let i = 0; i < newLines.length; i++) {
				if (i > 0) buffer += "\r\n";
				buffer += this.emitLine(newLines[i]);
			}
			this.cursorRow = Math.max(0, newLines.length - 1);
			this.hardwareCursorRow = this.cursorRow;
			buffer += this.buildHardwareCursorMove(cursorPos, newLines.length);
			buffer += "\x1b[?2026l"; // End synchronized output
			this.terminal.write(buffer);
			// Reset max lines when clearing, otherwise track growth
			if (clear) {
				this.maxLinesRendered = newLines.length;
			} else {
				this.maxLinesRendered = Math.max(this.maxLinesRendered, newLines.length);
			}
			const bufferLength = Math.max(height, newLines.length);
			this.previousViewportTop = Math.max(0, bufferLength - height);
			this.previousLines = newLines;
			this.previousKittyImageIds = this.collectKittyImageIds(newLines);
			this.previousWidth = width;
			this.previousHeight = height;
		};

		const debugRedraw = process.env.HOOCODE_DEBUG_REDRAW === "1";
		const logRedraw = (reason: string): void => {
			if (!debugRedraw) return;
			const agentDir = process.env.HOOCODE_CODING_AGENT_DIR ?? path.join(os.homedir(), ".hoocode", "agent");
			const logPath = path.join(agentDir, "hoocode-debug.log");
			const msg = `[${new Date().toISOString()}] fullRender: ${reason} (prev=${this.previousLines.length}, new=${newLines.length}, height=${height})\n`;
			fs.appendFileSync(logPath, msg);
		};

		// First render - just output everything without clearing (assumes clean screen)
		if (this.previousLines.length === 0 && !widthChanged && !heightChanged) {
			logRedraw("first render");
			fullRender(false);
			return;
		}

		// Width changes always need a full re-render because wrapping changes.
		if (widthChanged) {
			logRedraw(`terminal width changed (${this.previousWidth} -> ${width})`);
			fullRender(true);
			return;
		}

		// Height changes normally need a full re-render to keep the visible viewport aligned,
		// but Termux changes height when the software keyboard shows or hides.
		// In that environment, a full redraw causes the entire history to replay on every toggle.
		if (heightChanged && !isTermuxSession()) {
			logRedraw(`terminal height changed (${this.previousHeight} -> ${height})`);
			fullRender(true);
			return;
		}

		// Content shrunk below the working area and no overlays - re-render to clear empty rows
		// (overlays need the padding, so only do this when no overlays are active)
		// Configurable via setClearOnShrink() or HOOCODE_CLEAR_ON_SHRINK=0 env var
		if (this.clearOnShrink && newLines.length < this.maxLinesRendered && this.overlayStack.length === 0) {
			logRedraw(`clearOnShrink (maxLinesRendered=${this.maxLinesRendered})`);
			fullRender(true);
			return;
		}

		// Find first and last changed lines. When the root render() produced a
		// patch report the dirty range is already known and the whole-buffer scan
		// is skipped. On patched frames previousLines is the same in-place-updated
		// array as newLines, so the previous length must come from the report.
		let firstChanged: number;
		let lastChanged: number;
		let prevLineCount: number;
		if (patch !== "full") {
			if (patch === null) {
				prevLineCount = newLines.length;
				firstChanged = -1;
				lastChanged = -1;
			} else {
				prevLineCount = patch.prevLength;
				firstChanged = patch.low;
				lastChanged = patch.high;
			}
		} else {
			prevLineCount = this.previousLines.length;
			firstChanged = -1;
			lastChanged = -1;
			const maxLines = Math.max(newLines.length, prevLineCount);
			for (let i = 0; i < maxLines; i++) {
				const oldLine = i < prevLineCount ? this.previousLines[i] : "";
				const newLine = i < newLines.length ? newLines[i] : "";

				if (oldLine !== newLine) {
					if (firstChanged === -1) {
						firstChanged = i;
					}
					lastChanged = i;
				}
			}
		}
		const appendedLines = newLines.length > prevLineCount;
		if (appendedLines) {
			if (firstChanged === -1) {
				firstChanged = prevLineCount;
			}
			lastChanged = newLines.length - 1;
		}
		if (firstChanged !== -1) {
			lastChanged = this.expandLastChangedForKittyImages(firstChanged, lastChanged);
		}
		const appendStart = appendedLines && firstChanged === prevLineCount && firstChanged > 0;

		// No changes - but still need to update hardware cursor position if it moved
		if (firstChanged === -1) {
			this.positionHardwareCursor(cursorPos, newLines.length);
			this.previousViewportTop = prevViewportTop;
			this.previousHeight = height;
			return;
		}

		// All changes are in deleted lines (nothing to render, just clear)
		if (firstChanged >= newLines.length) {
			if (prevLineCount > newLines.length) {
				let buffer = "\x1b[?2026h";
				buffer += this.deleteChangedKittyImages(firstChanged, lastChanged);
				// Move to end of new content (clamp to 0 for empty content)
				const targetRow = Math.max(0, newLines.length - 1);
				if (targetRow < prevViewportTop) {
					logRedraw(`deleted lines moved viewport up (${targetRow} < ${prevViewportTop})`);
					fullRender(true);
					return;
				}
				const lineDiff = computeLineDiff(targetRow);
				if (lineDiff > 0) buffer += `\x1b[${lineDiff}B`;
				else if (lineDiff < 0) buffer += `\x1b[${-lineDiff}A`;
				buffer += "\r";
				// Clear extra lines without scrolling
				const extraLines = prevLineCount - newLines.length;
				if (extraLines > height) {
					logRedraw(`extraLines > height (${extraLines} > ${height})`);
					fullRender(true);
					return;
				}
				if (extraLines > 0) {
					buffer += "\x1b[1B";
				}
				for (let i = 0; i < extraLines; i++) {
					buffer += "\r\x1b[2K";
					if (i < extraLines - 1) buffer += "\x1b[1B";
				}
				if (extraLines > 0) {
					buffer += `\x1b[${extraLines}A`;
				}
				this.cursorRow = targetRow;
				this.hardwareCursorRow = targetRow;
				buffer += this.buildHardwareCursorMove(cursorPos, newLines.length);
				buffer += "\x1b[?2026l";
				this.terminal.write(buffer);
			} else {
				this.positionHardwareCursor(cursorPos, newLines.length);
			}
			this.previousLines = newLines;
			this.previousKittyImageIds = this.collectKittyImageIds(newLines);
			this.previousWidth = width;
			this.previousHeight = height;
			this.previousViewportTop = prevViewportTop;
			return;
		}

		// Differential rendering can only touch what was actually visible.
		// If the first changed line is above the previous viewport, we need a full redraw.
		if (firstChanged < prevViewportTop) {
			logRedraw(`firstChanged < viewportTop (${firstChanged} < ${prevViewportTop})`);
			fullRender(true);
			return;
		}

		// Render from first changed line to end
		// Build buffer with all updates wrapped in synchronized output
		let buffer = "\x1b[?2026h"; // Begin synchronized output
		buffer += this.deleteChangedKittyImages(firstChanged, lastChanged);
		const prevViewportBottom = prevViewportTop + height - 1;
		const moveTargetRow = appendStart ? firstChanged - 1 : firstChanged;
		if (moveTargetRow > prevViewportBottom) {
			const currentScreenRow = Math.max(0, Math.min(height - 1, hardwareCursorRow - prevViewportTop));
			const moveToBottom = height - 1 - currentScreenRow;
			if (moveToBottom > 0) {
				buffer += `\x1b[${moveToBottom}B`;
			}
			const scroll = moveTargetRow - prevViewportBottom;
			buffer += "\r\n".repeat(scroll);
			prevViewportTop += scroll;
			viewportTop += scroll;
			hardwareCursorRow = moveTargetRow;
		}

		// Move cursor to first changed line (use hardwareCursorRow for actual position)
		const lineDiff = computeLineDiff(moveTargetRow);
		if (lineDiff > 0) {
			buffer += `\x1b[${lineDiff}B`; // Move down
		} else if (lineDiff < 0) {
			buffer += `\x1b[${-lineDiff}A`; // Move up
		}

		buffer += appendStart ? "\r\n" : "\r"; // Move to column 0

		// Only render changed lines (firstChanged to lastChanged), not all lines to end
		// This reduces flicker when only a single line changes (e.g., spinner animation)
		const renderEnd = Math.min(lastChanged, newLines.length - 1);
		for (let i = firstChanged; i <= renderEnd; i++) {
			if (i > firstChanged) buffer += "\r\n";
			buffer += "\x1b[2K"; // Clear current line
			const line = newLines[i];
			const isImage = isImageLine(line);
			if (!isImage && visibleWidth(line) > width) {
				// Log all lines to crash file for debugging
				const agentDir = process.env.HOOCODE_CODING_AGENT_DIR ?? path.join(os.homedir(), ".hoocode", "agent");
				const crashLogPath = path.join(agentDir, "hoocode-crash.log");
				const crashData = [
					`Crash at ${new Date().toISOString()}`,
					`Terminal width: ${width}`,
					`Line ${i} visible width: ${visibleWidth(line)}`,
					"",
					"=== All rendered lines ===",
					...newLines.map((l, idx) => `[${idx}] (w=${visibleWidth(l)}) ${l}`),
					"",
				].join("\n");
				fs.mkdirSync(path.dirname(crashLogPath), { recursive: true });
				fs.writeFileSync(crashLogPath, crashData);

				// Clean up terminal state before throwing
				this.stop();

				const errorMsg = [
					`Rendered line ${i} exceeds terminal width (${visibleWidth(line)} > ${width}).`,
					"",
					"This is likely caused by a custom TUI component not truncating its output.",
					"Use visibleWidth() to measure and truncateToWidth() to truncate lines.",
					"",
					`Debug log written to: ${crashLogPath}`,
				].join("\n");
				throw new Error(errorMsg);
			}
			if (isImage) this.sawImageLine = true;
			buffer += isImage ? line : normalizeTerminalOutput(line) + TUI.SEGMENT_RESET;
		}

		// Track where cursor ended up after rendering
		let finalCursorRow = renderEnd;

		// If we had more lines before, clear them and move cursor back
		if (prevLineCount > newLines.length) {
			// Move to end of new content first if we stopped before it
			if (renderEnd < newLines.length - 1) {
				const moveDown = newLines.length - 1 - renderEnd;
				buffer += `\x1b[${moveDown}B`;
				finalCursorRow = newLines.length - 1;
			}
			const extraLines = prevLineCount - newLines.length;
			for (let i = newLines.length; i < prevLineCount; i++) {
				buffer += "\r\n\x1b[2K";
			}
			// Move cursor back to end of new content
			buffer += `\x1b[${extraLines}A`;
		}

		// Track cursor position for next render
		// cursorRow tracks end of content (for viewport calculation)
		// hardwareCursorRow tracks actual terminal cursor position (for movement)
		this.cursorRow = Math.max(0, newLines.length - 1);
		this.hardwareCursorRow = finalCursorRow;

		// Position hardware cursor for IME. Inside the synchronized block, so the
		// frame is never presented with the cursor still parked at the end of the
		// last redrawn line.
		buffer += this.buildHardwareCursorMove(cursorPos, newLines.length);

		buffer += "\x1b[?2026l"; // End synchronized output

		if (process.env.HOOCODE_TUI_DEBUG === "1") {
			const debugDir = "/tmp/tui";
			fs.mkdirSync(debugDir, { recursive: true });
			const debugPath = path.join(debugDir, `render-${Date.now()}-${Math.random().toString(36).slice(2)}.log`);
			const debugData = [
				`firstChanged: ${firstChanged}`,
				`viewportTop: ${viewportTop}`,
				`cursorRow: ${this.cursorRow}`,
				`height: ${height}`,
				`lineDiff: ${lineDiff}`,
				`hardwareCursorRow: ${hardwareCursorRow}`,
				`renderEnd: ${renderEnd}`,
				`finalCursorRow: ${finalCursorRow}`,
				`cursorPos: ${JSON.stringify(cursorPos)}`,
				`newLines.length: ${newLines.length}`,
				`previousLines.length: ${this.previousLines.length}`,
				"",
				"=== newLines ===",
				JSON.stringify(newLines, null, 2),
				"",
				"=== previousLines ===",
				JSON.stringify(this.previousLines, null, 2),
				"",
				"=== buffer ===",
				JSON.stringify(buffer),
			].join("\n");
			fs.writeFileSync(debugPath, debugData);
		}

		// Write entire buffer at once
		this.terminal.write(buffer);

		// Track terminal's working area (grows but doesn't shrink unless cleared)
		this.maxLinesRendered = Math.max(this.maxLinesRendered, newLines.length);
		this.previousViewportTop = Math.max(prevViewportTop, finalCursorRow - height + 1);

		this.previousLines = newLines;
		this.previousKittyImageIds = this.collectKittyImageIds(newLines);
		this.previousWidth = width;
		this.previousHeight = height;
	}

	/**
	 * Build the escape sequence that parks the hardware cursor for this frame.
	 *
	 * Callers must append the result to the frame buffer *inside* the
	 * synchronized-output block. Emitting it as a separate write leaves the
	 * cursor wherever the last redrawn line ended for the gap between the two
	 * writes, which on an animated status line shows up as a cursor flickering
	 * at the end of that line at the animation's cadence.
	 *
	 * Updates `hardwareCursorRow` to where the sequence leaves the cursor.
	 *
	 * @param cursorPos The cursor position extracted from rendered output, or null
	 * @param totalLines Total number of rendered lines
	 */
	private buildHardwareCursorMove(cursorPos: { row: number; col: number } | null, totalLines: number): string {
		const visibility = this.showHardwareCursor ? SHOW_CURSOR : HIDE_CURSOR;

		if (!cursorPos || totalLines <= 0) {
			// Nothing focused, so there is no position to honor - but the cursor
			// still has to land somewhere known. Lines are padded to the full
			// terminal width, so a frame that ends after the last emitted line
			// leaves the cursor in the terminal's pending-wrap state at the right
			// margin, where it renders on the following row on some terminals.
			// Returning to column 0 keeps it on the row we think it is on.
			return `\r${HIDE_CURSOR}`;
		}

		// Clamp cursor position to valid range
		const targetRow = Math.max(0, Math.min(cursorPos.row, totalLines - 1));
		const targetCol = Math.max(0, cursorPos.col);

		// Move cursor from current position to target
		const rowDelta = targetRow - this.hardwareCursorRow;
		let buffer = "";
		if (rowDelta > 0) {
			buffer += `\x1b[${rowDelta}B`; // Move down
		} else if (rowDelta < 0) {
			buffer += `\x1b[${-rowDelta}A`; // Move up
		}
		// Move to absolute column (1-indexed)
		buffer += `\x1b[${targetCol + 1}G`;

		this.hardwareCursorRow = targetRow;
		return buffer + visibility;
	}

	/**
	 * Position the hardware cursor for IME candidate window, as a standalone
	 * write. Only for frames that emit no content of their own; frames that
	 * build a buffer must fold `buildHardwareCursorMove` into it instead.
	 */
	private positionHardwareCursor(cursorPos: { row: number; col: number } | null, totalLines: number): void {
		this.terminal.write(this.buildHardwareCursorMove(cursorPos, totalLines));
	}
}
