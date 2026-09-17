import { Box, Text, visibleWidth } from "@kolisachint/hoocode-tui";
import stripAnsi from "strip-ansi";
import { beforeEach, describe, expect, it } from "vitest";
import { renderToolSignalLine, type ToolSignalInput } from "../src/modes/interactive/components/tool-signal.js";
import {
	applyPaperSheet,
	getMarkdownTheme,
	getPaperShadowFn,
	initTheme,
	messageLabel,
	PAPER_INSET,
	setTheme,
	theme,
} from "../src/modes/interactive/theme/theme.js";

/**
 * The four cut-out token groups are optional, and "optional" is a promise about
 * every theme that does not set them: `dark` has to render exactly as it did
 * before the tokens existed. Each group is checked from both sides — the
 * fallback on a theme without it, the effect on a theme with it — because a
 * fallback that is never exercised is the one that rots.
 */
/** One radar row's worth of input, so the stroke tests differ only in `isLatest`. */
const SIGNAL: ToolSignalInput = {
	toolName: "read",
	args: { path: "src/modes/interactive/theme/theme.ts" },
	cwd: "/repo",
	result: { content: [{ type: "text", text: "a\nb\nc" }], isError: false },
	isPartial: false,
	showImages: false,
};

describe("cut-out token fallbacks", () => {
	describe("a theme without them", () => {
		beforeEach(() => initTheme("dark", false));

		it("draws no shadow pass", () => {
			expect(getPaperShadowFn()).toBeUndefined();
		});

		it("adds no shadow row to a filled box, and no gutter", () => {
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			applyPaperSheet(box);
			box.addChild(new Text("hello", 0, 0));
			// One padding row, one content row, one padding row. Nothing else.
			const lines = box.render(20);
			expect(lines).toHaveLength(3);
			// And the band still runs the full width it always did.
			expect(lines.map((l) => visibleWidth(l))).toEqual([20, 20, 20]);
		});

		it("leaves headings as coloured text", () => {
			const md = getMarkdownTheme();
			// The block pass exists but does nothing: it is what a theme switch
			// goes through, and a heading laid out under this theme has to come
			// back exactly as it went in.
			expect(md.headingBlock?.("Title", 2)).toBe("Title");
			expect(md.heading("Title", 2)).toBe(theme.fg("mdHeading", "Title"));
		});

		it("keeps the bracketed message label", () => {
			expect(messageLabel("skill")).toBe(theme.fg("customMessageLabel", "\x1b[1m[skill]\x1b[22m"));
		});

		it("paints a gauge track in dim", () => {
			expect(theme.fg("halftone", "▱")).toBe(theme.fg("dim", "▱"));
		});

		it("draws no marker stroke on the newest radar row", () => {
			expect(theme.hasBg("activeToolBg")).toBe(false);
			const marked = renderToolSignalLine({ ...SIGNAL, isLatest: true }, 60);
			const plain = renderToolSignalLine({ ...SIGNAL, isLatest: false }, 60);
			expect(marked).toBe(plain);
		});
	});

	describe.each(["vox-cutout-light", "vox-cutout-dark"])("%s", (themeName) => {
		beforeEach(() => initTheme(themeName, false));

		it("cuts the block into a sheet with a shadow on both edges", () => {
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			applyPaperSheet(box);
			box.addChild(new Text("hello", 0, 0));
			const lines = box.render(40);
			expect(lines).toHaveLength(4);

			// The band holds back from the right margin, so the sheet has an edge.
			const band = 40 - PAPER_INSET;
			// The top row is the band alone; the rows under it add the column.
			expect(visibleWidth(lines[0])).toBe(band);
			for (const line of lines.slice(1, 3)) {
				expect(visibleWidth(line)).toBe(band + 1);
			}
			// Rows below the first cast a shadow along the right edge; the first
			// does not, because the offset is down as well as right.
			expect(lines[0]).not.toContain("▏");
			expect(lines[1] + lines[2]).toContain("▏");
			// And the bottom run is offset one column right of the band, stopping
			// where the right-hand column starts so the two meet as an L.
			expect(lines[3].startsWith(" ")).toBe(true);
			expect(lines[3].match(/▔/g)).toHaveLength(band - 1);
			expect(visibleWidth(lines[3])).toBe(band);
		});

		it("closes the bottom corner flush with the shadow's column", () => {
			// Neither glyph can be asked to draw the corner. `▔` fills a cell edge
			// to edge, so a run that took the column's cell overshot it by seven
			// eighths — a tip poking out past the corner. `▏` fills the full height
			// of a cell while `▔` fills the top eighth of one, so carrying `▏` down
			// into the run's row overshot downwards instead: a tick hanging a whole
			// row below the shadow's bottom edge. The run stops where the column
			// starts, and that is the corner.
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			applyPaperSheet(box);
			box.addChild(new Text("hello", 0, 0));
			const lines = box.render(40);
			const run = stripAnsi(lines[3]);
			expect(run).not.toContain("▏");
			// The run's right end is the cell the column above it occupies.
			expect(visibleWidth(run)).toBe(stripAnsi(lines[2]).indexOf("▏"));
		});

		it("rules the sheet's right edge instead of nicking it", () => {
			// The edge used to step one column in on roughly every fifth row, to
			// read as cut by hand. A terminal cell is far too coarse a step for
			// that, so it landed as damage: a bite out of the sheet's top-right
			// corner on the one row with no shadow behind it, and everywhere
			// else a tooth of shadow ink backfilling the gap — the fill leaving
			// holes and the shadow covering for them. Every row now ends in the
			// same cell, and the shadow never reaches inside the sheet.
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			applyPaperSheet(box);
			for (let i = 0; i < 24; i++) {
				box.addChild(new Text(`row ${i}`, 0, 0));
			}
			const rendered = box.render(40);
			const band = 40 - PAPER_INSET;
			// No block of shadow ink anywhere: that glyph only ever existed to
			// plug a nick.
			expect(rendered.some((line) => stripAnsi(line).includes("█"))).toBe(false);
			// The top row is the full band, with no shadow beside it, so its
			// corner is whole.
			expect(visibleWidth(rendered[0])).toBe(band);
			expect(rendered[0]).not.toContain("▏");
			// And every row below it is band plus exactly one column of shadow,
			// in the same cell every time.
			for (const line of rendered.slice(1, -1)) {
				expect(visibleWidth(line)).toBe(band + 1);
				expect(stripAnsi(line).indexOf("▏")).toBe(band);
			}
		});

		it("holds every row of the sheet to one width", () => {
			// A shadow that stepped in and out was not a shadow: the glyph is a
			// fraction of a cell wide, so a one-column step leaves no overlap
			// between one row's mark and the next, and the edge reads as a dashed
			// staircase. Every shadowed row ends in the same cell.
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			applyPaperSheet(box);
			for (let i = 0; i < 12; i++) {
				box.addChild(new Text(`row ${i}`, 0, 0));
			}
			const lines = box.render(40);
			const edge = 40 - PAPER_INSET + 1;
			for (const line of lines.slice(1, -1)) {
				expect(visibleWidth(line)).toBe(edge);
			}
			// All but the run, which stops one cell short — that cell is the column's.
			expect(visibleWidth(lines[lines.length - 1])).toBe(edge - 1);
			expect(lines.slice(1, -1).every((line) => line.includes("▏"))).toBe(true);
		});

		it("keeps the gutter off the content", () => {
			const text = "x".repeat(30);
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			applyPaperSheet(box);
			box.addChild(new Text(text, 0, 0));
			const rendered = box.render(40).join("\n");
			expect(stripAnsi(rendered)).toContain(text);
		});

		it("renders headings as a filled chip", () => {
			const md = getMarkdownTheme();
			expect(md.headingBlock).toBeDefined();
			expect(md.heading("Title", 2)).toBe(theme.fg("headlineText", "Title"));
			// The padding lives on the finished line, never in `heading` itself:
			// that one is also called with a sentinel to extract an ANSI prefix for
			// inline-token restores, and a literal space there would be spliced
			// back in around every codespan inside the heading.
			expect(md.heading("Title", 2)).not.toContain(" T");
			expect(md.headingBlock?.("Title", 2)).toBe(theme.bg("headlineBg", " Title "));
		});

		it("leaves headings below h2 as coloured text", () => {
			// The renderer keeps the `###` marker at these levels, and a marker
			// inside a filled chip says the same thing twice.
			const md = getMarkdownTheme();
			expect(md.heading("Deeper", 3)).toBe(theme.fg("mdHeading", "Deeper"));
			expect(md.headingBlock?.("Deeper", 3)).toBe("Deeper");
		});

		it("renders the message label as a tape strip", () => {
			const label = messageLabel("skill");
			expect(label).toBe(theme.bg("tapeBg", theme.fg("tapeText", "\x1b[1m skill \x1b[22m")));
			expect(label).not.toContain("[skill]");
		});

		it("paints a gauge track apart from dim", () => {
			expect(theme.fg("halftone", "▱")).not.toBe(theme.fg("dim", "▱"));
		});

		it("strokes the newest radar row and nothing else", () => {
			const marked = renderToolSignalLine({ ...SIGNAL, isLatest: true }, 60);
			const plain = renderToolSignalLine({ ...SIGNAL, isLatest: false }, 60);
			expect(marked).not.toBe(plain);

			const sentinel = "\u0000";
			const opener = theme.bg("activeToolBg", sentinel).split(sentinel)[0];
			expect(marked).toContain(opener);
			expect(plain).not.toContain(opener);

			// A highlighter runs over the words, not the whole line: the stroke
			// closes before the flush-right signal, which keeps its status colour.
			const close = marked.lastIndexOf("\x1b[49m");
			expect(close).toBeGreaterThan(-1);
			expect(marked.slice(close)).toContain("lines");
			// And it changes no visible character — only how they are painted.
			expect(stripAnsi(marked)).toBe(stripAnsi(plain));
		});
	});

	describe("a theme that sets only half a chip pair", () => {
		it("falls back rather than rendering a half-styled chip", () => {
			// hasBg/has are what the renderers gate on, so a theme missing either
			// half takes the same path as a theme missing both.
			initTheme("dark", false);
			expect(theme.hasBg("headlineBg")).toBe(false);
			expect(theme.has("headlineText")).toBe(false);
			expect(theme.hasBg("tapeBg")).toBe(false);
			expect(theme.has("tapeText")).toBe(false);
		});
	});
});

