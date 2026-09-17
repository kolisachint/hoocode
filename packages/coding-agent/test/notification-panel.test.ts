/**
 * The transient band above the prompt.
 *
 * Two properties, and they are deliberately different from each other:
 * a glimpse reports state, so only the newest one is true and it replaces;
 * a warning reports an event, so every one of them is still true when the next
 * arrives and they queue. Getting that backwards is how the last of three
 * startup warnings silently erases the two before it, or how holding a dial key
 * down leaves you watching a three-second slideshow of values that are already
 * wrong.
 */

import { stripVTControlCharacters } from "node:util";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NOTIFICATION_TTL_MS, NotificationPanel } from "../src/modes/interactive/components/notification-panel.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

const WIDTH = 60;

function setup() {
	let renders = 0;
	const panel = new NotificationPanel(() => {
		renders += 1;
	});
	return { panel, renderCount: () => renders };
}

/** The band's rows with the styling taken off, so a test reads as text. */
function rows(panel: NotificationPanel, width = WIDTH): string[] {
	return panel.render(width).map((line) => stripVTControlCharacters(line).trimEnd());
}

describe("NotificationPanel", () => {
	beforeEach(() => {
		initTheme("dark");
		vi.useFakeTimers();
	});
	afterEach(() => {
		vi.useRealTimers();
	});

	it("draws nothing at all when nothing is up", () => {
		const { panel } = setup();
		expect(panel.render(WIDTH)).toEqual([]);
		// Same array every time: the render caches compare by identity, and a
		// fresh [] for an empty band would read as a change on every frame.
		expect(panel.render(WIDTH)).toBe(panel.render(WIDTH));
	});

	it("leads with a blank row and never pads its own bottom edge", () => {
		// The blank is this block's separator from the ledger above it; the row
		// below it is the prompt's own border, which needs no blank beside it.
		const { panel } = setup();
		panel.notify("info", "Model: opus-5");
		const lines = rows(panel);
		expect(lines[0]).toBe("");
		expect(lines).toHaveLength(2);
		expect(lines[1]).toContain("Model: opus-5");
	});

	it("puts the note flush right, and drops it before the headline", () => {
		const { panel } = setup();
		panel.notify("info", "Thinking level: high", [], "shift+alt+t steps back");
		expect(rows(panel)[1]).toMatch(/Thinking level: high\s{3,}shift\+alt\+t steps back$/);

		// Too narrow for both: the hint goes, the thing that happened stays.
		const narrow = rows(panel, 26)[1];
		expect(narrow).toContain("Thinking level");
		expect(narrow).not.toContain("steps back");
	});

	it("fades on its own clock", () => {
		const { panel } = setup();
		panel.notify("info", "Chrome: compact");
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.info - 1);
		expect(panel.showing?.title).toBe("Chrome: compact");
		vi.advanceTimersByTime(1);
		expect(panel.showing).toBeUndefined();
		expect(panel.render(WIDTH)).toEqual([]);
	});

	it("gives a warning longer than a glimpse", () => {
		const { panel } = setup();
		panel.notify("warning", "No previous directory to return to");
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.info);
		expect(panel.showing?.title).toBe("No previous directory to return to");
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.warning - NOTIFICATION_TTL_MS.info);
		expect(panel.showing).toBeUndefined();
	});

	it("collapses a run of glimpses to the one that is still true", () => {
		const { panel } = setup();
		panel.notify("warning", "held");
		panel.notify("info", "Model: a");
		panel.notify("info", "Model: b");
		panel.notify("info", "Model: c");
		// Only the last glimpse survives, and it is still waiting behind the
		// warning rather than having pushed it off the screen.
		expect(panel.showing?.title).toBe("held");
		expect(panel.pending.map((n) => n.title)).toEqual(["Model: c"]);
	});

	it("never cuts short the notification already on screen", () => {
		// The head is what the user is reading. Replacing it in place is how a
		// message becomes unreadable through no fault of the reader.
		const { panel } = setup();
		panel.notify("info", "Model: a");
		panel.notify("info", "Model: b");
		expect(panel.showing?.title).toBe("Model: a");
		expect(panel.pending.map((n) => n.title)).toEqual(["Model: b"]);
	});

	it("shows queued warnings one after another", () => {
		const { panel } = setup();
		panel.notify("warning", "first");
		panel.notify("warning", "second");
		panel.notify("warning", "third");
		expect(panel.showing?.title).toBe("first");
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.warning);
		expect(panel.showing?.title).toBe("second");
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.warning);
		expect(panel.showing?.title).toBe("third");
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.warning);
		expect(panel.showing).toBeUndefined();
	});

	it("stops growing rather than queueing forever", () => {
		const { panel } = setup();
		for (let i = 0; i < 40; i++) panel.notify("warning", `w${i}`);
		expect(1 + panel.pending.length).toBeLessThanOrEqual(8);
		// The one being read is kept, and so is the newest arrival.
		expect(panel.showing?.title).toBe("w0");
		expect(panel.pending.at(-1)?.title).toBe("w39");
	});

	it("bounds the body so the band cannot eat the conversation", () => {
		const { panel } = setup();
		panel.notify("warning", "headline", ["a", "b", "c", "d", "e"]);
		expect(rows(panel)).toHaveLength(5); // blank + headline + 3 body rows
	});

	it("asks for a frame when it appears and when it goes", () => {
		const { panel, renderCount } = setup();
		panel.notify("info", "Model: a");
		expect(renderCount()).toBe(1);
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.info);
		expect(renderCount()).toBe(2);
	});

	it("takes the whole queue down on dismiss", () => {
		const { panel } = setup();
		panel.notify("warning", "first");
		panel.notify("warning", "second");
		panel.dismiss();
		expect(panel.showing).toBeUndefined();
		expect(panel.pending).toEqual([]);
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.warning * 2);
		expect(panel.showing).toBeUndefined();
	});
});
