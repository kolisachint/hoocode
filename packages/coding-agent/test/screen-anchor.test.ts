/**
 * The app fills the terminal, and the prompt is on the floor of it.
 *
 * `tui/test/screen-fill.test.ts` holds the mechanism; this holds the thing the
 * mechanism exists for, against the real mode with the real chrome in it. The
 * bug it is written from: on a fresh session the banner sat a few rows down
 * with the prompt directly under it and the user's shell history above, and the
 * prompt then walked down the screen over the next few turns. Pinning the
 * bottom is what makes those two the same screen.
 *
 * The fill sits directly below the banner, so a fresh session opens with the
 * logo on the first row, the leftover rows between it and the conversation,
 * and the prompt hard against the floor. A band of blank between the last
 * thing the agent said and the box you answer it in is the thing this file
 * exists to catch.
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
	it("opens with the banner on the first row, the prompt on the floor, and the leftover rows between", async () => {
		const harness = await createSurfaceHarness();
		try {
			await settle();
			const rows = frameRows(harness);
			expect(rows).toHaveLength(ROWS);
			// The banner holds the first row and the prompt and footer the last
			// ones; whatever nobody is using sits between the banner and the
			// conversation below it.
			const banner = rows.findIndex((row) => row.includes("hoo"));
			expect(banner).toBe(0);
			const prompt = rows.findIndex((row) => row.includes("❯"));
			expect(prompt).toBeGreaterThan(banner + 3);
			expect(rows.slice(prompt).every((row) => row !== "")).toBe(true);
			expect(rows.at(-1)).not.toBe("");
			expect(rows.slice(banner + 3, prompt).some((row) => row === "")).toBe(true);
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

	it("puts a command's receipt on the band too, and paints it", async () => {
		// Every command used to leave a dimmed row in the conversation — `Chrome:
		// compact`, `Mode set to "build"`, `Copied last agent message` — true for
		// a moment and litter for the rest of the session. They report on the
		// band now, where the eye already is after typing a command, and the fill
		// is what makes one readable against a transcript full of text.
		const harness = await createSurfaceHarness();
		try {
			await settle();
			await harness.submit("/chrome");
			await settle();
			const rows = frameRows(harness);
			const prompt = rows.findIndex((row) => row.includes("❯"));
			expect(rows[prompt - 2]).toContain("Chrome:");
			expect(rows.slice(0, prompt - 2).join("\n")).not.toContain("Chrome:");
			// The band's row is filled, and the fill runs the whole width.
			const painted = harness.rawFrame().split("\n")[prompt - 2];
			expect(painted).toContain("\x1b[48;");
			expect(painted.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "")).toHaveLength(100);
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
