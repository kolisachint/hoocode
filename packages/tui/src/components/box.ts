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

/**
 * The paper treatment a box asks its owner for on every frame: the ink of its
 * shadow, the gutter it holds back from the right margin, and whether its right
 * edge is cut by hand. Each part is optional, and a sheet with none of them is
 * the plain full-width band.
 */
export interface PaperSheet {
	/** Paints the shadow's own glyphs. Omitted, the sheet casts no shadow. */
	shadow?: (text: string) => string;
	/**
	 * Columns of page held back at the right margin.
	 *
	 * A block that runs the full width of the terminal has no right edge to
	 * show: it is a band of colour between two screen edges, not a sheet on a
	 * page. Reserving a few columns gives it one, which is what makes both the
	 * shadow's right-hand column and a cut edge possible at all.
	 */
	inset?: number;
	/**
	 * Cut the right edge by hand rather than ruling it.
	 *
	 * The jitter is one column at most and it only ever eats padding, never
	 * content, so a cut edge cannot cost a character. It is derived from the row
	 * index and the block's own content, which keeps it identical between frames
	 * — an edge that reshuffled on every render would read as noise, not paper.
	 */
	cutEdge?: boolean;
}

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
	private paperFn?: () => PaperSheet | undefined;

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
	 * Give the box the paper treatment, resolved on every frame.
	 *
	 * A shadow, a gutter and a cut edge arrive together because they are one
	 * decision, and it is a decision the *theme* makes — so the box asks for it
	 * at render time rather than being told once. A box built under one theme
	 * outlives it: the user switches theme with the block already on screen, and
	 * a shadow function captured at construction would still be painting the old
	 * theme's ink (or reaching for a colour the new theme never defined).
	 *
	 * The shadow itself is one extra row of `▀` — an upper half-block, which
	 * paints solid colour across the top half of its cells and so hugs the box's
	 * bottom edge — indented one column to give the offset a terminal cannot
	 * give in sub-pixels. An inset box gets a matching column of `▌` down its
	 * right edge and the bottom run reaches under it, so the two close the
	 * corner; with no inset the band already owns the last cell, so the bottom
	 * edge is all there is room for.
	 *
	 * Passing no provider, or one that returns nothing, draws the full-width
	 * band the box has always drawn.
	 */
	setPaper(paperFn?: () => PaperSheet | undefined): void {
		this.paperFn = paperFn;
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
		inset: number,
		cutEdge: boolean,
	): boolean {
		const cache = this.cache;
		return (
			!!cache &&
			cache.width === width &&
			cache.bgSample === bgSample &&
			cache.shadowSample === shadowSample &&
			cache.inset === inset &&
			cache.cutEdge === cutEdge &&
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

		// The paper treatment is asked for per frame, so a box already on screen
		// follows a theme switch instead of holding the treatment it was built
		// with.
		const paper = this.paperFn?.();
		const shadowFn = paper?.shadow;
		const inset = Math.max(0, paper?.inset ?? 0);
		const cutEdge = paper?.cutEdge === true;

		// The band stops short of the right margin when inset, leaving a gutter of
		// page for the sheet's own edge and the shadow that follows it.
		const bandWidth = Math.max(1, width - inset);
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
		const shadowSample = shadowFn ? shadowFn("test") : undefined;

		// Check cache validity
		if (this.matchCache(width, childLines, bgSample, shadowSample, inset, cutEdge)) {
			return this.cache!.lines;
		}

		// Apply background and padding
		const rows: string[] = [];
		for (let i = 0; i < this.paddingY; i++) rows.push("");
		rows.push(...childLines);
		for (let i = 0; i < this.paddingY; i++) rows.push("");

		// A cut edge needs a seed that is stable for this block but different
		// from its neighbours, or every sheet on screen is cut identically.
		const seed = cutEdge ? hashString(childLines.join("\n")) : 0;
		// The right-hand column only exists when a gutter was reserved for it.
		const hasColumn = shadowFn !== undefined && inset > 0;
		const result: string[] = [];
		rows.forEach((line, index) => {
			// Never let the cut reach into the content: the jitter comes out of
			// the padding the row was going to draw anyway.
			const slack = Math.max(0, bandWidth - visibleWidth(line));
			const cut = cutEdge && slack > 0 && hashString(`${seed}:${index}`) % CUT_EVERY === 0 ? 1 : 0;
			const rowWidth = bandWidth - cut;
			const band = this.applyBg(line, rowWidth);
			// The shadow holds one column whatever the cut does to the edge in
			// front of it. Following the cut was the first attempt and it broke
			// the shadow: `▌` paints half a cell, so a one-column step leaves no
			// overlap at all between one row's mark and the next, and what the
			// eye gets is a dashed staircase rather than an edge. A cut row
			// shows its nick as a column of page between sheet and shadow —
			// which is what a nick is — and the shadow stays a single line.
			// The first row has none: the offset is down *and* right.
			const column = hasColumn && index > 0 ? " ".repeat(cut) + shadowFn("▌") : "";
			result.push(band + column);
		});

		// The shadow's bottom run, offset one column right of the band. It ends
		// under the right-hand column so the two close the corner; with no
		// gutter there is no such column, and the run stops one cell short of
		// the margin instead of wrapping past it.
		if (shadowFn && bandWidth > 1) {
			result.push(` ${shadowFn("▀".repeat(hasColumn ? bandWidth : bandWidth - 1))}`);
		}

		// Update cache
		this.cache = {
			childLines,
			width,
			bgSample,
			shadowSample,
			inset,
			cutEdge,
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
