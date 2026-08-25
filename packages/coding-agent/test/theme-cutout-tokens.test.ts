import { Box, Text } from "@kolisachint/hoocode-tui";
import { beforeEach, describe, expect, it } from "vitest";
import {
	getMarkdownTheme,
	getPaperShadowFn,
	initTheme,
	messageLabel,
	theme,
} from "../src/modes/interactive/theme/theme.js";

/**
 * The four cut-out token groups are optional, and "optional" is a promise about
 * every theme that does not set them: `dark` has to render exactly as it did
 * before the tokens existed. Each group is checked from both sides — the
 * fallback on a theme without it, the effect on a theme with it — because a
 * fallback that is never exercised is the one that rots.
 */
describe("cut-out token fallbacks", () => {
	describe("a theme without them", () => {
		beforeEach(() => initTheme("dark", false));

		it("draws no shadow pass", () => {
			expect(getPaperShadowFn()).toBeUndefined();
		});

		it("adds no shadow row to a filled box", () => {
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			box.setShadowFn(getPaperShadowFn());
			box.addChild(new Text("hello", 0, 0));
			// One padding row, one content row, one padding row. Nothing else.
			expect(box.render(20)).toHaveLength(3);
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
	});

	describe.each(["vox-cutout-light", "vox-cutout-dark"])("%s", (themeName) => {
		beforeEach(() => initTheme(themeName, false));

		it("draws a shadow row under a filled box", () => {
			const box = new Box(1, 1, (t) => theme.bg("userMessageBg", t));
			box.setShadowFn(getPaperShadowFn());
			box.addChild(new Text("hello", 0, 0));
			const lines = box.render(20);
			expect(lines).toHaveLength(4);
			// Indented one cell to give the offset, and one cell shorter than the
			// block so it stays inside the terminal's last column.
			const shadow = lines[3];
			expect(shadow.startsWith(" ")).toBe(true);
			expect(shadow.match(/▀/g)).toHaveLength(19);
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
