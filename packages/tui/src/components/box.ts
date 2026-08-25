import type { Component } from "../tui.js";
import { applyBackgroundToLine, visibleWidth } from "../utils.js";

type RenderCache = {
	childLines: string[];
	width: number;
	bgSample: string | undefined;
	shadowSample: string | undefined;
	inset: number;
	cutEdge: boolean;
	lines: string[];
};

/**
 * Roughly one row in this many takes the cut.
 *
 * A coin flip per row is not a cut edge, it is a sawtooth: the eye reads
 * alternating columns as a pattern, which is the opposite of hand-made. Scissors
 * leave a mostly straight line with the occasional nick, so the deviation has to
 * be rare enough to read as an accident.
 */
const CUT_EVERY = 5;

/** djb2. Small, stable, and good enough to decide one column of a paper edge. */
function hashString(value: string): number {
	let hash = 5381;
	for (let i = 0; i < value.length; i++) {
		hash = ((hash << 5) + hash + value.charCodeAt(i)) >>> 0;
	}
	return hash;
}

/**
 * Box component - a container that applies padding and background to all children
 */
export class Box implements Component {
	children: Component[] = [];
	private paddingX: number;
	private paddingY: number;
	private bgFn?: (text: string) => string;
	private shadowFn?: (text: string) => string;
	private inset = 0;
	private cutEdge = false;

	// Cache for rendered output
	private cache?: RenderCache;

	constructor(paddingX = 1, paddingY = 1, bgFn?: (text: string) => string) {
		this.paddingX = paddingX;
		this.paddingY = paddingY;
		this.bgFn = bgFn;
	}

	/**
	 * Change the horizontal padding after construction.
	 *
	 * For a box whose padding exists to carry a background band: with no band to
	 * draw, the padding is just an indent that pushes the content out of line
	 * with everything around it.
	 */
	setPaddingX(paddingX: number): void {
		if (this.paddingX === paddingX) return;
		this.paddingX = paddingX;
		this.invalidateCache();
	}

	addChild(component: Component): void {
		this.children.push(component);
		this.invalidateCache();
	}

	removeChild(component: Component): void {
		const index = this.children.indexOf(component);
		if (index !== -1) {
			this.children.splice(index, 1);
			this.invalidateCache();
		}
	}

	clear(): void {
		this.children = [];
		this.invalidateCache();
	}

	setBgFn(bgFn?: (text: string) => string): void {
		this.bgFn = bgFn;
		// Don't invalidate here - we'll detect bgFn changes by sampling output
	}

	/**
	 * Draw an offset band under the box, so the block reads as a sheet laid on
	 * the page rather than a color printed into it.
	 *
	 * A terminal has no sub-pixel offsets, so the shadow is one extra row of
	 * `▀` — an upper half-block, which paints solid color across the top half of
	 * its cells and so hugs the box's bottom edge — indented one column to give
	 * the offset. There is no matching right-hand column: these boxes render at
	 * the full terminal width, and a column past the last cell has nowhere to go.
	 *
	 * Passing no function draws no shadow, which is the behavior of every theme
	 * that does not opt in.
	 */
	setShadowFn(shadowFn?: (text: string) => string): void {
		this.shadowFn = shadowFn;
		this.invalidateCache();
	}

	/**
	 * Hold the band back from the right margin, leaving a gutter of page.
	 *
	 * A block that runs the full width of the terminal has no right edge to
	 * show: it is a band of colour between two screen edges, not a sheet on a
	 * page. Reserving a few columns gives it one, which is what makes both the
	 * shadow's right-hand column and a cut edge possible at all.
	 */
	setInset(inset: number): void {
		if (this.inset === inset) return;
		this.inset = Math.max(0, inset);
		this.invalidateCache();
	}

