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
 *
 * This lives in `core/` rather than `modes/interactive/` because the `/plugin`
 * command extension labels the same capability classes, and extensions must not
 * reach into a mode's internals. The module is pure data — no theme, no
 * rendering — so it is safe for every surface.
 */

/** The HooCode mark — a filled hexagon, rendered in the accent colour. */
export const BRAND_MARK = "⬢";

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
	marketplaces: "⊞",
	themes: "◒",
	context: "❯",
	extensions: "⊹",
} as const;

export type CategoryKey = keyof typeof CATEGORY_GLYPH;

/** A soft dot separator used between footer/summary segments. */
export const SEGMENT_SEP = "·";

/** Fork glyph preceding a git branch. */
export const GIT_BRANCH_GLYPH = "⑂";
