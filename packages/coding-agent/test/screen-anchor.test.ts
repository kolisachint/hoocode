/**
 * The app fills the terminal, and the prompt is on the floor of it.
 *
 * `tui/test/screen-fill.test.ts` holds the mechanism; this holds the thing the
 * mechanism exists for, against the real mode with the real chrome in it. The
 * bug it is written from: on a fresh session the banner sat a few rows down
 * with the prompt directly under it and the user's shell history above, and the
 * prompt then walked down the screen over the next few turns. Pinning the
 * bottom is what makes those two the same screen.
 */

import { describe, expect, it } from "vitest";
import { createSurfaceHarness, type SurfaceHarness } from "./suite/interactive-surface-harness.js";

/** The capturing terminal's size — the harness renders at exactly this. */
const ROWS = 40;

/** Frame rows, ANSI off, trailing space gone, and *not* trimmed at the ends. */
function frameRows(harness: SurfaceHarness): string[] {
	return harness
		.rawFrame()
		.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "")
		.split("\n")
		.map((line) => line.replace(/\s+$/, ""));
}

/** Let the render loop run, so the frame has been fitted to the screen. */
async function settle(): Promise<void> {
	await new Promise((resolve) => setTimeout(resolve, 120));
}

describe("the app fills the screen", () => {
	it("opens with the banner on the first row and the footer on the last", async () => {
		const harness = await createSurfaceHarness();
		try {
			await settle();
			const rows = frameRows(harness);
			expect(rows).toHaveLength(ROWS);
			expect(rows[0]).toContain("hoo");
			// The prompt and the footer are hard against the bottom edge: the rows
			// under the prompt are the footer's, and the blank ones are all above.
			const prompt = rows.findIndex((row) => row.includes("❯"));
			expect(prompt).toBeGreaterThan(0);
			expect(rows.slice(prompt).every((row) => row !== "")).toBe(true);
			expect(rows.at(-1)).not.toBe("");
			// And the blank band is the fill, in one run, above the chrome.
			expect(rows.slice(6, prompt - 2).every((row) => row === "")).toBe(true);
		} finally {
			harness.cleanup();
		}
	}, 60000);

	it("puts a glimpse on the band directly above the prompt, not in the transcript", async () => {
		const harness = await createSurfaceHarness();
		try {
			await settle();
			await harness.submit("/name probe-name");
			await settle();
			const rows = frameRows(harness);
			const prompt = rows.findIndex((row) => row.includes("❯"));
			// The band is the row above the prompt's top border, and the only place
			// the message appears: a session name is on the border and in the
			// footer from here on, so a permanent transcript row for it says
			// nothing the screen does not already say.
			expect(rows[prompt - 2]).toContain("Session name set:");
			expect(rows.slice(0, prompt - 2).join("\n")).not.toContain("Session name set:");
			expect(rows).toHaveLength(ROWS);
		} finally {
			harness.cleanup();
		}
	}, 60000);

	it("keeps the prompt on the floor once there is a conversation above it", async () => {
		const harness = await createSurfaceHarness();
		try {
			await settle();
			await harness.submit("hello");
			await settle();
			const rows = frameRows(harness);
			// Never shorter than the screen: the fill gives back exactly what the
			// conversation took, so the prompt does not walk down the terminal as
			// the session grows.
			expect(rows.length).toBeGreaterThanOrEqual(ROWS);
			const prompt = rows.findIndex((row) => row.includes("❯"));
			expect(prompt).toBeGreaterThan(0);
			expect(rows.slice(prompt).every((row) => row !== "")).toBe(true);
		} finally {
			harness.cleanup();
		}
	}, 60000);
});
