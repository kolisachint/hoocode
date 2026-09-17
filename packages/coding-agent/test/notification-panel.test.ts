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
import { initTheme, theme } from "../src/modes/interactive/theme/theme.js";

const WIDTH = 60;

function setup(maxBodyRows?: () => number) {
	let renders = 0;
	const panel = new NotificationPanel(() => {
		renders += 1;
	}, maxBodyRows);
	return { panel, renderCount: () => renders };
}

/** Visible width of a row, styling excluded. */
function width(line: string): number {
	return stripVTControlCharacters(line).length;
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

	it("paints every row it draws, edge to edge", () => {
		// The band carries what a command had to say now, against a transcript
		// already full of text. The fill is what makes it findable at a glance —
		// and a fill that stops short of the margin is a band with a bite out of
		// it, so the painted rows are exactly as wide as the screen.
		const { panel } = setup();
		panel.notify("info", "Mode: build", ["described in .hoo/modes/build.md"]);
		const [separator, headline, body] = panel.render(WIDTH);
		expect(separator).toBe("");
		expect(headline).toContain(theme.getBgAnsi("customMessageBg"));
		expect(body).toContain(theme.getBgAnsi("customMessageBg"));
		expect(width(headline)).toBe(WIDTH);
		expect(width(body)).toBe(WIDTH);
	});

	it("paints a warning in the warning fill", () => {
		const { panel } = setup();
		panel.notify("warning", "No previous directory to return to");
		expect(panel.render(WIDTH)[1]).toContain(theme.getBgAnsi("warningBg"));
	});

	it("takes the rows the screen can spare for a listing", () => {
		// Commands report on the band now, listings included, so the bound is
		// the owner's to set from the terminal's height rather than a constant
		// that makes every listing three rows long on every screen.
		const { panel } = setup(() => 6);
		panel.notify("info", "Marketplaces", ["one", "two", "three", "four", "five", "six", "seven"]);
		expect(rows(panel)).toHaveLength(8); // blank + headline + 6 body rows
	});

	it("never takes less than one row of body, whatever it is told", () => {
		const { panel } = setup(() => 0);
		panel.notify("info", "Marketplaces", ["one", "two"]);
		expect(rows(panel)).toHaveLength(3);
	});

	it("gives a listing the time to be read", () => {
		// A five-row listing timed like a one-line glimpse is a listing nobody
		// finished. The body earns reading time; the headline alone does not.
		const { panel } = setup(() => 6);
		panel.notify("info", "Marketplaces", ["one", "two", "three"]);
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.info);
		expect(panel.showing?.title).toBe("Marketplaces");
		vi.advanceTimersByTime(NOTIFICATION_TTL_MS.info * 3);
		expect(panel.showing).toBeUndefined();
	});

	it("lets a listing keep the colours it painted itself", () => {
		// A listing that colours its own columns arrives styled. Wrapping it
		// would drop that colour at the first inner span, because `theme.fg`
		// closes with a reset rather than a restore — the same rule the
		// transcript's own status rows follow.
		const { panel } = setup(() => 4);
		const coloured = `${theme.getFgAnsi("accent")}plugin\u001b[39m  installed`;
		panel.notify("info", "Marketplaces", [coloured]);
		const body = panel.render(WIDTH)[2];
		expect(body).toContain(theme.getFgAnsi("accent"));
		expect(body).not.toContain(theme.getFgAnsi("muted"));
	});

	it("redraws when the screen gets shorter, not just narrower", () => {
		// The row budget is the terminal's height, which changes without the
		// width changing; a cache watching only the width would go on drawing a
		// band the screen no longer has room for.
		let budget = 6;
		const { panel } = setup(() => budget);
		panel.notify("info", "Marketplaces", ["one", "two", "three", "four", "five", "six"]);
		expect(rows(panel)).toHaveLength(8);
		budget = 2;
		expect(rows(panel)).toHaveLength(4);
	});

	it("lets a caller set the time itself", () => {
		const { panel } = setup();
		panel.notify("info", "quick", ["a", "b"], undefined, 500);
		vi.advanceTimersByTime(500);
		expect(panel.showing).toBeUndefined();
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
