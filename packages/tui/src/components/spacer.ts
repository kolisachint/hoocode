import type { Component } from "../tui.js";

/**
 * Spacer component that renders empty lines
 */
export class Spacer implements Component {
	private lines: number;

	constructor(lines: number = 1) {
		this.lines = lines;
	}

	// Cached so repeated renders stay reference-stable (parents memoize by ref).
	private cached?: string[];

	setLines(lines: number): void {
		this.lines = lines;
		this.cached = undefined;
	}

	invalidate(): void {
		// Output depends only on `lines`; keep the cache.
	}

	render(_width: number): string[] {
		if (!this.cached || this.cached.length !== this.lines) {
			const result: string[] = [];
			for (let i = 0; i < this.lines; i++) {
				result.push("");
			}
			this.cached = result;
		}
		return this.cached;
	}
}

/**
 * A spacer whose height the renderer decides, so the layout can fill the screen.
 *
 * ## Why the renderer and not the layout
 *
 * The app is a full-screen one: whatever is at the top of the tree belongs at
 * the top of the terminal and the prompt belongs on the bottom row, with the
 * conversation between them. Nothing in the component tree can work that out on
 * its own — the height it has to make up is the terminal's height minus the
 * height of *every other child*, which is only known once they have all
 * rendered. So the root measures the frame it just built and tells this spacer
 * what is left over (`TUI.setFlexSpacer`).
 *
 * Put one child of the root between the part that flows from the top and the
 * chrome that hangs off the bottom. Once the content is taller than the screen
 * the height settles at 0 and this costs nothing.
 *
 * `setHeight` reports whether it moved, because the root's flat cache compares
 * child output by array identity: handing back a fresh array for an unchanged
 * height would mark the whole tail of the buffer dirty on every single frame.
 */
export class FlexSpacer implements Component {
	private height = 0;
	private lines: string[] = [];

	/** Rows it is currently contributing. */
	get currentHeight(): number {
		return this.height;
	}

	/** Set the fill; true when it changed and a re-flatten is owed. */
	setHeight(height: number): boolean {
		const next = Math.max(0, Math.floor(height));
		if (next === this.height) return false;
		this.height = next;
		this.lines = new Array<string>(next).fill("");
		return true;
	}

	invalidate(): void {
		// Output depends only on `height`; the blank rows never restyle.
	}

	render(_width: number): string[] {
		return this.lines;
	}
}
