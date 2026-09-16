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

export function setInputFrameBorder(style: FrameBorderStyle): void {
	inputBorderStyle = style;
}

export function getInputFrameBorder(): FrameBorderStyle {
	return inputBorderStyle;
}

export interface InputFrameOptions {
	/** Laid into the top border, flush right. */
	title?: string;
	/** Columns of gutter inside the side borders. One, as the prompt's is. */
	paddingX?: number;
}

/**
 * A `Frame` wearing the app's border colour and the current border style.
 *
 * The colour is the plain `border` token, not the prompt's: the prompt's border
 * carries the thinking level and bash mode, and a picker has neither to report.
 * Both are resolved per frame, so a theme switch under an open pane repaints it.
 */
export class InputFrame extends Frame {
	private hintRow?: Text;

	constructor(options: InputFrameOptions = {}) {
		super({
			border: getInputFrameBorder(),
			paddingX: options.paddingX ?? 1,
			color: (str: string) => theme.fg("border", str),
		});
		if (options.title !== undefined) this.setTitle(options.title);
	}

	/** Name this surface in the top border. An empty title draws a plain edge. */
	setTitle(title: string): void {
		if (!title) {
			this.setLabel(undefined);
			return;
		}
		// The chip's own spacing, so a title sits off the rule the way the
		// session name does on the prompt.
		const plain = ` ${title} `;
		this.setLabel({ plain, styled: theme.fg("accent", theme.bold(plain)) });
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

	override render(width: number): string[] {
		// The style is read per frame: a `/settings` edit has to reach a pane
		// that is already open, and the pane is what the user is looking at.
		this.setBorder(getInputFrameBorder());
		return super.render(width);
	}
}
