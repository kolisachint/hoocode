import assert from "node:assert";
import { describe, it } from "node:test";
import { applyBackgroundToLine, visibleWidth } from "../src/utils.js";

/**
 * A filled band has to survive whatever styling its children close.
 *
 * Every case here is a child ending its own styling in a way that also ends the
 * band's fill. The band is a background the *parent* opened, so the cells after
 * the child's reset belong to the parent and have to keep its colour —
 * otherwise the block reads as a sheet with a hole punched from the child to the
 * right margin, which is what a cut-out theme's shadow then appears to float on.
 */

/** A band painted in 256-colour 224, closed the way a theme's `bg()` closes it. */
const band = (text: string) => `\x1b[48;5;224m${text}\x1b[49m`;

/** The cells a terminal would paint in a background, as a run of booleans. */
function paintedCells(rendered: string, width: number): boolean[] {
	const cells: boolean[] = [];
	let painted = false;
	let i = 0;
	while (i < rendered.length && cells.length < width) {
		const sgr = /^\x1b\[([0-9;]*)m/.exec(rendered.slice(i));
		if (sgr) {
			for (const param of (sgr[1] === "" ? "0" : sgr[1]).split(";")) {
				const code = Number(param === "" ? "0" : param);
				if (code === 0 || code === 49) painted = false;
				if (code === 48) painted = true;
			}
			i += sgr[0].length;
			continue;
		}
		cells.push(painted);
		i += 1;
	}
	return cells;
}

const WIDTH = 20;
const fullyPainted = Array(WIDTH).fill(true);

describe("applyBackgroundToLine", () => {
	it("paints every cell of a plain line", () => {
		assert.deepStrictEqual(paintedCells(applyBackgroundToLine("hello", WIDTH, band), WIDTH), fullyPainted);
	});

	// The hole this guards against starts at the reset and runs to the right
	// margin, so a single unpainted cell anywhere is the bug.
	const resets: Array<[string, string]> = [
		["a bare background reset", "\x1b[49m"],
		["a full reset", "\x1b[0m"],
		["an implicit reset", "\x1b[m"],
		["a compound reset", "\x1b[0;32m"],
		["a reset with an empty parameter", "\x1b[;32m"],
	];
	for (const [label, reset] of resets) {
		it(`keeps the fill across ${label} from a child`, () => {
			const rendered = applyBackgroundToLine(`ab${reset}cd`, WIDTH, band);
			assert.deepStrictEqual(paintedCells(rendered, WIDTH), fullyPainted);
		});
	}

	it("keeps the fill when a child closes at the very end of the line", () => {
		// `truncateToWidth` ends this way, so it is the common case rather than an
		// edge one: the padding after it is what would lose its colour.
		const rendered = applyBackgroundToLine("abcd\x1b[0m", WIDTH, band);
		assert.deepStrictEqual(paintedCells(rendered, WIDTH), fullyPainted);
	});

	it("leaves a child's own fill alone", () => {
		// The chip keeps its colour; only what follows it returns to the band.
		const rendered = applyBackgroundToLine("a\x1b[48;5;33mchip\x1b[49mb", WIDTH, band);
		assert.strictEqual(rendered.includes("\x1b[48;5;33mchip"), true);
		assert.deepStrictEqual(paintedCells(rendered, WIDTH), fullyPainted);
	});

	it("adds no width, so layout is unchanged", () => {
		for (const line of ["plain", "ab\x1b[0mcd", "ab\x1b[49mcd"]) {
			assert.strictEqual(visibleWidth(applyBackgroundToLine(line, WIDTH, band)), WIDTH);
		}
	});

	it("is a no-op for a bgFn that paints nothing", () => {
		const rendered = applyBackgroundToLine("ab\x1b[0mcd", WIDTH, (text) => text);
		assert.strictEqual(rendered, `ab\x1b[0mcd${" ".repeat(WIDTH - 4)}`);
	});
});
