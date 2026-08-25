import type { Component } from "../tui.js";
import { applyBackgroundToLine, visibleWidth } from "../utils.js";

type RenderCache = {
	childLines: string[];
	width: number;
	bgSample: string | undefined;
	shadowSample: string | undefined;
	lines: string[];
};

/**
 * Box component - a container that applies padding and background to all children
 */
export class Box implements Component {
	children: Component[] = [];
	private paddingX: number;
	private paddingY: number;
	private bgFn?: (text: string) => string;
	private shadowFn?: (text: string) => string;

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

		const contentWidth = Math.max(1, width - this.paddingX * 2);
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
		const result: string[] = [];

		// Top padding
		for (let i = 0; i < this.paddingY; i++) {
			result.push(this.applyBg("", width));
		}

		// Content
		for (const line of childLines) {
			result.push(this.applyBg(line, width));
		}

		// Bottom padding
		for (let i = 0; i < this.paddingY; i++) {
			result.push(this.applyBg("", width));
		}

		// The shadow rides under the finished block, indented by one cell. It is
		// drawn on the page, not on the box's fill, so it is not run through
		// applyBg.
		if (this.shadowFn && width > 1) {
			result.push(` ${this.shadowFn("▀".repeat(width - 1))}`);
		}

		// Update cache
		this.cache = { childLines, width, bgSample, shadowSample, lines: result };

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