/**
 * A background is a pair, so a chip painted inside a filled block has to close
 * its own fill — and that close ended the block's fill too, leaving the row
 * bare from the chip to the right margin. Unit-testing the tokens in isolation
 * could not see it; rendering a real message block does.
 */
describe("a chip painted inside a filled block", () => {
	beforeEach(() => initTheme("vox-cutout-light", false));

	it("does not punch a hole in the block behind it", () => {
		// The block's opener, recovered the way the renderer recovers it.
		const sentinel = "\u0000";
		const blockOpener = theme.bg("customMessageBg", sentinel).split(sentinel)[0];
		expect(blockOpener).not.toBe("");

		const box = new Box(1, 1, (t) => theme.bg("customMessageBg", t));
		box.addChild(new Text(messageLabel("extension"), 0, 0));
		const line = box.render(40).find((l) => l.includes("extension")) as string;
		expect(line).toBeDefined();

		// Walk the row: after the chip closes its own fill, the block's opener has
		// to appear again before any run of padding spaces, or the row finishes on
		// the terminal's canvas instead of on the block.
		const chipClose = line.indexOf("\x1b[49m");
		expect(chipClose).toBeGreaterThan(-1);
		const tail = line.slice(chipClose + "\x1b[49m".length);
		expect(tail.startsWith(blockOpener)).toBe(true);
		// And the row still ends by closing the fill exactly once more.
		expect(tail.endsWith("\x1b[49m")).toBe(true);
	});
});