	/**
	 * Cut the right edge by hand rather than ruling it.
	 *
	 * The jitter is one column at most and it only ever eats padding, never
	 * content, so a cut edge cannot cost a character. It is derived from the row
	 * index and the block's own content, which keeps it identical between frames
	 * — an edge that reshuffled on every render would read as noise, not paper.
	 */
	setCutEdge(cutEdge: boolean): void {
		if (this.cutEdge === cutEdge) return;
		this.cutEdge = cutEdge;
		this.invalidateCache();
	}

	private invalidateCache(): void {
		this.cache = undefined;
	}

	private matchCache(
		width: number,
		childLines: string[],
		bgSample: string | undefined,
		shadowSample: string | undefined,
	): boolean {
		const cache = this.cache;
		return (
			!!cache &&
			cache.width === width &&
			cache.bgSample === bgSample &&
			cache.shadowSample === shadowSample &&
			cache.inset === this.inset &&
			cache.cutEdge === this.cutEdge &&
			cache.childLines.length === childLines.length &&
			cache.childLines.every((line, i) => line === childLines[i])
		);
	}

	invalidate(): void {
		this.invalidateCache();
		for (const child of this.children) {
			child.invalidate?.();
		}
	}

	render(width: number): string[] {
		if (this.children.length === 0) {
			return [];
		}

		// The band stops short of the right margin when inset, leaving a gutter of
		// page for the sheet's own edge and the shadow that follows it.
		const bandWidth = Math.max(1, width - this.inset);
		const contentWidth = Math.max(1, bandWidth - this.paddingX * 2);
		const leftPad = " ".repeat(this.paddingX);

		// Render all children
		const childLines: string[] = [];
		for (const child of this.children) {
			const lines = child.render(contentWidth);
			for (const line of lines) {
				childLines.push(leftPad + line);
			}
		}

		if (childLines.length === 0) {
			return [];
		}

		// Check if bgFn output changed by sampling
		const bgSample = this.bgFn ? this.bgFn("test") : undefined;
		const shadowSample = this.shadowFn ? this.shadowFn("test") : undefined;

		// Check cache validity
		if (this.matchCache(width, childLines, bgSample, shadowSample)) {
			return this.cache!.lines;
		}

		// Apply background and padding
		const rows: string[] = [];
		for (let i = 0; i < this.paddingY; i++) rows.push("");
		rows.push(...childLines);
		for (let i = 0; i < this.paddingY; i++) rows.push("");

		// A cut edge needs a seed that is stable for this block but different
		// from its neighbours, or every sheet on screen is cut identically.
		const seed = this.cutEdge ? hashString(childLines.join("\n")) : 0;
		const result: string[] = [];
		rows.forEach((line, index) => {
			// Never let the cut reach into the content: the jitter comes out of
			// the padding the row was going to draw anyway.
			const slack = Math.max(0, bandWidth - visibleWidth(line));
			const cut = this.cutEdge && slack > 0 && hashString(`${seed}:${index}`) % CUT_EVERY === 0 ? 1 : 0;
			const rowWidth = bandWidth - cut;
			const band = this.applyBg(line, rowWidth);
			// The shadow follows the edge it is cast by, so it jitters with it.
			// The first row has none: the offset is down *and* right.
			const column = this.shadowFn && this.inset > 0 && index > 0 ? this.shadowFn("▌") : "";
			result.push(band + column);
		});

		// The shadow's bottom run, offset one column right of the band.
		if (this.shadowFn && bandWidth > 1) {
			result.push(` ${this.shadowFn("▀".repeat(bandWidth - 1))}`);
		}

		// Update cache
		this.cache = {
			childLines,
			width,
			bgSample,
			shadowSample,
			inset: this.inset,
			cutEdge: this.cutEdge,
			lines: result,
		};

		return result;
	}

	private applyBg(line: string, width: number): string {
		const visLen = visibleWidth(line);
		const padNeeded = Math.max(0, width - visLen);
		const padded = line + " ".repeat(padNeeded);

		if (this.bgFn) {
			return applyBackgroundToLine(padded, width, this.bgFn);
		}
		return padded;
	}
}
