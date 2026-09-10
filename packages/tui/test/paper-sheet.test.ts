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
	box.setPaper(() => ({ shadow: SHADOW, inset: INSET }));
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

	it("draws one half-cell of shadow beside the sheet and nothing more", () => {
		// The edge used to step one column in on roughly every fifth row, to
		// read as cut by hand rather than ruled. A terminal cell is far too
		// coarse a step for that: it read as damage, and the block of shadow ink
		// that backfilled the gap put a tooth of shadow inside the sheet's own
		// outline. The fill was leaving holes and the shadow was covering for
		// them; a ruled edge has neither.
		const lines = sheet(24).slice(1, -1);
		for (const segment of lines.map(shadowSegment)) {
			assert.equal(segment, "<s>▌</s>", `shadow segment should be one column, got ${JSON.stringify(segment)}`);
		}
	});

	it("leaves the sheet's top-right corner whole", () => {
		// The top row is the one row with no shadow behind it, because the
		// offset is down as well as right. A nick there had nothing to fall back
		// on and showed as a bite taken out of the corner.
		const lines = sheet(24);
		assert.equal(visibleWidth(plain(lines[0])), WIDTH - INSET);
		assert.equal(shadowSegment(lines[0]), "");
	});

	it("holds every shadowed row to the same width", () => {
		// The column stays in one cell: a one-column step leaves no overlap
		// between one row's half-cell mark and the next, and the edge reads as a
		// dashed staircase.
		const lines = sheet(24);
		const edge = WIDTH - INSET + 1;
		for (const line of lines.slice(1)) {
			assert.equal(visibleWidth(plain(line)), edge);
		}
	});

	it("gives the treatment up rather than degenerate when the band is too narrow", () => {
		// Below a band that can carry its own bottom run there is nowhere for the
		// gutter to go: the sheet came out as a one-column band with a shadow
		// beside it and no run under it, and the row it drew ran past the right
		// margin and wrapped. At that size the plain full-width band — what a
		// theme without paper draws — is the honest answer.
		for (const width of [1, 2, 3, 4]) {
			const box = new Box(1, 1, SHEET);
			box.setPaper(() => ({ shadow: SHADOW, inset: INSET }));
			box.addChild(new Text("hello there", 0, 0));
			const lines = box.render(width);
			for (const line of lines) {
				assert.ok(
					visibleWidth(plain(line)) <= width,
					`@${width}: row is ${visibleWidth(plain(line))} cells wide and will wrap`,
				);
				assert.ok(!/[▌▀▘]/.test(line), `@${width}: drew shadow ink with no room for it`);
			}
		}
	});

	it("keeps every row inside the terminal at the widths where it is a sheet", () => {
		for (const width of [5, 6, 8, 12, 20, 40, 120]) {
			const box = new Box(1, 1, SHEET);
			box.setPaper(() => ({ shadow: SHADOW, inset: INSET }));
			for (let i = 0; i < 12; i++) box.addChild(new Text(`a row of text ${i}`, 0, 0));
			for (const line of box.render(width)) {
				assert.ok(
					visibleWidth(plain(line)) <= width,
					`@${width}: row is ${visibleWidth(plain(line))} cells wide and will wrap`,
				);
			}
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
