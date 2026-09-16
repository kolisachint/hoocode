/**
 * The chrome every surface that asks the user for something draws.
 *
 * All of them — the pickers, the `ask_options` pane, the extension input and
 * editor, the login dialog — take the prompt editor's place in
 * `editorContainer`. They used to draw two bare `DynamicBorder` rules instead
 * of the prompt's box, each with its own idea of whether the title went inside,
 * whether the hints got a blank line above them, and whether content was inset
 * at all. So the one surface the eye returns to changed shape whenever the
 * agent needed an answer, and the `editorBorder` setting moved the prompt
 * without moving anything that stands in for it.
 *
 * One frame, one border style, one place the title goes (into the top border,
 * flush right, the slot the session chip rides on the prompt) and one place the
 * hints go (the last row inside, flush above the bottom rule — a blank line
 * next to a rule is a blank line wasted).
 */

import { type Component, Frame, type FrameBorderStyle, Text } from "@kolisachint/hoocode-tui";
import { theme } from "../theme/theme.js";

/**
 * The border style the prompt is currently drawing, so a surface standing in
 * for it draws the same one.
 *
 * Module state rather than a constructor argument because these components are
 * built deep inside command handlers that have no business knowing about
 * settings, and because it has to follow a live `/settings` edit for panes that
 * are already on screen. `interactive-mode` pushes it in `applyRuntimeSettings`
 * alongside `editor.setBorder`, and from the settings callback.
 */
let inputBorderStyle: FrameBorderStyle = "box";

/**
 * Not exported from the package: this follows the user's `editorBorder`
 * setting, and an extension does not get to overrule it. `interactive-mode` is
 * the one caller.
 */
export function setInputFrameBorder(style: FrameBorderStyle): void {
	inputBorderStyle = style;
}

export function getInputFrameBorder(): FrameBorderStyle {
	return inputBorderStyle;
}

export interface InputFrameOptions {
	/** The surface's name. Laid into the top border when it fits there. */
	title?: string;
	/** Columns of gutter inside the side borders. One, as the prompt's is. */
	paddingX?: number;
}

/**
 * A `Frame` wearing the app's border colour and the current border style.
 *
 * The colour is the plain `border` token, not the prompt's: the prompt's border
 * carries the thinking level and bash mode, and a picker has neither to report.
 *
 * The colour, the style and the title are all resolved per frame, so a theme
 * switch or a `/settings` edit repaints a pane that is already open — a `Text`
 * holds its string with the escapes already in it, so a title decided once
 * would keep the colours of the theme it was built under. Re-resolving is
 * cheap: `setBorder` and `setLabel` both compare before they take, so an
 * unchanged frame still returns the same array and the renderer goes on
 * diffing whole regions by identity.
 */
export class InputFrame extends Frame {
	private hintRow?: Text;
	/** The title as one line, or "" for none. Where it is drawn is a render-time call. */
	private titleText = "";
	/** Holds the title when it will not fit in the border. */
	private titleRow?: Text;

	constructor(options: InputFrameOptions = {}) {
		super({
			border: getInputFrameBorder(),
			paddingX: options.paddingX ?? 1,
			color: (str: string) => theme.fg("border", str),
		});
		if (options.title !== undefined) this.setTitle(options.title);
	}

	/**
	 * Name this surface. An empty title draws a plain edge.
	 *
	 * Collapsed to one line first, because the border *is* one line: an
	 * extension's `confirm` passes its question and its detail as one
	 * newline-joined string, and a newline inside a rendered row splits the
	 * border open and throws the renderer's row count out with it.
	 *
	 * Where it ends up is decided per frame in `layOutTitle`, not here — it
	 * depends on the width, which a caller does not know.
	 */
	setTitle(title: string): void {
		this.titleText = title.replace(/\s+/g, " ").trim();
	}

	/**
	 * Put the title where it fits: in the top border, or — when the border has
	 * no room for a run of rule beside it — on the frame's first row.
	 *
	 * The fallback is what stops a title being lost. `renderFrameEdge` drops a
	 * label it cannot draw, which is right for the session chip (the footer also
	 * names the session) and wrong for a question the user is about to answer
	 * yes or no to.
	 *
	 * Rebuilt every frame rather than at `setTitle`, so a theme switch under an
	 * open pane repaints the title along with everything else: a `Text` holds
	 * its string with the escapes already in it.
	 */
	private layOutTitle(width: number): void {
		const plain = this.titleText ? ` ${this.titleText} ` : "";
		// The chip's own spacing, so a title sits off the rule the way the
		// session name does on the prompt.
		const inBorder = this.labelFits(plain, width);
		this.setLabel(inBorder ? { plain, styled: theme.fg("accent", theme.bold(plain)) } : undefined);

		if (inBorder || !this.titleText) {
			if (this.titleRow) {
				this.removeChild(this.titleRow);
				this.titleRow = undefined;
			}
			return;
		}
		if (!this.titleRow) {
			this.titleRow = new Text("", 0, 0);
			this.unshiftTitleRow(this.titleRow);
		}
		this.titleRow.setText(theme.fg("accent", theme.bold(this.titleText)));
	}

	/**
	 * The key hints, as the last row inside the frame.
	 *
	 * Owned here rather than added as a child by each caller so it always ends
	 * up in the same place: callers that added it themselves put it above their
	 * own `Spacer(1)` about half the time, which is the wasted row the vertical
	 * rhythm rules name.
	 */
	setHint(hint: string): void {
		if (this.hintRow) {
			this.hintRow.setText(hint);
			return;
		}
		const row = new Text(hint, 0, 0);
		super.addChild(row);
		this.hintRow = row;
	}

	/**
	 * Add a row above the hints.
	 *
	 * The hints are the frame's last row by construction, so a pane that sets
	 * them before it finishes building its body still gets them at the bottom —
	 * which is the ordering half the panes got wrong when each added its own.
	 */
	override addChild(component: Component): void {
		if (!this.hintRow) {
			super.addChild(component);
			return;
		}
		const hint = this.hintRow;
		this.removeChild(hint);
		super.addChild(component);
		super.addChild(hint);
	}

	/** The title row, when there is one, is the frame's first row. */
	private unshiftTitleRow(row: Text): void {
		this.children.unshift(row);
		this.invalidate();
	}

	override render(width: number): string[] {
		// The style is read per frame: a `/settings` edit has to reach a pane
		// that is already open, and the pane is what the user is looking at.
		this.setBorder(getInputFrameBorder());
		this.layOutTitle(width);
		return super.render(width);
	}
}
