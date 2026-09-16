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

	it("answers whether a label would actually fit, so an owner can put it elsewhere", () => {
		const frame = new Frame({ border: "box" });
		frame.addChild(new WidthProbe());
		assert.equal(frame.labelFits(" models ", 60), true);
		assert.equal(frame.labelFits(" a very long surface name ", 20), false);
		assert.equal(frame.labelFits("", 60), false);
		// A label it says fits is a label it actually draws, and vice versa.
		for (const [plain, width] of [
			[" models ", 60],
			[" a very long surface name ", 20],
			[" x ", 8],
			[" x ", 7],
		] as Array<[string, number]>) {
			frame.setLabel({ plain, styled: plain });
			const drawn = frame.render(width)[0].includes(plain.trim());
			assert.equal(drawn, frame.labelFits(plain, width), `${JSON.stringify(plain)} @${width}`);
		}
	});

	it("returns the same lines across frames when nothing changed", () => {
		// The TUI root diffs whole regions by array identity. Re-setting an
		// unchanged label must not drop the memo, or an owner that resolves its
		// label per render hands the root a fresh array every frame.
		const frame = new Frame({ border: "box" });
		// A child that is itself reference-stable, which is the only case where
		// the frame's own stability is observable.
		const rows = ["row0"];
		frame.addChild({ render: () => rows, invalidate: () => {} });
		frame.setLabel({ plain: " demo ", styled: " demo " });
		const first = frame.render(80);
		assert.strictEqual(frame.render(80), first, "an unchanged frame");
		frame.setLabel({ plain: " demo ", styled: " demo " });
		assert.strictEqual(frame.render(80), first, "the same label set again");
		frame.setLabel({ plain: " other ", styled: " other " });
		assert.notStrictEqual(frame.render(80), first, "a label that actually changed");
	});
});

describe("renderFrameEdge", () => {
	it("announces hidden rows on the edge they are hidden past", () => {
		const edge = (side: "top" | "bottom", hidden: number) =>
			renderFrameEdge({
				edge: side,
				barWidth: 58,
				box: true,
				chars: DEFAULT_FRAME_BORDER_CHARS,
				color: (s: string) => s,
				hidden,
			});
		assert.ok(edge("top", 3).includes("↑ 3 more"));
		assert.ok(edge("bottom", 7).includes("↓ 7 more"));
		assert.equal(visibleWidth(edge("top", 3)), 60);
		assert.equal(visibleWidth(edge("bottom", 7)), 60);
	});

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
