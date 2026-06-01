/**
 * ASCII wordmark for the startup banner.
 * Rendered in the terminal with accent color on the "hoo" portion.
 * Source: design-system/assets/wordmark.txt
 */

import { WORDMARK_SYMBOL_BLOCKS } from "./wordmark-symbol.generated.js";

export const WORDMARK = [
	"                   __  __            ______          __",
	"                  / / / /___  ____  / ____/___  ____/ /__",
	"                 / /_/ / __ \\/ __ \\/ /   / __ \\/ __  / _ \\",
	"                / __  / /_/ / /_/ / /___/ /_/ / /_/ /  __/",
	"               /_/ /_/\\____/\\____/\\____/\\____/\\__,_/\\___/",
	"",
	"               deterministic terminal coding agent   >  hoo",
].join("\n");

/** Compact one-line logo for tight spaces. */
export const WORDMARK_COMPACT = "hoo — deterministic terminal coding agent";

/**
 * Colored half-block owl symbol, indented to sit centered above the ASCII
 * wordmark. Carries baked-in brand colors (Signal Cyan + white), so it only
 * renders on truecolor terminals; callers fall back to {@link WORDMARK}
 * otherwise.
 */
const SYMBOL_INDENT = " ".repeat(15);
export const WORDMARK_SYMBOL = WORDMARK_SYMBOL_BLOCKS.split("\n")
	.map((line) => SYMBOL_INDENT + line)
	.join("\n");

/** Compact three-line owl glyph rendered beside the brand text in the banner. */
export const WORDMARK_GLYPH = ["▟▀▀▀▀▀▙", "▌▟▙ ▟▙▐", "▜▄▄▄▄▄▛"];

const GLYPH_INDENT = " ".repeat(3);
const GLYPH_GAP = " ".repeat(2);

export interface CompactWordmarkOptions {
	appName: string;
	version: string;
	cwd: string;
	/** Tagline shown next to the version. */
	tagline?: string;
	/** Colorize the brand "hoo" portion / glyph. */
	accent: (text: string) => string;
	/** Colorize secondary text (tagline, version, cwd). */
	dim: (text: string) => string;
	/** Colorize separators and the glyph outline. */
	muted: (text: string) => string;
	/** Render the trailing blinking cursor (e.g. blink + accent). Optional. */
	cursor?: (text: string) => string;
	/** Optional note appended to the cwd line (e.g. a keybinding hint). */
	note?: () => string;
}

/**
 * Build the compact startup banner: a small owl glyph beside the brand name,
 * tagline + version, and the working directory.
 *
 *     ▟▀▀▀▀▀▙  hoocode
 *     ▌▟▙ ▟▙▐  agentic coding agent · v0.1.0
 *     ▜▄▄▄▄▄▛  ~/project
 */
export function buildCompactWordmark(options: CompactWordmarkOptions): string {
	const { appName, version, cwd, accent, dim, muted } = options;
	const tagline = options.tagline ?? "agentic coding agent";

	// Highlight the "hoo" prefix when present, otherwise accent the whole name.
	const name = appName.startsWith("hoo") ? accent("hoo") + muted("│") + appName.slice(3) : accent(appName);
	const brand = options.cursor ? name + options.cursor("_") : name;

	const right = [
		brand,
		`${dim(tagline)} ${muted("·")} ${dim(`v${version}`)}`,
		options.note ? `${dim(cwd)}${options.note()}` : dim(cwd),
	];

	return WORDMARK_GLYPH.map((glyphLine, index) => {
		const text = right[index] ?? "";
		return `${GLYPH_INDENT}${accent(glyphLine)}${GLYPH_GAP}${text}`;
	}).join("\n");
}
