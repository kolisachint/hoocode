/**
 * The frame the prompt draws, and the geometry every surface that replaces the
 * prompt now inherits from it.
 *
 * Two things are asserted at every width worth worrying about: the frame is
 * exactly as wide as the terminal it was handed — no row short, no row over —
 * and the editor's own border is still the same run of glyphs it always was,
 * because both now come out of one renderer.
 */

import assert from "node:assert";
import { describe, it } from "node:test";
import { Editor } from "../src/components/editor.js";
import { DEFAULT_FRAME_BORDER_CHARS, Frame, renderFrameEdge } from "../src/components/frame.js";
import { Text } from "../src/components/text.js";
import type { Component } from "../src/tui.js";
import { visibleWidth } from "../src/utils.js";

/** A child that reports exactly the width it was handed, so geometry is visible. */
class WidthProbe implements Component {
	seen: number[] = [];
	constructor(private readonly rows: number = 1) {}
	invalidate(): void {}
	render(width: number): string[] {
		this.seen.push(width);
		return Array.from({ length: this.rows }, (_, i) => `row${i}`);
	}
}

/** A child that deliberately overruns whatever width it is given. */
class Overrunner implements Component {
	invalidate(): void {}
	render(width: number): string[] {
		return ["x".repeat(width + 12)];
	}
}

const WIDTHS = [200, 120, 100, 80, 60, 40, 20, 12, 8, 6, 5, 4, 3, 2, 1];

describe("Frame", () => {
	it("fills exactly the width it is given, at every width", () => {
		for (const border of ["box", "rule"] as const) {
			for (const width of WIDTHS) {
				const frame = new Frame({ border });
				frame.addChild(new WidthProbe(3));
				for (const [i, line] of frame.render(width).entries()) {
					assert.equal(visibleWidth(line), width, `${border} @${width}: row ${i} is ${visibleWidth(line)} cells`);
				}
			}
		}
	});

	it("keeps its geometry when a child overruns its width", () => {
		for (const width of WIDTHS) {
			const frame = new Frame({ border: "box" });
			frame.addChild(new Overrunner());
			for (const line of frame.render(width)) {
				assert.equal(visibleWidth(line), width, `@${width}: ${visibleWidth(line)} cells`);
			}
		}
	});

	it("spends two columns on the border and two on the gutter, and no more", () => {
		const probe = new WidthProbe();
		const frame = new Frame({ border: "box", paddingX: 1 });
		frame.addChild(probe);
		frame.render(100);
		assert.equal(probe.seen.at(-1), 96);
		assert.equal(frame.contentWidth(100), 96);
	});

	it("draws nothing at all when it holds nothing", () => {
		const frame = new Frame({ border: "box" });
		assert.deepEqual(frame.render(100), []);
		frame.addChild(new Text("", 0, 0));
		assert.deepEqual(frame.render(100), []);
	});

	it("degrades to rules rather than overflowing when a box will not fit", () => {
		const frame = new Frame({ border: "box" });
		frame.addChild(new WidthProbe());
		const lines = frame.render(3);
		assert.ok(!lines[0].includes("┌"), "a 3-column box has no room for corners");
		for (const line of lines) assert.equal(visibleWidth(line), 3);
	});

	it("passes the whole width through when the border is none", () => {
		const probe = new WidthProbe();
		const frame = new Frame({ border: "none" });
		frame.addChild(probe);
		const lines = frame.render(100);
		assert.equal(probe.seen.at(-1), 100);
		assert.deepEqual(lines, ["row0"]);
	});

	it("lays the label into the top border and leaves the bottom plain", () => {
		const frame = new Frame({ border: "box" });
		frame.setLabel({ plain: " models ", styled: " models " });
		frame.addChild(new WidthProbe());
		const lines = frame.render(60);
		assert.ok(lines[0].includes(" models "));
		assert.ok(!lines.at(-1)!.includes("models"));
		assert.equal(visibleWidth(lines[0]), 60);
	});

	it("drops a label with no room for a run of border beside it", () => {
		const frame = new Frame({ border: "box" });
		frame.setLabel({ plain: " a very long surface name ", styled: " a very long surface name " });
		frame.addChild(new WidthProbe());
		const lines = frame.render(20);
		assert.ok(!lines[0].includes("surface name"));
		assert.equal(visibleWidth(lines[0]), 20);
	});

	it("announces hidden rows on the edge they are hidden past", () => {
		const frame = new Frame({ border: "box" });
		frame.hiddenAbove = 3;
		frame.hiddenBelow = 7;
		frame.addChild(new WidthProbe());
		const lines = frame.render(60);
		assert.ok(lines[0].includes("↑ 3 more"));
		assert.ok(lines.at(-1)!.includes("↓ 7 more"));
	});
});

describe("renderFrameEdge", () => {
	it("is what the editor draws its own border with", () => {
		// One renderer: an editor's top border and a frame's top border at the
		// same width are the same string. If these ever diverge, the prompt and
		// everything that stands in for it have drifted apart again.
		const editor = new Editor(
			{ terminal: { columns: 80, rows: 24 } } as never,
			{
				borderColor: (s: string) => s,
				selectList: {
					selectedPrefix: (s: string) => s,
					selectedText: (s: string) => s,
					description: (s: string) => s,
					scrollInfo: (s: string) => s,
					noMatch: (s: string) => s,
				},
			},
			{ border: "box" },
		);
		const editorTop = editor.render(80)[0];
		const edge = renderFrameEdge({
			edge: "top",
			barWidth: 78,
			box: true,
			chars: DEFAULT_FRAME_BORDER_CHARS,
			color: (s: string) => s,
		});
		assert.equal(editorTop, edge);
	});
});
