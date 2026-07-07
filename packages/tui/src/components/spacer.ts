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
