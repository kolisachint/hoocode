import assert from "node:assert";
import { describe, it } from "node:test";
import { Box } from "../src/components/box.js";
import { Text } from "../src/components/text.js";
import { visibleWidth } from "../src/utils.js";

/**
 * The geometry of a paper sheet's shadow, checked at the cell.
 *
 * Both faults this file guards against were half-cell errors, which is why they
 * survived review of the code and only showed up on screen: `▌` paints the left
 * half of its cell, `▀` paints a full-width top half, and a run of one under a
 * column of the other does not line up. Markers stand in for the theme's ink so
 * every cell in the rendered row can be named.
 */
const SHEET = (text: string) => `<b>${text}</b>`;
const SHADOW = (text: string) => `<s>${text}</s>`;
const INSET = 3;
const WIDTH = 40;

/** A sheet with the full paper treatment, `rows` lines of content deep. */
function sheet(rows: number): string[] {
	const box = new Box(1, 1, SHEET);
	box.setPaper(() => ({ shadow: SHADOW, inset: INSET, cutEdge: true }));
	for (let i = 0; i < rows; i++) box.addChild(new Text(`row ${i}`, 0, 0));
	return box.render(WIDTH);
}

/** A rendered row with the ink markers taken back out. */
function plain(line: string): string {
	return line.replace(/<\/?[bs]>/g, "");
}

/** Everything a row draws after its band — the shadow's own segment. */
function shadowSegment(line: string): string {
	const end = line.lastIndexOf("</b>");
	return end === -1 ? line : line.slice(end + "</b>".length);
}

describe("a sheet's shadow", () => {
	it("closes the bottom corner flush with its right-hand column", () => {
		// `▀` fills a cell edge to edge and `▌` fills the left half of one, so a
		// bottom run that ended on `▀` under the column overshot it by half a
		// cell — a tip poking out past the corner, on every filled block the
		// theme drew. `▘` is that same top half cut back to the column's width.
		const lines = sheet(3);
		const run = lines[lines.length - 1];
		assert.ok(run.endsWith("▘</s>"), `bottom run should end on ▘, got ${JSON.stringify(run)}`);
		assert.equal(run.match(/▘/g)?.length, 1);

		// And it ends in the same cell the column above it occupies.
		assert.equal(plain(run).indexOf("▘"), plain(lines[lines.length - 2]).indexOf("▌"));
	});

	it("leaves no page between the sheet and its own shadow", () => {
		// A nick is a notch scissors took out of the sheet, not a hole in the
		// shadow behind it, so the column it gives back is inked. Left as page
		// — which it used to be — it parked a gutter of paper between the sheet
		// and its shadow, and a detached shadow reads as a rendering fault.
		const lines = sheet(24).slice(1, -1);
		const segments = lines.map(shadowSegment);
		assert.ok(
			segments.some((segment) => segment === "<s>█▌</s>"),
			"the sample needs at least one nicked row to prove anything",
		);
		for (const segment of segments) {
			assert.ok(
				segment === "<s>▌</s>" || segment === "<s>█▌</s>",
				`shadow segment should carry no page, got ${JSON.stringify(segment)}`,
			);
		}
	});

	it("holds every shadowed row to the same width, nick or no nick", () => {
		// The column stays in one cell whatever the cut does to the edge in
		// front of it: a one-column step leaves no overlap between one row's
		// half-cell mark and the next, and the edge reads as a dashed staircase.
		const lines = sheet(24);
		const edge = WIDTH - INSET + 1;
		for (const line of lines.slice(1)) {
			assert.equal(visibleWidth(plain(line)), edge);
		}
	});

	it("stops one cell short of the margin when there is no gutter to close", () => {
		// With no inset there is no right-hand column, so there is no corner to
		// close and nothing for `▘` to line up with.
		const box = new Box(1, 1, SHEET);
		box.setPaper(() => ({ shadow: SHADOW }));
		box.addChild(new Text("hello", 0, 0));
		const lines = box.render(WIDTH);
		const run = lines[lines.length - 1];
		assert.ok(!run.includes("▘"));
		assert.equal(run.match(/▀/g)?.length, WIDTH - 1);
	});
});
