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
			expect(md.heading("Title")).toBe(theme.fg("mdHeading", "Title"));
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
			expect(md.heading("Title")).toBe(theme.fg("headlineText", "Title"));
			// The padding lives on the finished line, never in `heading` itself:
			// that one is also called with a sentinel to extract an ANSI prefix for
			// inline-token restores, and a literal space there would be spliced
			// back in around every codespan inside the heading.
			expect(md.heading("Title")).not.toContain(" T");
			expect(md.headingBlock?.("Title")).toBe(theme.bg("headlineBg", " Title "));
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
