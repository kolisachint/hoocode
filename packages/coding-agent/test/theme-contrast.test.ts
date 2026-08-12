import { readFileSync } from "fs";
import { describe, expect, it } from "vitest";
import {
	getAvailableThemes,
	getResolvedThemeColors,
	getThemeByName,
	getThemeExportColors,
	loadThemeFromPath,
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

/** HSL saturation (0-1). How much hue a color actually carries, independent of how dark it is. */
function saturation(hex: string): number {
	const value = hex.replace("#", "");
	const [r, g, b] = [0, 2, 4].map((offset) => Number.parseInt(value.slice(offset, offset + 2), 16) / 255);
	const max = Math.max(r, g, b);
	const min = Math.min(r, g, b);
	if (max === min) return 0;
	return (max - min) / (1 - Math.abs(max + min - 1));
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

/**
 * Tokens the default light theme draws as rules and inactive chrome — an editor
 * border, a horizontal rule, the "thinking off" marker. They are meant to
 * recede, so they answer to a separation bar rather than a text-contrast one.
 */
const LIGHT_DECORATIVE_TOKENS = new Set(["borderMuted", "mdHr", "thinkingOff"]);

/**
 * Tokens the default light theme uses to carry meaning through hue — success vs
 * error, a link, an agent's identity. On a light backdrop a hue only reads if it
 * is saturated; the washed-out pastels this theme used to ship blurred into the
 * neutral grays around them.
 */
const LIGHT_HUE_TOKENS = [
	"accent",
	"border",
	"borderAccent",
	"success",
	"error",
	"warning",
	"customMessageLabel",
	"mdHeading",
	"mdLink",
	"mdCode",
	"mdQuoteBorder",
	"mdListBullet",
	"toolDiffAdded",
	"toolDiffRemoved",
	"syntaxComment",
	"syntaxKeyword",
	"syntaxFunction",
	"syntaxVariable",
	"syntaxString",
	"syntaxNumber",
	"syntaxType",
	"thinkingLow",
	"thinkingMedium",
	"thinkingHigh",
	"thinkingXhigh",
	"bashMode",
	...AGENT_TOKENS,
	"mcp",
];

describe("default light theme", () => {
	/** AA for body text. The accessible light themes exist for the AAA bar. */
	const MIN_TEXT_CONTRAST = 4.5;
	/** Rules and inactive chrome only have to separate from the surface behind them. */
	const MIN_DECORATIVE_CONTRAST = 2.8;
	/** Below this a hue reads as gray once it is dark enough to be legible on white. */
	const MIN_SATURATION = 0.35;

	it("keeps every foreground legible on every surface it paints", () => {
		const colors = getResolvedThemeColors("light");
		const backgrounds = surfaces("light");

		const failures: string[] = [];
		for (const [token, value] of Object.entries(colors)) {
			if ((BG_TOKENS as readonly string[]).includes(token)) continue;
			const minimum = LIGHT_DECORATIVE_TOKENS.has(token) ? MIN_DECORATIVE_CONTRAST : MIN_TEXT_CONTRAST;
			for (const [surface, background] of Object.entries(backgrounds)) {
				const ratio = contrast(value, background);
				if (ratio < minimum) {
					failures.push(`${token} (${value}) on ${surface} (${background}): ${ratio.toFixed(2)}:1`);
				}
			}
		}
		expect(failures).toEqual([]);
	});

	it("keeps meaning-carrying tokens saturated enough to read as color", () => {
		const colors = getResolvedThemeColors("light");
		const washedOut = LIGHT_HUE_TOKENS.filter((token) => saturation(colors[token]) < MIN_SATURATION).map(
			(token) => `${token} (${colors[token]}): ${(saturation(colors[token]) * 100).toFixed(0)}% saturation`,
		);
		expect(washedOut).toEqual([]);
	});

	it("keeps colors that mean different things apart after the 256-color downgrade", () => {
		// Apple Terminal, GNU screen and TERM=linux get the quantized palette, and
		// two hues that round to the same cube index become the same color there.
		// These groups are the ones a user reads against each other.
		const groups = {
			"core UI": ["accent", "success", "error", "warning", "border", "customMessageLabel"],
			"agent identity": [...AGENT_TOKENS, "mcp"],
			syntax: [
				"syntaxComment",
				"syntaxKeyword",
				"syntaxFunction",
				"syntaxVariable",
				"syntaxString",
				"syntaxNumber",
				"syntaxType",
			],
		};
		const quantized = loadThemeFromPath(
			new URL("../src/modes/interactive/theme/light.json", import.meta.url).pathname,
			"256color",
		);
		const indexOf = (token: string) => quantized.getFgAnsi(token as ThemeColor);

		for (const [label, tokens] of Object.entries(groups)) {
			const byIndex = new Map<string, string[]>();
			for (const token of tokens) {
				const index = indexOf(token);
				byIndex.set(index, [...(byIndex.get(index) ?? []), token]);
			}
			const collisions = [...byIndex.values()].filter((members) => members.length > 1).map((m) => m.join(" = "));
			expect(collisions, label).toEqual([]);
		}
	});

	it("marks selection visibly against the page without darkening it into text territory", () => {
		const colors = getResolvedThemeColors("light");
		const pageBg = getThemeExportColors("light").pageBg as string;
		expect(pageBg).toBeDefined();
		expect(contrast(colors.selectedBg, pageBg)).toBeGreaterThan(1.2);
		expect(luminance(colors.selectedBg)).toBeGreaterThan(0.5);
	});
});

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
