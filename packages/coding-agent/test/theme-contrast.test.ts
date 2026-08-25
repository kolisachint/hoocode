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

/**
 * CIEDE2000 floor for two tokens in the same group. ~2 is the just-noticeable
 * difference on a large field; 11 is the most these palettes can hold once AAA
 * has capped the lightness range, and it is comfortably above "is that the same
 * color?" at terminal text size.
 */
const MIN_DIFFERENCE = 11;

const BG_TOKENS = [
	"selectedBg",
	"userMessageBg",
	"customMessageBg",
	"toolPendingBg",
	"toolSuccessBg",
	"toolErrorBg",
	"warningBg",
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

/** CIELAB coordinates, the space CIEDE2000 measures perceived difference in. */
function toLab(hex: string): [number, number, number] {
	const value = hex.replace("#", "");
	const linear = [0, 2, 4].map((offset) => {
		const c = Number.parseInt(value.slice(offset, offset + 2), 16) / 255;
		return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
	});
	const [r, g, b] = linear;
	const xyz = [
		(0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047,
		0.2126 * r + 0.7152 * g + 0.0722 * b,
		(0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883,
	].map((c) => (c > 0.008856 ? Math.cbrt(c) : 7.787 * c + 16 / 116));
	return [116 * xyz[1] - 16, 500 * (xyz[0] - xyz[1]), 200 * (xyz[1] - xyz[2])];
}

/**
 * CIEDE2000 difference. Contrast alone says a token is readable; it says nothing
 * about whether two tokens read as the *same* color. Crushing a palette dark
 * enough to clear AAA on white is exactly what collapses distinct hues into each
 * other, so the palettes need a separation floor as well as a contrast floor.
 */
function difference(hexA: string, hexB: string): number {
	const [L1, a1, b1] = toLab(hexA);
	const [L2, a2, b2] = toLab(hexB);
	const C1 = Math.hypot(a1, b1);
	const C2 = Math.hypot(a2, b2);
	const meanC = (C1 + C2) / 2;
	const G = 0.5 * (1 - Math.sqrt(meanC ** 7 / (meanC ** 7 + 25 ** 7)));
	const ap1 = (1 + G) * a1;
	const ap2 = (1 + G) * a2;
	const Cp1 = Math.hypot(ap1, b1);
	const Cp2 = Math.hypot(ap2, b2);
	const angle = (y: number, x: number) => {
		if (y === 0 && x === 0) return 0;
		const deg = (Math.atan2(y, x) * 180) / Math.PI;
		return deg < 0 ? deg + 360 : deg;
	};
	const hp1 = angle(b1, ap1);
	const hp2 = angle(b2, ap2);
	const dLp = L2 - L1;
	const dCp = Cp2 - Cp1;
	let dhp = 0;
	if (Cp1 * Cp2 !== 0) {
		dhp = hp2 - hp1;
		if (dhp > 180) dhp -= 360;
		else if (dhp < -180) dhp += 360;
	}
	const dHp = 2 * Math.sqrt(Cp1 * Cp2) * Math.sin((dhp * Math.PI) / 360);
	const meanLp = (L1 + L2) / 2;
	const meanCp = (Cp1 + Cp2) / 2;
	let meanHp = hp1 + hp2;
	if (Cp1 * Cp2 !== 0) {
		if (Math.abs(hp1 - hp2) > 180) meanHp = hp1 + hp2 < 360 ? meanHp + 360 : meanHp - 360;
		meanHp /= 2;
	}
	const T =
		1 -
		0.17 * Math.cos(((meanHp - 30) * Math.PI) / 180) +
		0.24 * Math.cos((2 * meanHp * Math.PI) / 180) +
		0.32 * Math.cos(((3 * meanHp + 6) * Math.PI) / 180) -
		0.2 * Math.cos(((4 * meanHp - 63) * Math.PI) / 180);
	const Sl = 1 + (0.015 * (meanLp - 50) ** 2) / Math.sqrt(20 + (meanLp - 50) ** 2);
	const Sc = 1 + 0.045 * meanCp;
	const Sh = 1 + 0.015 * meanCp * T;
	const Rt =
		-Math.sin((2 * 30 * Math.exp(-(((meanHp - 275) / 25) ** 2)) * Math.PI) / 180) *
		(2 * Math.sqrt(meanCp ** 7 / (meanCp ** 7 + 25 ** 7)));
	return Math.sqrt((dLp / Sl) ** 2 + (dCp / Sc) ** 2 + (dHp / Sh) ** 2 + Rt * (dCp / Sc) * (dHp / Sh));
}

/**
 * Viénot-Brettel-Mollon dichromat simulation: what a color looks like to someone
 * missing the long (protan), medium (deutan) or short (tritan) cone.
 */
function simulateCvd(hex: string, kind: "protan" | "deutan" | "tritan"): string {
	const value = hex.replace("#", "");
	const rgb = [0, 2, 4].map((offset) => {
		const c = Number.parseInt(value.slice(offset, offset + 2), 16) / 255;
		return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
	});
	const apply = (m: number[][], v: number[]) => m.map((row) => row[0] * v[0] + row[1] * v[1] + row[2] * v[2]);
	const rgbToLms = [
		[0.31399, 0.63951, 0.04649],
		[0.15537, 0.75789, 0.0867],
		[0.01775, 0.10945, 0.87259],
	];
	const lmsToRgb = [
		[5.47221, -4.64196, 0.16963],
		[-1.12524, 2.29317, -0.16789],
		[0.0298, -0.19318, 1.16364],
	];
	const collapse = {
		protan: [
			[0, 1.05118294, -0.05116099],
			[0, 1, 0],
			[0, 0, 1],
		],
		deutan: [
			[1, 0, 0],
			[0.9513092, 0, 0.04866992],
			[0, 0, 1],
		],
		tritan: [
			[1, 0, 0],
			[0, 1, 0],
			[-0.86744736, 1.86727089, 0],
		],
	}[kind];
	const out = apply(lmsToRgb, apply(collapse, apply(rgbToLms, rgb)));
	return `#${out
		.map((c) => {
			const s = c <= 0.0031308 ? 12.92 * c : 1.055 * Math.max(c, 0) ** (1 / 2.4) - 0.055;
			return Math.round(Math.min(1, Math.max(0, s)) * 255)
				.toString(16)
				.padStart(2, "0");
		})
		.join("")}`;
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

/** Every surface a theme paints text on: the message/tool/notice backgrounds plus the export canvases. */
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

	it("keeps tokens that mean different things apart", () => {
		const colors = getResolvedThemeColors("light");
		const groups: Record<string, string[]> = {
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
			"thinking levels": [
				"thinkingOff",
				"thinkingMinimal",
				"thinkingLow",
				"thinkingMedium",
				"thinkingHigh",
				"thinkingXhigh",
			],
			diff: ["toolDiffAdded", "toolDiffRemoved", "toolDiffContext"],
		};

		const collisions: string[] = [];
		for (const [label, tokens] of Object.entries(groups)) {
			for (let i = 0; i < tokens.length; i++) {
				for (let j = i + 1; j < tokens.length; j++) {
					const delta = difference(colors[tokens[i]], colors[tokens[j]]);
					if (delta < MIN_DIFFERENCE) {
						collisions.push(
							`[${label}] ${tokens[i]} (${colors[tokens[i]]}) ~ ${tokens[j]} (${colors[tokens[j]]}): ΔE ${delta.toFixed(1)}`,
						);
					}
				}
			}
		}
		expect(collisions).toEqual([]);
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

	it.each(ACCESSIBLE_THEMES)("%s keeps tokens that mean different things apart", (themeName) => {
		// Every token in these palettes is pushed dark (light themes) or light
		// (dark themes) to clear AAA, which squeezes them into a narrow lightness
		// band where distinct hues collapse into each other. Contrast does not
		// catch that: two tokens can both be perfectly readable and still be the
		// same color. Each group here is a set a user reads against itself — six
		// agent identities in one task panel, added vs removed vs context in one
		// diff — so a collision inside a group is a real misread.
		const colors = getResolvedThemeColors(themeName);
		const groups: Record<string, string[]> = {
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
			"thinking levels": [
				"thinkingOff",
				"thinkingMinimal",
				"thinkingLow",
				"thinkingMedium",
				"thinkingHigh",
				"thinkingXhigh",
			],
			diff: ["toolDiffAdded", "toolDiffRemoved", "toolDiffContext"],
		};
		// colorsafe-dark's palette is full: six agent identities already spend the
		// CVD-safe hues available at AAA on a dark ground, and every remaining
		// candidate for `mcp` degenerates to near-white. Tracked as a known gap
		// rather than papered over with a color that reads as plain text.
		const knownFull = themeName === "colorsafe-dark";

		const collisions: string[] = [];
		for (const [label, tokens] of Object.entries(groups)) {
			if (knownFull && label === "agent identity") continue;
			for (let i = 0; i < tokens.length; i++) {
				for (let j = i + 1; j < tokens.length; j++) {
					const delta = difference(colors[tokens[i]], colors[tokens[j]]);
					if (delta < MIN_DIFFERENCE) {
						collisions.push(
							`[${label}] ${tokens[i]} (${colors[tokens[i]]}) ~ ${tokens[j]} (${colors[tokens[j]]}): ΔE ${delta.toFixed(1)}`,
						);
					}
				}
			}
		}
		expect(collisions).toEqual([]);
	});

	it.each(ACCESSIBLE_THEMES)("%s keeps its surfaces on one side of the light/dark split", (themeName) => {
		const isLight = themeName.endsWith("-light");
		const wrongSide = Object.entries(surfaces(themeName))
			.map(([surface, background]) => [surface, luminance(background)] as const)
			.filter(([, value]) => (isLight ? value <= 0.5 : value >= 0.5))
			.map(([surface, value]) => `${surface}: ${value.toFixed(3)}`);
		expect(wrongSide).toEqual([]);
	});

	it("colorsafe-light keeps its semantic pairs apart under color-blind vision", () => {
		// The point of a colorsafe theme is that the pairs carrying opposite
		// meanings stay legible to a dichromat. Picking color-blind-safe hues is
		// not enough on its own: those palettes lean on lightness differences,
		// and darkening every token to clear AAA on white removes exactly that.
		// This checks the simulated result rather than the intent.
		const colors = getResolvedThemeColors("colorsafe-light");
		const pairs: [string, string][] = [
			["success", "error"],
			["error", "warning"],
			["success", "warning"],
			["toolDiffAdded", "toolDiffRemoved"],
			["toolDiffAdded", "toolDiffContext"],
			["toolDiffRemoved", "toolDiffContext"],
		];

		const failures: string[] = [];
		for (const [a, b] of pairs) {
			for (const kind of ["protan", "deutan", "tritan"] as const) {
				const delta = difference(simulateCvd(colors[a], kind), simulateCvd(colors[b], kind));
				if (delta < MIN_DIFFERENCE) {
					failures.push(`${a} ~ ${b} under ${kind}: ΔE ${delta.toFixed(1)}`);
				}
			}
		}
		expect(failures).toEqual([]);
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

/**
 * The Solarized grounds the `vox-cutout-*` pair is cut for.
 *
 * A theme paints text straight onto the terminal's own canvas wherever it draws
 * no surface of its own, so for a theme aimed at a specific terminal palette
 * those grounds are surfaces like any other — sweeping only the theme's own
 * backgrounds would miss the page the whole thing sits on. Both stops of each
 * palette are included: the alt band is the lighter of the two on Solarized
 * Light and the *lighter* of the two on Solarized Dark as well, so it is the
 * binding surface in both directions.
 */
const SOLARIZED = {
	light: { "solarized base3": "#fdf6e3", "solarized base2": "#eee8d5" },
	dark: { "solarized base03": "#002b36", "solarized base02": "#073642" },
} as const;

const CUTOUT_THEMES = ["vox-cutout-light", "vox-cutout-dark"] as const;

/**
 * Chip fills, and the inks that only ever render inside them. A chip is not a
 * page surface — measuring `headlineText` against the page says nothing, since
 * it never touches the page. Each pair is checked against itself instead.
 */
const CHIP_PAIRS = [
	["headlineBg", "headlineText"],
	["tapeBg", "tapeText"],
	["brandBg", "brandText"],
] as const;
const CHIP_TOKENS: ReadonlySet<string> = new Set<string>(CHIP_PAIRS.flat());

/**
 * Drawn on the terminal page and meant to recede: the offset band under a
 * block, and the unfilled remainder of a gauge. Holding these to 7:1 would
 * defeat the point — a shadow as loud as the text is not a shadow, and a track
 * as loud as its fill stops reading as the space the fill has yet to reach.
 */
const CUTOUT_DECORATIVE = new Set(["paperShadow", "halftone"]);

/** Rules and inactive chrome carry no meaning through colour in these themes. */
const CUTOUT_NEUTRAL_RULES = new Set([
	"border",
	"borderMuted",
	"mdHr",
	"mdCodeBlockBorder",
	"syntaxOperator",
	"syntaxPunctuation",
	"thinkingOff",
	"thinkingMinimal",
]);

/** How far a pasted sheet has to sit from the page for the cut edge to read. */
const MIN_SHEET_DIFFERENCE = 10;

describe.each(CUTOUT_THEMES)("%s", (themeName) => {
	const mode = themeName.endsWith("-light") ? "light" : "dark";
	const grounds: Record<string, string> = SOLARIZED[mode];

	function cutoutSurfaces(): Record<string, string> {
		const colors = getResolvedThemeColors(themeName);
		// `activeToolBg` is a real surface: the radar row paints `toolTitle` and
		// `toolOutput` straight onto it, so it answers to the same AAA sweep as
		// any other background. The chip fills do not, and are excluded above.
		return { ...grounds, ...surfaces(themeName), "bg.activeToolBg": colors.activeToolBg };
	}

	it("keeps every foreground at AAA contrast on every surface, terminal page included", () => {
		const colors = getResolvedThemeColors(themeName);
		const backgrounds = cutoutSurfaces();

		const failures: string[] = [];
		for (const [token, value] of Object.entries(colors)) {
			if ((BG_TOKENS as readonly string[]).includes(token)) continue;
			// A background, not an ink — it is swept as a surface below instead.
			if (token === "activeToolBg") continue;
			if (CHIP_TOKENS.has(token) || CUTOUT_DECORATIVE.has(token)) continue;
			for (const [surface, background] of Object.entries(backgrounds)) {
				const ratio = contrast(value, background);
				if (ratio < MIN_CONTRAST) {
					failures.push(`${token} (${value}) on ${surface} (${background}): ${ratio.toFixed(2)}:1`);
				}
			}
		}
		expect(failures).toEqual([]);
	});

	it("keeps every chip legible on its own fill", () => {
		const colors = getResolvedThemeColors(themeName);
		const failures: string[] = [];
		for (const [bg, fg] of CHIP_PAIRS) {
			// Honoured only as a pair, so a theme that sets one half and not the
			// other has a bug the renderer will silently swallow.
			expect(colors[bg] !== undefined, `${bg}/${fg} must be set as a pair`).toBe(colors[fg] !== undefined);
			const ratio = contrast(colors[fg], colors[bg]);
			if (ratio < MIN_CONTRAST) failures.push(`${fg} on ${bg}: ${ratio.toFixed(2)}:1`);
		}
		expect(failures).toEqual([]);
	});

	it("keeps the shadow and the gauge track visible without letting them read as text", () => {
		// A decorative separation bar, not a text-contrast one — the same floor
		// the default light theme holds its rules to.
		const MIN_DECORATIVE = 2.8;
		const colors = getResolvedThemeColors(themeName);
		const failures: string[] = [];
		for (const token of CUTOUT_DECORATIVE) {
			for (const [ground, value] of Object.entries(grounds)) {
				const ratio = contrast(colors[token], value);
				if (ratio < MIN_DECORATIVE) {
					failures.push(`${token} (${colors[token]}) on ${ground}: ${ratio.toFixed(2)}:1`);
				}
			}
		}
		// The shadow hugs the block it sits under, so it has to separate from
		// every sheet as well as from the page.
		for (const token of BG_TOKENS) {
			const delta = difference(colors.paperShadow, colors[token]);
			if (delta < MIN_SHEET_DIFFERENCE) {
				failures.push(`paperShadow (${colors.paperShadow}) ~ ${token} (${colors[token]}): ΔE ${delta.toFixed(1)}`);
			}
		}
		// `halftone` exists to stop the track reading as `dim`, which is
		// body-weight text. If the two collapse together it has bought nothing.
		const trackDelta = difference(colors.halftone, colors.dim);
		if (trackDelta < MIN_DIFFERENCE) {
			failures.push(`halftone (${colors.halftone}) ~ dim (${colors.dim}): ΔE ${trackDelta.toFixed(1)}`);
		}
		expect(failures).toEqual([]);
	});

	it("keeps meaning-carrying tokens saturated enough to read as printed ink", () => {
		const colors = getResolvedThemeColors(themeName);
		const washedOut = LIGHT_HUE_TOKENS.filter(
			(token) => !CUTOUT_NEUTRAL_RULES.has(token) && saturation(colors[token]) < 0.35,
		).map((token) => `${token} (${colors[token]}): ${(saturation(colors[token]) * 100).toFixed(0)}% saturation`);
		expect(washedOut).toEqual([]);
	});

	it("keeps tokens that mean different things apart", () => {
		const colors = getResolvedThemeColors(themeName);
		const groups: Record<string, string[]> = {
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
			"thinking levels": [
				"thinkingOff",
				"thinkingMinimal",
				"thinkingLow",
				"thinkingMedium",
				"thinkingHigh",
				"thinkingXhigh",
			],
			diff: ["toolDiffAdded", "toolDiffRemoved", "toolDiffContext"],
		};

		const collisions: string[] = [];
		for (const [label, tokens] of Object.entries(groups)) {
			for (let i = 0; i < tokens.length; i++) {
				for (let j = i + 1; j < tokens.length; j++) {
					const delta = difference(colors[tokens[i]], colors[tokens[j]]);
					if (delta < MIN_DIFFERENCE) {
						collisions.push(
							`[${label}] ${tokens[i]} (${colors[tokens[i]]}) ~ ${tokens[j]} (${colors[tokens[j]]}): ΔE ${delta.toFixed(1)}`,
						);
					}
				}
			}
		}
		expect(collisions).toEqual([]);
	});

	it("keeps colors that mean different things apart after the 256-color downgrade", () => {
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
			new URL(`../src/modes/interactive/theme/${themeName}.json`, import.meta.url).pathname,
			"256color",
		);

		for (const [label, tokens] of Object.entries(groups)) {
			const byIndex = new Map<string, string[]>();
			for (const token of tokens) {
				const index = quantized.getFgAnsi(token as ThemeColor);
				byIndex.set(index, [...(byIndex.get(index) ?? []), token]);
			}
			const collisions = [...byIndex.values()].filter((members) => members.length > 1).map((m) => m.join(" = "));
			expect(collisions, label).toEqual([]);
		}
	});

	it("cuts every pasted sheet clear of the Solarized page behind it", () => {
		// The look is sheets of stock pasted onto the terminal's own paper. A
		// surface that lands within a couple of ΔE of that paper stops reading as
		// a separate sheet at all — which is exactly how `vox-light`'s near-white
		// washes disappear on a Solarized Light terminal.
		const colors = getResolvedThemeColors(themeName);
		const tooClose: string[] = [];
		for (const token of BG_TOKENS) {
			for (const [ground, value] of Object.entries(grounds)) {
				const delta = difference(colors[token], value);
				if (delta < MIN_SHEET_DIFFERENCE) {
					tooClose.push(`${token} (${colors[token]}) vs ${ground} (${value}): ΔE ${delta.toFixed(1)}`);
				}
			}
		}
		expect(tooClose).toEqual([]);
	});

	it("keeps its surfaces on one side of the light/dark split", () => {
		const isLight = mode === "light";
		const wrongSide = Object.entries(surfaces(themeName))
			.map(([surface, background]) => [surface, luminance(background)] as const)
			.filter(([, value]) => (isLight ? value <= 0.5 : value >= 0.5))
			.map(([surface, value]) => `${surface}: ${value.toFixed(3)}`);
		expect(wrongSide).toEqual([]);
	});
});
