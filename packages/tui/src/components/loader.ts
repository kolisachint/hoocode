import type { TUI } from "../tui.js";
import { truncateToWidth, visibleWidth } from "../utils.js";
import { Text } from "./text.js";

export interface LoaderIndicatorOptions {
	/** Animation frames. Use an empty array to hide the indicator. */
	frames?: string[];
	/** Frame interval in milliseconds for animated indicators. */
	intervalMs?: number;
}

const DEFAULT_FRAMES = ["○", "●"];

// A two-frame indicator is a pulse, not a spinner: the eye reads it as the text
// next to it switching on and off rather than as motion, so a spinner-speed
// cadence turns the status line into a strobe. Give pulses a calm beat and keep
// the fast cadence for indicators with enough frames to read as rotation.
// Each frame also requests a full TUI re-render, so the slower default is a
// direct saving on long transcripts.
const PULSE_INTERVAL_MS = 640;
const SPINNER_INTERVAL_MS = 120;
// Floor for caller-supplied intervals. Anything quicker flickers rather than
// animates on every platform, and is unpleasant for photosensitive users.
const MIN_INTERVAL_MS = 80;

function defaultIntervalFor(frames: string[]): number {
	return frames.length <= 2 ? PULSE_INTERVAL_MS : SPINNER_INTERVAL_MS;
}

/**
 * Loader component that updates with an optional spinning animation.
 *
 * Always renders as a single line: the status line sits directly above the
 * editor, so letting a long message wrap would grow and shrink the layout under
 * the prompt every time the message changes.
 */
export class Loader extends Text {
	private frames = [...DEFAULT_FRAMES];
	private intervalMs = defaultIntervalFor(DEFAULT_FRAMES);
	private currentFrame = 0;
	private intervalId: NodeJS.Timeout | null = null;
	private ui: TUI | null = null;
	private renderIndicatorVerbatim = false;
	/** Current single-line content, kept in sync with the Text base state. */
	private line = "";
	// Reference-stable output: containers up the tree diff by array identity, so
	// re-rendering an unchanged loader must hand back the same array.
	private renderedLine?: string;
	private renderedWidth?: number;
	private renderedLines?: string[];

	constructor(
		ui: TUI,
		private spinnerColorFn: (str: string) => string,
		private messageColorFn: (str: string) => string,
		private message: string = "Loading...",
		indicator?: LoaderIndicatorOptions,
	) {
		super("", 1, 0);
		this.ui = ui;
		this.setIndicator(indicator);
	}

	render(width: number): string[] {
		if (this.renderedLines && this.renderedLine === this.line && this.renderedWidth === width) {
			return this.renderedLines;
		}
		// Mirrors Text's paddingX of 1 on both sides, minus the wrapping.
		const contentWidth = Math.max(1, width - 2);
		const content =
			visibleWidth(this.line) > contentWidth ? truncateToWidth(this.line, contentWidth, "...") : this.line;
		const padding = " ".repeat(Math.max(0, contentWidth - visibleWidth(content)));
		this.renderedLine = this.line;
		this.renderedWidth = width;
		this.renderedLines = ["", ` ${content}${padding} `];
		return this.renderedLines;
	}

	override invalidate(): void {
		super.invalidate();
		this.renderedLines = undefined;
	}

	start(): void {
		this.updateDisplay();
		this.restartAnimation();
	}

	stop(): void {
		if (this.intervalId) {
			clearInterval(this.intervalId);
			this.intervalId = null;
		}
	}

	setMessage(message: string): void {
		this.message = message;
		this.updateDisplay();
	}

	setIndicator(indicator?: LoaderIndicatorOptions): void {
		this.renderIndicatorVerbatim = indicator !== undefined;
		this.frames = indicator?.frames !== undefined ? [...indicator.frames] : [...DEFAULT_FRAMES];
		this.intervalMs =
			indicator?.intervalMs && indicator.intervalMs > 0
				? Math.max(MIN_INTERVAL_MS, indicator.intervalMs)
				: defaultIntervalFor(this.frames);
		this.currentFrame = 0;
		this.start();
	}

	private restartAnimation(): void {
		this.stop();
		if (this.frames.length <= 1) {
			return;
		}
		this.intervalId = setInterval(() => {
			this.currentFrame = (this.currentFrame + 1) % this.frames.length;
			this.updateDisplay();
		}, this.intervalMs);
	}

	private updateDisplay(): void {
		const frame = this.frames[this.currentFrame] ?? "";
		const renderedFrame = this.renderIndicatorVerbatim ? frame : this.spinnerColorFn(frame);
		const indicator = frame.length > 0 ? `${renderedFrame} ` : "";
		this.line = `${indicator}${this.messageColorFn(this.message)}`;
		this.setText(this.line);
		if (this.ui) {
			this.ui.requestRender();
		}
	}
}
