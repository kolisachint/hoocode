/**
 * HooCode brand identity — the single source of truth for the product mark and
 * the glyph vocabulary the interactive surfaces share.
 *
 * The brand *colour* is the theme's `accent` token (see theme/dark.json,
 * theme/light.json) so it stays theme-aware and swappable; this module owns the
 * non-colour identity: the mark and the category glyphs used by the footer,
 * the startup resource summary, and anywhere a capability class is labelled.
 * Keeping them here means a future rebrand touches one file, not twenty.
 *
 * Glyphs are chosen to be single terminal cell, widely supported, and distinct
 * from the task-panel's own status/owner glyphs (◐ ◆ ◇ ▸ ⧉ ✓ ✗ ○) so the two
 * vocabularies never read as the same signal.
 */

/** The HooCode mark — a filled hexagon, rendered in the accent colour. */
export const BRAND_MARK = "⬢";

/** Product name, for splashes and headers. */
export const BRAND_NAME = "HooCode";

/**
 * Glyphs for the capability classes a session can load. Used by the startup
 * "resources ready" summary and reusable anywhere a class needs a label. Each is
 * a single cell so counts and columns stay aligned.
 */
export const CATEGORY_GLYPH = {
	skills: "✦",
	commands: "⌘",
	agents: "◈",
	mcp: "⧉",
	plugins: "⬡",
	themes: "◒",
	context: "❯",
	extensions: "⊹",
} as const;

export type CategoryKey = keyof typeof CATEGORY_GLYPH;

/** A soft dot separator used between footer/summary segments. */
export const SEGMENT_SEP = "·";

/** Marker appended to a git branch when the working tree is dirty. */
export const GIT_DIRTY_MARK = "*";

/** Fork glyph preceding a git branch. */
export const GIT_BRANCH_GLYPH = "⑂";
