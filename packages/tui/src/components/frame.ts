/**
 * The frame the prompt box draws, and the one every surface that takes its
 * place draws with it.
 *
 * The editor owned this border alone for a long time, and every other surface
 * that replaces the editor — the pickers, the options pane, the extension
 * input and editor, the login dialog — drew two bare rules instead. Two rules
 * is not the same shape as a box: the content sat flush at column 0 where the
 * editor insets it, nothing named the surface, and switching the editor to
 * `rule` or back moved the prompt without moving anything that stands in for
 * it. One renderer for the edge, one component for the frame, and the app
 * points both at the same border style.
 */

import { Container } from "../tui.js";
import { truncateToWidth, visibleWidth } from "../utils.js";

/**
 * `box` rules all four sides; `rule` is the bare pair of horizontals; `none`
 * draws no border at all — for a component nested inside a frame that is
 * already ruling it, where a second rule is just noise.
 */
export type FrameBorderStyle = "rule" | "box" | "none";

export interface FrameBorderChars {
	horizontal: string;
	vertical: string;
	topLeft: string;
	topRight: string;
	bottomLeft: string;
	bottomRight: string;
}

export const DEFAULT_FRAME_BORDER_CHARS: FrameBorderChars = {
	horizontal: "─",
	vertical: "│",
	topLeft: "┌",
	topRight: "┐",
	bottomLeft: "└",
	bottomRight: "┘",
};

/**
 * A label laid into a frame's top border, flush right. `plain` drives the
 * width math; `styled` is what is emitted and must occupy exactly
 * `visibleWidth(plain)` cells — the same plain/styled discipline the footer uses.
 */
export interface FrameLabel {
	plain: string;
	styled: string;
}

/** Border cells kept between the label and the top-right corner, so it reads as inset rather than jammed. */
export const LABEL_RIGHT_INSET = 2;

/**
 * Border cells required to the *left* of the label. Below this the label is
 * dropped rather than drawn: a label butting straight up against the scroll
 * indicator reads as one run-on string, and a border with no run of border in it
 * has stopped looking like a border.
 */
export const MIN_LABEL_LEAD_IN = 4;

export interface FrameEdgeOptions {
	edge: "top" | "bottom";
	/** Cells between the corners in box mode; the whole width in rule mode. */
	barWidth: number;
	box: boolean;
	chars: FrameBorderChars;
	color: (str: string) => string;
	/** Rides the top border only, and yields to the scroll indicator. */
	label?: FrameLabel;
	/** Rows hidden past this edge, announced as `─── ↑ N more `. */
	hidden?: number;
}

/**
 * One horizontal edge of a frame: the rule, the scroll indicator that eats into
 * it, the label that rides what is left, and the corners in box mode.
 */
export function renderFrameEdge(options: FrameEdgeOptions): string {
	const { edge, barWidth, box, chars, color } = options;
	const hidden = options.hidden ?? 0;
	let indicator = "";
	if (hidden > 0) {
		const arrow = edge === "top" ? "↑" : "↓";
		indicator = `${chars.horizontal.repeat(3)} ${arrow} ${hidden} more `;
	}
	const indicatorWidth = visibleWidth(indicator);

	// The label rides on the top border only, and yields to the scroll
	// indicator: the indicator says the content is longer than the frame, which
	// the reader needs *now*, while the label only says which surface this is.
	const label = edge === "top" ? options.label : undefined;
	const leadIn = label ? barWidth - indicatorWidth - visibleWidth(label.plain) - LABEL_RIGHT_INSET : -1;

	let bar: string;
	if (label && leadIn >= MIN_LABEL_LEAD_IN) {
		bar =
			color(indicator + chars.horizontal.repeat(leadIn)) +
			label.styled +
			color(chars.horizontal.repeat(LABEL_RIGHT_INSET));
	} else if (indicatorWidth > 0) {
		const remaining = barWidth - indicatorWidth;
		bar = color(
			remaining >= 0 ? indicator + chars.horizontal.repeat(remaining) : truncateToWidth(indicator, barWidth),
		);
	} else {
		bar = color(chars.horizontal.repeat(barWidth));
	}
	if (!box) return bar;
	// Corners must stay aligned, so pad back any width lost to truncation.
	bar += color(chars.horizontal.repeat(Math.max(0, barWidth - visibleWidth(bar))));
	const left = edge === "top" ? chars.topLeft : chars.bottomLeft;
	const right = edge === "top" ? chars.topRight : chars.bottomRight;
	return color(left) + bar + color(right);
}

export interface FrameOptions {
	border?: FrameBorderStyle;
	/** Columns of gutter inside the side borders. Defaults to 1, as the prompt's does. */
	paddingX?: number;
	borderChars?: Partial<FrameBorderChars>;
	color?: (str: string) => string;
}

