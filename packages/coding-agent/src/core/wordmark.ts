/**
 * Compact wordmark banner shown at startup.
 */

/** Compact three-line owl glyph rendered beside the brand text in the banner. */
const WORDMARK_GLYPH = ["▟▀▀▀▀▀▙", "▌▟▙ ▟▙▐", "▜▄▄▄▄▄▛"];

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
 *     ▌▟▙ ▟▙▐  coding agent · v0.1.0
 *     ▜▄▄▄▄▄▛  ~/project
 */
export function buildCompactWordmark(options: CompactWordmarkOptions): string {
	const { appName, version, cwd, accent, dim, muted } = options;
	const tagline = options.tagline ?? "coding agent";

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