/**
 * The tokens are optional per *theme*, and a session is not one theme: the user
 * switches with the transcript already on screen, and every block up there was
 * built under whichever theme was current at the time. Anything that reads an
 * optional token has to read it at render time — a decision frozen when the
 * block was built goes on asking a theme that never defined the token, which is
 * a crash, not a fallback.
 */
describe("switching theme under blocks already on screen", () => {
	const sheet = () => {
		const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
		applyPaperSheet(box);
		box.addChild(new Text("hello", 0, 0));
		return box;
	};

	it("drops the paper treatment when leaving a cut-out theme", () => {
		initTheme("vox-cutout-dark", false);
		const box = sheet();
		expect(box.render(40)).toHaveLength(4);

		setTheme("dark", false);
		// Shadow row gone, gutter gone: the block a plain theme would have built.
		const lines = box.render(40);
		expect(lines).toHaveLength(3);
		expect(lines.map((line) => visibleWidth(line))).toEqual([40, 40, 40]);
	});

	it("picks the paper treatment up when entering one", () => {
		initTheme("dark", false);
		const box = sheet();
		expect(box.render(40)).toHaveLength(3);

		setTheme("vox-cutout-dark", false);
		const lines = box.render(40);
		expect(lines).toHaveLength(4);
		expect(visibleWidth(lines[3])).toBe(40 - PAPER_INSET);
	});

	it("keeps a markdown theme rendering across the switch, both ways", () => {
		initTheme("vox-cutout-dark", false);
		const fromCutout = getMarkdownTheme();
		setTheme("dark", false);
		expect(fromCutout.heading("Title", 2)).toBe(theme.fg("mdHeading", "Title"));
		expect(fromCutout.headingBlock?.("Title", 2)).toBe("Title");

		const fromPlain = getMarkdownTheme();
		setTheme("vox-cutout-dark", false);
		expect(fromPlain.heading("Title", 2)).toBe(theme.fg("headlineText", "Title"));
		expect(fromPlain.headingBlock?.("Title", 2)).toBe(theme.bg("headlineBg", " Title "));
	});
});
