/**
 * The chrome dial's stops.
 *
 * Lives in `core` next to `tool-output-view.ts` for the same reason that one
 * does: the settings manager has to read and validate the value, and the
 * settings manager must not reach up into `modes/interactive`. The policy that
 * turns a stop into a layout is up there, in `interactive/chrome.ts`, where it
 * belongs — this file is only the vocabulary they share.
 */

/** In the order the dial steps through them, most chrome to least. */
export const CHROME_DENSITIES = ["full", "compact", "bare"] as const;

export type ChromeDensity = (typeof CHROME_DENSITIES)[number];

export function isChromeDensity(value: unknown): value is ChromeDensity {
	return typeof value === "string" && (CHROME_DENSITIES as readonly string[]).includes(value);
}
