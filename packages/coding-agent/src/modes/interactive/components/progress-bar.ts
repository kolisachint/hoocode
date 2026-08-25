/**
 * The one progress bar. Every surface that makes the user wait on measurable
 * work renders it through here.
 *
 * There used to be two, written months apart, and they had drifted in three
 * separate ways: the footer drew `▰▱` while the voice panel drew `·` over `·`,
 * the footer said `2.0 MB` while the voice panel said `2 MB`, and each carried
 * its own copy of the percent-and-detail layout. None of that is a decision
 * anyone made — it is just what happens when the same widget is written twice.
 *
 * Bar *width* is still the caller's, because that is a genuine property of the
 * surface: a footer line shares its row with other status, a panel has the room
 * to be finer-grained. Everything else — glyphs, colours, percent, separator,
 * byte formatting — is fixed here, so two bars on screen at once always agree.
 */

import { theme } from "../theme/theme.js";

/**
 * Default cells in a bar. Compact enough that several stacked footer lines
 * still fit a narrow terminal.
 */
export const PROGRESS_BAR_CELLS = 12;

/**
 * Megabytes, to one decimal.
 *
 * The decimal is the point: whole megabytes make a slow download look frozen
 * for tens of seconds at a time, which is precisely when the user is deciding
 * whether it has hung.
 */
export function formatMegabytes(bytes: number): string {
	return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * The filled/empty track.
 *
 * The glyphs differ in shape, not only in colour, matching the context gauge in
 * the footer. An earlier version drew both halves as `·` and left the whole
 * reading to the colour, so at a glance — or on a low-contrast theme, or piped
 * somewhere that drops styling — a bar at 10% and one at 90% were identical.
 *
 * A started-but-tiny ratio keeps one filled cell rather than rounding to an
 * empty bar: "nothing has happened yet" and "the first of forty is done" are
 * different things to be told.
 */
function track(ratio: number, cells: number): string {
	const clamped = Math.max(0, Math.min(1, ratio));
	const filled = Math.max(clamped > 0 ? 1 : 0, Math.min(cells, Math.round(clamped * cells)));
	return theme.fg("accent", "▰".repeat(filled)) + theme.fg("halftone", "▱".repeat(cells - filled));
}

export interface ProgressBarOptions {
	/** Track width. Defaults to {@link PROGRESS_BAR_CELLS}. */
	cells?: number;
}

/**
 * A determinate bar: track, percent, and a trailing detail such as
 * `3/12 sessions` or `2.0 MB / 8.0 MB`.
 *
 * Returns a styled string with no leading or trailing space, so callers can
 * place it after their own label. Callers width-clamp.
 */
export function renderProgressBar(ratio: number, detail: string, options: ProgressBarOptions = {}): string {
	const cells = options.cells ?? PROGRESS_BAR_CELLS;
	const clamped = Math.max(0, Math.min(1, ratio));
	const percent = `${Math.round(clamped * 100)}%`;
	return `${track(clamped, cells)} ${theme.fg("muted", percent)} ${theme.fg("dim", `· ${detail}`)}`;
}

/**
 * A download's progress, determinate when the server sent a length and a
 * running byte count when it did not.
 *
 * The indeterminate case deliberately drops the bar rather than animating a
 * fake one: a moving byte count already says "still going", and a bar that
 * cannot reach the end is worse than no bar.
 */
export function renderDownloadProgress(
	receivedBytes: number,
	totalBytes: number | null,
	options: ProgressBarOptions = {},
): string {
	if (totalBytes === null || totalBytes <= 0) {
		return theme.fg("dim", `${formatMegabytes(receivedBytes)}…`);
	}
	const detail = `${formatMegabytes(receivedBytes)} / ${formatMegabytes(totalBytes)}`;
	return renderProgressBar(receivedBytes / totalBytes, detail, options);
}
