import type { Component } from "../tui.js";
import { applyBackgroundToLine, visibleWidth } from "../utils.js";

type RenderCache = {
	childLines: string[];
	width: number;
	bgSample: string | undefined;
	shadowSample: string | undefined;
	inset: number;
	lines: string[];
};

/**
 * The paper treatment a box asks its owner for on every frame: the ink of its
 * shadow and the gutter it holds back from the right margin. Both are optional,
 * and a sheet with neither is the plain full-width band.
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
	 * shadow's right-hand column and a visible right edge possible at all.
	 */
	inset?: number;
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
	 * A shadow and the gutter it needs arrive together because they are one
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
	): boolean {
		const cache = this.cache;
		return (
			!!cache &&
			cache.width === width &&
			cache.bgSample === bgSample &&
			cache.shadowSample === shadowSample &&
			cache.inset === inset &&
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
		// A sheet needs a band wide enough to carry its own bottom run. Narrower
		// than that the gutter has nowhere to go: the treatment degenerated into
		// a one-column band with a shadow beside it and no run under it. Below
		// the threshold the box falls back to the plain full-width band a theme
		// without paper draws, which is the honest answer at that size.
		const wide = width - Math.max(0, paper?.inset ?? 0) > 1;
		const shadowFn = wide ? paper?.shadow : undefined;
		const inset = wide ? Math.max(0, paper?.inset ?? 0) : 0;

		// The band stops short of the right margin when inset, leaving a gutter of
		// page for the sheet's own edge and the shadow that follows it.
		const bandWidth = Math.max(1, width - inset);
		// Padding only exists to carry the band, so it gives way rather than
		// pushing content off the end of it. Clamping `contentWidth` alone was
		// not enough: at a width where the gutter and two columns of padding
		// leave no room, the row came out wider than the band it was supposed
		// to fill, the shadow's column went with it, and the line wrapped past
		// the right margin. Both halves are derived from the band instead.
		const paddingX = Math.min(this.paddingX, Math.max(0, Math.floor((bandWidth - 1) / 2)));
		const contentWidth = Math.max(1, bandWidth - paddingX * 2);
		const leftPad = " ".repeat(paddingX);

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
		if (this.matchCache(width, childLines, bgSample, shadowSample, inset)) {
			return this.cache!.lines;
		}

		// Apply background and padding
		const rows: string[] = [];
		for (let i = 0; i < this.paddingY; i++) rows.push("");
		rows.push(...childLines);
		for (let i = 0; i < this.paddingY; i++) rows.push("");

		// The right-hand column only exists when a gutter was reserved for it.
		const hasColumn = shadowFn !== undefined && inset > 0;
		const result: string[] = [];
		rows.forEach((line, index) => {
			// Every row of the sheet ends in the same column.
			//
			// The edge used to be nicked one column in on roughly every fifth
			// row, to read as cut by hand rather than ruled. At a terminal's
			// resolution it could not: a nick is a whole cell, which is a step
			// far too coarse to read as the wobble of a pair of scissors, and it
			// landed as damage instead. On the top row — the one row with no
			// shadow behind it, because the offset is down as well as right — it
			// showed as a bite taken out of the sheet's corner. Everywhere else
			// it was backfilled with a block of shadow ink, which put a tooth of
			// shadow *inside* the sheet's own outline. Both are the same mistake
			// seen from two sides: the fill was leaving holes and the shadow was
			// covering for them. A ruled edge has neither.
			const band = this.applyBg(line, bandWidth);
			// `▌` paints the left half of its cell, so the column reads as a thin
			// line hugging the sheet rather than a second band beside it. The
			// first row has no column at all: the offset is down *and* right.
			const column = hasColumn && index > 0 ? shadowFn("▌") : "";
			result.push(band + column);
		});

		// The shadow's bottom run, offset one column right of the band. It ends
		// under the right-hand column so the two close the corner; with no
		// gutter there is no such column, and the run stops one cell short of
		// the margin instead of wrapping past it.
		//
		// That last cell is `▘`, not `▀`. The run is a full-width glyph and the
		// column above it is a half-width one, so a run that ended on `▀`
		// overshot the column by half a cell and left a tip poking out past the
		// corner — a stray line coming out of the shadow. `▘` is the same top
		// half narrowed to the column's own width, so the two edges close flush.
		if (shadowFn && bandWidth > 1) {
			const run = "▀".repeat(bandWidth - 1);
			result.push(` ${shadowFn(hasColumn ? `${run}▘` : run)}`);
		}

		// Update cache
		this.cache = {
			childLines,
			width,
			bgSample,
			shadowSample,
			inset,
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
