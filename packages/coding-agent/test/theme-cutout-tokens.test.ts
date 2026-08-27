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
			expect(md.headingBlock).toBeUndefined();
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
			for (const line of lines.slice(0, 3)) {
				// A cut edge takes at most one column, and only out of padding.
				expect(visibleWidth(line)).toBeGreaterThanOrEqual(band - 1);
				expect(visibleWidth(line)).toBeLessThanOrEqual(band + 1);
			}
			// Rows below the first cast a shadow along the right edge; the first
			// does not, because the offset is down as well as right.
			expect(lines[0]).not.toContain("▌");
			expect(lines[1] + lines[2]).toContain("▌");
			// And the bottom run is offset one column right of the band, ending
			// under the right-hand column so the two close the corner.
			expect(lines[3].startsWith(" ")).toBe(true);
			expect(lines[3].match(/▀/g)).toHaveLength(band);
			expect(visibleWidth(lines[3])).toBe(band + 1);
		});

		it("holds the shadow in one column, whatever the cut does to the edge", () => {
			// A shadow that stepped in and out with the nick was not a shadow:
			// the glyph is half a cell wide, so a one-column step leaves no
			// overlap between one row's mark and the next, and the edge reads as
			// a dashed staircase. Every shadowed row ends in the same cell.
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			applyPaperSheet(box);
			for (let i = 0; i < 12; i++) {
				box.addChild(new Text(`row ${i}`, 0, 0));
			}
			const lines = box.render(40);
			const edge = 40 - PAPER_INSET + 1;
			for (const line of lines.slice(1)) {
				expect(visibleWidth(line)).toBe(edge);
			}
			expect(lines.slice(1, -1).every((line) => line.includes("▌"))).toBe(true);
		});

		it("still cuts the sheet's own edge", () => {
			// The nick moved off the shadow, not out of the theme: with the
			// shadow's column left off, the rows that took one are narrower.
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			box.setInset(PAPER_INSET);
			box.setCutEdge(true);
			for (let i = 0; i < 12; i++) {
				box.addChild(new Text(`row ${i}`, 0, 0));
			}
			const band = 40 - PAPER_INSET;
			const widths = new Set(box.render(40).map((line) => visibleWidth(line)));
			expect(widths).toEqual(new Set([band, band - 1]));
		});

		it("never lets the cut edge eat a character", () => {
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
