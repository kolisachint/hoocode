import { readFileSync } from "fs";
import { describe, expect, it } from "vitest";
import {
	getAvailableThemes,
	getResolvedThemeColors,
	getThemeByName,
	getThemeExportColors,
	type ThemeColor,
} from "../src/modes/interactive/theme/theme.js";

/**
 * The themes built for low-vision use. Everything they draw has to clear WCAG
 * AAA (7:1) against every surface the TUI paints behind it, so a theme change
 * cannot quietly reintroduce a washed-out token.
 */
const ACCESSIBLE_THEMES = [
	"colorsafe-dark",
	"colorsafe-light",
	"high-contrast-dark",
	"high-contrast-light",
	"warm-dark",
	"warm-light",
] as const;

const DARK_THEMES = ACCESSIBLE_THEMES.filter((name) => name.endsWith("-dark"));
const LIGHT_THEMES = ACCESSIBLE_THEMES.filter((name) => name.endsWith("-light"));

/** AAA for body text. Large text would allow 4.5, but the TUI has none. */
const MIN_CONTRAST = 7;

const BG_TOKENS = [
	"selectedBg",
	"userMessageBg",
	"customMessageBg",
	"toolPendingBg",
	"toolSuccessBg",
	"toolErrorBg",
] as const;

const AGENT_TOKENS = ["agent1", "agent2", "agent3", "agent4", "agent5", "agent6"];

const schema = JSON.parse(
	readFileSync(new URL("../src/modes/interactive/theme/theme-schema.json", import.meta.url), "utf-8"),
) as { properties: { colors: { required: string[] } } };

/** Every token a theme must carry: the 51 required by the schema plus the optional palette. */
const ALL_TOKENS = [...schema.properties.colors.required, ...AGENT_TOKENS, "mcp"];

function luminance(hex: string): number {
	const value = hex.replace("#", "");
	const toLinear = (channel: number) => {
		const s = channel / 255;
		return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
	};
	const [r, g, b] = [0, 2, 4].map((offset) => toLinear(Number.parseInt(value.slice(offset, offset + 2), 16)));
	return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
	const [la, lb] = [luminance(a), luminance(b)];
	return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** Every surface a theme paints text on: the six message/tool backgrounds plus the export canvases. */
function surfaces(themeName: string): Record<string, string> {
	const colors = getResolvedThemeColors(themeName);
	const exported = getThemeExportColors(themeName);
	const result: Record<string, string> = {};
	for (const token of BG_TOKENS) result[token] = colors[token];
	for (const [key, value] of Object.entries(exported)) {
		if (value) result[`export.${key}`] = value;
	}
	return result;
}

describe("accessible themes", () => {
	it("ships three light and three dark themes alongside the originals", () => {
		const available = getAvailableThemes();
		expect(available).toEqual(expect.arrayContaining([...ACCESSIBLE_THEMES, "dark", "light"]));
		expect(DARK_THEMES).toHaveLength(3);
		expect(LIGHT_THEMES).toHaveLength(3);
	});

	it.each(ACCESSIBLE_THEMES)("%s defines every color token explicitly", (themeName) => {
		const raw = JSON.parse(
			readFileSync(new URL(`../src/modes/interactive/theme/${themeName}.json`, import.meta.url), "utf-8"),
		) as { name: string; colors: Record<string, string | number> };

		expect(raw.name).toBe(themeName);
		const missing = ALL_TOKENS.filter((token) => raw.colors[token] === undefined);
		expect(missing).toEqual([]);
		// "" means "whatever the terminal defaults to", which is exactly the
		// unknown-contrast case these themes exist to remove.
		const terminalDefaults = ALL_TOKENS.filter((token) => raw.colors[token] === "");
		expect(terminalDefaults).toEqual([]);
	});

	it.each(ACCESSIBLE_THEMES)("%s renders every token without falling back", (themeName) => {
		const theme = getThemeByName(themeName);
		expect(theme).toBeDefined();
		for (const token of ALL_TOKENS) {
			if ((BG_TOKENS as readonly string[]).includes(token)) {
				expect(() => theme?.bg(token as (typeof BG_TOKENS)[number], "x")).not.toThrow();
			} else {
				expect(() => theme?.fg(token as ThemeColor, "x")).not.toThrow();
			}
		}
	});

	it.each(ACCESSIBLE_THEMES)("%s keeps every foreground at AAA contrast on every surface", (themeName) => {
		const colors = getResolvedThemeColors(themeName);
		const backgrounds = surfaces(themeName);

		const failures: string[] = [];
		for (const [token, value] of Object.entries(colors)) {
			if ((BG_TOKENS as readonly string[]).includes(token)) continue;
			for (const [surface, background] of Object.entries(backgrounds)) {
				const ratio = contrast(value, background);
				if (ratio < MIN_CONTRAST) {
					failures.push(`${token} (${value}) on ${surface} (${background}): ${ratio.toFixed(2)}:1`);
				}
			}
		}
		expect(failures).toEqual([]);
	});

	it.each(ACCESSIBLE_THEMES)("%s keeps its surfaces on one side of the light/dark split", (themeName) => {
		const isLight = themeName.endsWith("-light");
		const wrongSide = Object.entries(surfaces(themeName))
			.map(([surface, background]) => [surface, luminance(background)] as const)
			.filter(([, value]) => (isLight ? value <= 0.5 : value >= 0.5))
			.map(([surface, value]) => `${surface}: ${value.toFixed(3)}`);
		expect(wrongSide).toEqual([]);
	});

	it.each(ACCESSIBLE_THEMES)("%s marks selection and tool state visibly against the page", (themeName) => {
		const colors = getResolvedThemeColors(themeName);
		const pageBg = getThemeExportColors(themeName).pageBg;
		expect(pageBg).toBeDefined();
		// A selected row a low-vision user cannot see is as broken as unreadable
		// text, so the highlight has to separate from the page it sits on.
		expect(contrast(colors.selectedBg, pageBg as string)).toBeGreaterThan(1.2);
	});
});