/**
 * A `Container` that frames its children.
 *
 * Children are rendered at the width left inside the border and the gutter, and
 * every row is padded out to it, so a row that styles a whole band (a selected
 * list row, say) fills the frame rather than the terminal. A frame with nothing
 * in it draws nothing at all — the same contract `Box` keeps.
 */
export class Frame extends Container {
	public borderColor: (str: string) => string;
	/** Laid into the top border, flush right. Undefined draws a plain edge. */
	public label?: FrameLabel;
	private border: FrameBorderStyle;
	private paddingX: number;
	private borderChars: FrameBorderChars;
	private memo?: { width: number; childLines: string[]; top: string; bottom: string; lines: string[] };

	constructor(options: FrameOptions = {}) {
		super();
		this.border = options.border ?? "box";
		this.paddingX = options.paddingX ?? 1;
		this.borderChars = { ...DEFAULT_FRAME_BORDER_CHARS, ...options.borderChars };
		this.borderColor = options.color ?? ((str: string) => str);
	}

	getBorder(): FrameBorderStyle {
		return this.border;
	}

	setBorder(border: FrameBorderStyle): void {
		if (this.border === border) return;
		this.border = border;
		this.invalidate();
	}

	setPaddingX(paddingX: number): void {
		if (this.paddingX === paddingX) return;
		this.paddingX = paddingX;
		this.invalidate();
	}

	/**
	 * Lay a label into the top border, or clear it.
	 *
	 * Compared before it is taken, not just assigned: an owner that resolves its
	 * label per frame (as `InputFrame` does, so a theme switch repaints it) calls
	 * this on every render, and dropping the memo each time would hand the TUI
	 * root a fresh array every frame — which is exactly the reference stability
	 * the renderer diffs whole regions by.
	 */
	setLabel(label: FrameLabel | undefined): void {
		if (this.label?.plain === label?.plain && this.label?.styled === label?.styled) return;
		this.label = label;
		this.memo = undefined;
	}

	override invalidate(): void {
		this.memo = undefined;
		super.invalidate();
	}

	/** The width children are handed at this frame width. */
	contentWidth(width: number): number {
		if (this.border === "none") return Math.max(1, width);
		const box = this.isBox(width);
		const innerWidth = Math.max(1, width - (box ? 2 : 0));
		return Math.max(1, innerWidth - this.resolvePaddingX(innerWidth) * 2);
	}

	/**
	 * Whether a label of this plain text would actually ride the top border at
	 * this width, rather than being dropped for want of a run of border beside
	 * it. The owner asks so it can put the text somewhere else instead of
	 * losing it — see `InputFrame`.
	 */
	labelFits(plain: string, width: number): boolean {
		if (this.border === "none" || plain === "") return false;
		const innerWidth = Math.max(1, width - (this.isBox(width) ? 2 : 0));
		return innerWidth - visibleWidth(plain) - LABEL_RIGHT_INSET >= MIN_LABEL_LEAD_IN;
	}

	/** Box mode needs two columns for the sides plus one of content; below that it rules. */
	private isBox(width: number): boolean {
		return this.border === "box" && width >= 4;
	}

	private resolvePaddingX(innerWidth: number): number {
		return Math.min(this.paddingX, Math.max(0, Math.floor((innerWidth - 1) / 2)));
	}

	override render(width: number): string[] {
		// No border means no gutter either: the children get the whole width,
		// exactly as a plain Container would hand it to them.
		if (this.border === "none") return super.render(width);
		const box = this.isBox(width);
		const innerWidth = Math.max(1, width - (box ? 2 : 0));
		const paddingX = this.resolvePaddingX(innerWidth);
		const contentWidth = Math.max(1, innerWidth - paddingX * 2);

		const childLines = super.render(contentWidth);
		if (childLines.length === 0) return [];

		const edge = (which: "top" | "bottom") =>
			renderFrameEdge({
				edge: which,
				barWidth: innerWidth,
				box,
				chars: this.borderChars,
				color: this.borderColor,
				label: this.label,
			});
		const top = edge("top");
		const bottom = edge("bottom");

		const memo = this.memo;
		if (
			memo &&
			memo.width === width &&
			memo.childLines === childLines &&
			memo.top === top &&
			memo.bottom === bottom
		) {
			return memo.lines;
		}

		const vertical = box ? this.borderColor(this.borderChars.vertical) : "";
		const gutter = " ".repeat(paddingX);
		const lines: string[] = [top];
		for (const line of childLines) {
			// A child that overruns its width would push the right border onto the
			// next row and wrap the whole frame. The frame's geometry is not the
			// child's to break, so it is cut here rather than trusted.
			const body = visibleWidth(line) > contentWidth ? truncateToWidth(line, contentWidth) : line;
			const fill = " ".repeat(Math.max(0, contentWidth - visibleWidth(body)));
			lines.push(`${vertical}${gutter}${body}${fill}${gutter}${vertical}`);
		}
		lines.push(bottom);

		this.memo = { width, childLines, top, bottom, lines };
		return lines;
	}
}
