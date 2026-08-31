import { type EditorTopBorderLabel, truncateToWidth, visibleWidth } from "@kolisachint/hoocode-tui";
import { sessionColorToken, theme } from "../theme/theme.js";

/**
 * The session chip: the session's name, filled with its colour, laid into the
 * top-right of the input box.
 *
 * The footer used to be the only place a session announced itself, at the tail of
 * its busiest line in its quietest colour. The chip puts the same fact where the
 * cursor already is, and gives it a colour so it can be recognised rather than
 * read — which is what you want when four terminals are open.
 *
 * This module owns the *rule* as well as the rendering, because two components
 * depend on it: the editor draws the chip, and the footer drops its own copy of
 * the name when the chip is showing. Both call `sessionChipFits`, so the name
 * cannot end up in neither place.
 */

/**
 * Longest name the chip will show. Past this it truncates: a chip is a glance
 * target, and a name that eats half the border has stopped being one.
 */
const MAX_CHIP_NAME_WIDTH = 20;

/**
 * Terminal width at which the chip is worth drawing at all. Below it the box has
 * more urgent things to spend columns on, and the footer keeps the name instead.
 *
 * The number is not arbitrary: with the name capped at 20 columns the chip is at
 * most 22 cells, and 48 columns leaves the editor's border enough room for the
 * chip, its inset, and a lead-in even with the scroll indicator also present — so
 * "fits" here and "fits" inside the editor agree.
 */
export const SESSION_CHIP_MIN_WIDTH = 48;

/** Whether a chip is drawn at this terminal width. */
export function sessionChipFits(width: number): boolean {
	return width >= SESSION_CHIP_MIN_WIDTH;
}

/**
 * Build the chip for a session. Returns the plain/styled pair the editor's top
 * border needs; the padding is part of the plain string too, so width math and
 * truncation stay honest.
 *
 * The fill carries the colour and the ink is picked from the fill's luminance, so
 * one call renders correctly on a dark theme's bright hues and a light theme's
 * deep ones alike. Themes whose palette entry cannot be filled fall back to an
 * outline chip — quieter, but never unreadable.
 */
export function renderSessionChip(name: string, colorSlot: number): EditorTopBorderLabel | undefined {
	const trimmed = name.trim();
	if (!trimmed) return undefined;
	const shown =
		visibleWidth(trimmed) > MAX_CHIP_NAME_WIDTH ? truncateToWidth(trimmed, MAX_CHIP_NAME_WIDTH, "…") : trimmed;
	const token = sessionColorToken(colorSlot);
	if (theme.canFill(token)) {
		const plain = ` ${shown} `;
		// Bold is written as a raw SGR pair rather than through chalk so it
		// survives inside the fill's own colour run, matching how the tape chip
		// emits its label.
		return { plain, styled: theme.fill(token, `\x1b[1m${plain}\x1b[22m`) };
	}
	const plain = `┤${shown}├`;
	return { plain, styled: theme.fg("border", "┤") + theme.fg(token, shown) + theme.fg("border", "├") };
}
