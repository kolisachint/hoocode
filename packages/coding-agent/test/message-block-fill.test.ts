/**
 * The fill under a message block, checked cell by cell.
 *
 * The shadow is the part you notice, but it is only ever as good as the band it
 * hangs off: a fill that stops short, runs long, or develops a hole where a
 * child closed its own styling puts the shadow somewhere the sheet is not. This
 * walks the rendered rows one terminal cell at a time — tracking which
 * background is in force, in grapheme clusters so an emoji is one wide cell
 * rather than several — and asserts the sheet's whole geometry against it, for
 * every block, every content shape that has ever wrapped oddly, and every width
 * from a full terminal down to a pathological one.
 */

import { Box, Text, visibleWidth } from "@kolisachint/hoocode-tui";
import { describe, expect, it } from "vitest";
import { CustomMessageComponent } from "../src/modes/interactive/components/custom-message.js";
import { UserMessageComponent } from "../src/modes/interactive/components/user-message.js";
import { applyBlockFill, initTheme, PAPER_INSET } from "../src/modes/interactive/theme/theme.js";

type Cell = { bg: string; glyph: string };

const SEGMENTER = new Intl.Segmenter(undefined, { granularity: "grapheme" });

function scan(line: string): Cell[] {
	let bg = "";
	const out: Cell[] = [];
	let i = 0;
	while (i < line.length) {
		if (line[i] === "\x1b") {
			const sgr = /^\x1b\[([0-9;]*)m/.exec(line.slice(i));
			if (sgr) {
				const params = (sgr[1] === "" ? "0" : sgr[1]).split(";");
				for (let k = 0; k < params.length; k++) {
					const code = Number(params[k] === "" ? "0" : params[k]);
					if (code === 38 || code === 48) {
						const mode = Number(params[k + 1] ?? "0");
						const span = mode === 2 ? 5 : mode === 5 ? 3 : 1;
						if (code === 48) bg = params.slice(k, k + span).join(";");
						k += span - 1;
						continue;
					}
					if (code === 0 || code === 49) bg = "";
					else if ((code >= 40 && code <= 47) || (code >= 100 && code <= 107)) bg = String(code);
				}
				i += sgr[0].length;
				continue;
			}
			const osc = /^\x1b\][^\x07\x1b]*(\x07|\x1b\\)/.exec(line.slice(i));
			if (osc) {
				i += osc[0].length;
				continue;
			}
			const csi = /^\x1b\[[0-9;?]*[A-Za-z]/.exec(line.slice(i));
			if (csi) {
				i += csi[0].length;
				continue;
			}
			i += 1;
			continue;
		}
		// Grapheme clusters, the way visibleWidth measures them — splitting by
		// code point counts an emoji ZWJ sequence as several wide cells and
		// invents an overflow that is not on screen.
		const rest = line.slice(i);
		const nextEsc = rest.indexOf("\x1b");
		const chunk = nextEsc === -1 ? rest : rest.slice(0, nextEsc);
		const ch = [...SEGMENTER.segment(chunk)][0]!.segment;
		const w = Math.max(visibleWidth(ch), 1);
		for (let c = 0; c < w; c++) out.push({ bg, glyph: c === 0 ? ch : "" });
		i += ch.length;
	}
	return out;
}

const SHADOW_GLYPHS = new Set(["▏", "▔"]);

function faultsIn(label: string, lines: string[], width: number): string[] {
	const faults: string[] = [];
	const body = lines.filter((l) => scan(l).length > 0);
	if (body.length === 0) return faults;
	const shadowRun = body[body.length - 1];
	// Too narrow to carry a gutter, the box draws the plain full-width band.
	const sheet = scan(shadowRun).some((c) => SHADOW_GLYPHS.has(c.glyph));
	const band = sheet ? Math.max(1, width - PAPER_INSET) : width;
	if (!sheet) {
		for (const [i, line] of body.entries()) {
			const cells = scan(line);
			if (cells.length > width) faults.push(`row ${i}: ${cells.length} cells overflows width ${width}`);
			for (let c = 0; c < cells.length; c++) {
				if (cells[c].bg === "") faults.push(`row ${i}: hole in the plain band at cell ${c}`);
			}
		}
		return faults.map((fault) => `${label} @${width}: ${fault}`);
	}
	for (const [i, line] of body.entries()) {
		const cells = scan(line);
		const last = i === body.length - 1;
		if (cells.length > width) faults.push(`row ${i}: ${cells.length} cells overflows terminal width ${width}`);
		if (last) {
			// The bottom run: page at cell 0, shadow ink across 1..band.
			if (cells[0]?.glyph !== " ") faults.push(`bottom run: does not start on page`);
			if (cells.length !== band + 1) faults.push(`bottom run: ${cells.length} cells, expected ${band + 1}`);
			if (cells.at(-1)?.glyph !== "▏") faults.push(`bottom run: ends on ${cells.at(-1)?.glyph}, expected ▏`);
			continue;
		}
		// Where the sheet's fill stops. Every row stops in the same cell: the
		// edge is ruled, so a fill that ends short is a hole, not a nick.
		let fillEnd = 0;
		while (fillEnd < cells.length && cells[fillEnd].bg !== "" && !SHADOW_GLYPHS.has(cells[fillEnd].glyph)) fillEnd++;
		if (fillEnd !== band) {
			faults.push(`row ${i}: fill ends at ${fillEnd}, expected ${band}`);
		}
		for (let c = 0; c < fillEnd; c++) {
			if (cells[c].bg === "") faults.push(`row ${i}: hole in the fill at cell ${c}`);
		}
		// The shadow column — one half-cell of ink, and never a block of it
		// backfilling a gap the sheet left.
		const rest = cells
			.slice(fillEnd)
			.map((c) => c.glyph)
			.join("");
		if (i === 0) {
			if (rest !== "") faults.push(`row 0: casts a shadow (${JSON.stringify(rest)}) but should not`);
		} else if (rest !== "▏") {
			faults.push(`row ${i}: shadow segment is ${JSON.stringify(rest)}, expected "▏"`);
		}
	}
	void shadowRun;
	return faults.map((fault) => `${label} @${width}: ${fault}`);
}

const CONTENT: Array<[string, string]> = [
	["plain", "hello there"],
	["heading chip", "# Big Title\n\nbody"],
	["inline code and bold", "run `npm test` now, **really**"],
	["link", "see [docs](https://example.com/a/very/long/path) ok"],
	["code block", "```ts\nconst x = 1;\n```"],
	["code block, long line", "```ts\nconst identifier = someVeryLongFunctionName(argumentOne, argumentTwo);\n```"],
	["unbreakable word", "a".repeat(120)],
	["unbreakable word in code", `\`\`\`\n${"z".repeat(120)}\n\`\`\``],
	["unbreakable url", `https://example.com/${"segment/".repeat(20)}`],
	["cjk", "日本語のテキストがここにあります、もっと長くします"],
	["emoji, including zwj sequences", "done ✅ 🎉 shipped 👍🏽 family 👩‍👩‍👧‍👦 end"],
	["table", "| a | b |\n|---|---|\n| 1 | 2 |"],
	["table wider than the band", "| alpha | beta | gamma | delta |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |"],
	["blockquote", "> quoted line that wraps around here\n\nafter"],
	["horizontal rule", "above\n\n---\n\nbelow"],
	["list", "- one\n- two\n- three"],
	["nested list", "- one\n  - two\n    - three deeply nested item text"],
	["blank lines", "a\n\n\n\nb"],
	["trailing whitespace", "text with trailing   \n\nmore"],
	["a child that resets its own styling", "\x1b[31mred\x1b[0m then plain"],
];

/**
 * Full terminals, awkward ones, and widths narrower than the gutter itself.
 *
 * Stops at two columns: a CJK ideograph or an emoji occupies two of them and
 * cannot be split, so a one-column terminal overflows on content no layout can
 * make fit. Every width above that is the box's own to get right.
 */
const WIDTHS = [120, 80, 60, 40, 34, 24, 20, 16, 12, 10, 8, 6, 5, 4, 3, 2];

/**
 * What a sheet costs in columns, pinned to a number rather than to itself.
 *
 * Every other assertion in this file derives the band from `PAPER_INSET`, so
 * they all held while the gutter was three columns wide and two of them showed
 * nothing at all — the sheet stopped short and the page beside it was empty.
 * The treatment needs exactly one: the shadow's `▏` is a *left one-eighth* block, so
 * it paints the sheet's edge in the left half of that last cell and leaves the
 * right half as page. This is the test that fails if the gutter grows back.
 */
describe.each(["vox-cutout-dark", "vox-cutout-light"])("a sheet's reach on %s", (themeName) => {
	it.each([120, 100, 80, 60, 40])("runs to the terminal's last cell at width %i", (width) => {
		initTheme(themeName, false);
		const lines = new UserMessageComponent("a message").render(width).filter((line) => scan(line).length > 0);
		expect(PAPER_INSET).toBe(1);
		// The shadow is offset down *and* right, so the sheet's own top row is the
		// one row with nothing in the last cell. Every row under it — and the
		// bottom run — reaches it.
		expect(scan(lines[0]).length, "the top row").toBe(width - 1);
		for (const [i, line] of lines.slice(1).entries()) {
			expect(scan(line).length, `row ${i + 1} stops short of the margin`).toBe(width);
		}
	});
});

describe.each(["vox-cutout-dark", "vox-cutout-light"])("a sheet's fill on %s", (themeName) => {
	it.each(CONTENT)("holds its geometry at every width: %s", (label, text) => {
		initTheme(themeName, false);
		const faults: string[] = [];
		for (const width of WIDTHS) {
			faults.push(...faultsIn(`user "${label}"`, new UserMessageComponent(text).render(width), width));
			faults.push(
				...faultsIn(
					`extension "${label}"`,
					new CustomMessageComponent({
						role: "custom",
						customType: "skill",
						content: text,
						timestamp: 0,
					} as never).render(width),
					width,
				),
			);
			const box = new Box(1, 1);
			applyBlockFill(box, "warningBg");
			for (const line of text.split("\n")) box.addChild(new Text(line, 0, 0));
			faults.push(...faultsIn(`warning "${label}"`, box.render(width), width));
		}
		expect(faults).toEqual([]);
	});
});
