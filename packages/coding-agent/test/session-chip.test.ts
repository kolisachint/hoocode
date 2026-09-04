import { stripVTControlCharacters } from "node:util";
import { visibleWidth } from "@kolisachint/hoocode-tui";
import { beforeAll, describe, expect, it } from "vitest";
import {
	renderSessionChip,
	SESSION_CHIP_MIN_WIDTH,
	sessionChipFits,
} from "../src/modes/interactive/components/session-chip.js";
import {
	getThemeByName,
	initTheme,
	sessionColorToken,
	setThemeInstance,
} from "../src/modes/interactive/theme/theme.js";

describe("session chip", () => {
	beforeAll(() => {
		initTheme(undefined, false);
	});

	it("pads the name so the fill reads as a chip rather than as text", () => {
		const chip = renderSessionChip("refactor-auth", 1);
		expect(chip?.plain).toBe(" refactor-auth ");
	});

	// The plain/styled pair is what the editor does its border arithmetic on, so a
	// styled string that measures differently would push the corner off the row.
	it("keeps the styled string exactly as wide as the plain one", () => {
		for (let slot = 1; slot <= 6; slot++) {
			const chip = renderSessionChip("amber-harbor", slot);
			expect(visibleWidth(chip!.styled)).toBe(visibleWidth(chip!.plain));
		}
	});

	it("truncates a name too long to glance at", () => {
		const chip = renderSessionChip("an-extremely-long-session-name-nobody-can-scan", 1);
		expect(visibleWidth(chip!.plain)).toBeLessThanOrEqual(22);
		expect(stripVTControlCharacters(chip!.styled)).toContain("…");
	});

	it("has nothing to draw for an empty name", () => {
		expect(renderSessionChip("", 1)).toBeUndefined();
		expect(renderSessionChip("   ", 1)).toBeUndefined();
	});

	it("fills with the session's own colour", () => {
		const chip = renderSessionChip("refactor-auth", 3);
		const other = renderSessionChip("refactor-auth", 4);
		expect(chip!.styled).not.toBe(other!.styled);
	});

	// A light theme's palette is ink — dark enough to read on paper, which makes a
	// dark yellow into brown and a dark cyan into navy. The chip lifts the fill off
	// that ink so the hue reads as itself, and takes its ink from the lifted fill.
	it("lifts a light theme's fill off the ink it is drawn from", () => {
		// Written in both encodings because a fill is emitted as truecolour or as a
		// 256-colour index depending on what the terminal running the tests
		// advertises; the colour it stands for is the same either way.
		const bgOf = (styled: string) => /\x1b\[(48;[^m]*)m/.exec(styled)?.[1];
		const inkOf = (styled: string) => /\x1b\[(38;[^m]*)m/.exec(styled)?.[1];
		const DARK_INK = ["38;2;11;11;15", "38;5;232"];

		const dark = getThemeByName("dark");
		const light = getThemeByName("light");
		expect(dark).toBeDefined();
		expect(light).toBeDefined();

		setThemeInstance(dark!);
		const onDark = renderSessionChip("refactor-auth", 1)!.styled;
		const darkToken = dark!.getFgAnsi(sessionColorToken(1)).replace("38;", "48;");

		setThemeInstance(light!);
		const onLight = renderSessionChip("refactor-auth", 1)!.styled;
		const lightToken = light!.getFgAnsi(sessionColorToken(1)).replace("38;", "48;");

		// A dark theme's palette is already bright, so the chip is filled with the
		// token itself and nothing is lifted.
		expect(`\x1b[${bgOf(onDark)}m`).toBe(darkToken);
		// A light theme's is not, so the fill is a lifted version of it — same hue,
		// enough lightness to be recognisable as that hue.
		expect(`\x1b[${bgOf(onLight)}m`).not.toBe(lightToken);

		// Either way the ink is picked from whatever the chip ended up filled with.
		expect(DARK_INK).toContain(inkOf(onDark));
		expect(DARK_INK).toContain(inkOf(onLight));

		initTheme(undefined, false);
	});

	it("can fill every slot in every shipped theme", () => {
		for (const name of [
			"dark",
			"light",
			"warm-dark",
			"warm-light",
			"vox-dark",
			"vox-light",
			"vox-cutout-dark",
			"vox-cutout-light",
			"solarized-dark",
			"solarized-light",
			"colorsafe-dark",
			"colorsafe-light",
			"high-contrast-dark",
			"high-contrast-light",
		]) {
			const themeInstance = getThemeByName(name);
			expect(themeInstance, name).toBeDefined();
			for (let slot = 1; slot <= 6; slot++) {
				expect(themeInstance!.canFill(sessionColorToken(slot)), `${name} slot ${slot}`).toBe(true);
			}
		}
	});

	it("drops the chip only where the box has no room for it", () => {
		expect(sessionChipFits(SESSION_CHIP_MIN_WIDTH)).toBe(true);
		expect(sessionChipFits(SESSION_CHIP_MIN_WIDTH - 1)).toBe(false);
		expect(sessionChipFits(200)).toBe(true);
	});
});
