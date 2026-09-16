/**
 * The pinned viewport.
 *
 * The bug this exists for: scrolling back through a session did not stay put.
 * Output arriving, a pane closing, a line above the fold re-rendering — any of
 * them took the reader back to the bottom, because "scrolled up" was a fact
 * about the terminal and every repaint went through `\x1b[3J`, which throws the
 * scrollback away.
 *
 * So the property under test throughout is *stillness*: given a pinned view,
 * the rows on screen are the same rows before and after whatever the session
 * just did. Everything else here — the keys, the ends, the release at the
 * bottom — is in service of that one guarantee being usable.
 */

import assert from "node:assert";
import { beforeEach, describe, it } from "node:test";
import { Text } from "../src/components/text.js";
import { type Component, TUI } from "../src/tui.js";
import { VirtualTerminal } from "./virtual-terminal.js";

/** A transcript of numbered rows, so a window's position is readable from it. */
class Transcript implements Component {
	lines: string[];
	constructor(count: number, label = "line") {
		this.lines = Array.from({ length: count }, (_, i) => `${label} ${i + 1}`);
	}
	append(text: string): void {
		this.lines = [...this.lines, text];
	}
	invalidate(): void {}
	render(_width: number): string[] {
		return this.lines;
	}
}

const WIDTH = 40;
const HEIGHT = 10;
/** The last row is the indicator, so a window shows one fewer than the height. */
const VIEW = HEIGHT - 1;

/**
 * A started TUI with one frame already painted.
 *
 * The first paint matters: before it there is no line buffer, so there is
 * genuinely nothing to scroll and every scroll call correctly declines.
 */
async function setup(lineCount = 100): Promise<{ tui: TUI; terminal: VirtualTerminal; transcript: Transcript }> {
	const terminal = new VirtualTerminal(WIDTH, HEIGHT);
	const tui = new TUI(terminal);
	const transcript = new Transcript(lineCount);
	tui.addChild(transcript);
	tui.start();
	await settle(terminal);
	return { tui, terminal, transcript };
}

/** Let the render loop run and the terminal catch up. */
async function settle(terminal: VirtualTerminal): Promise<void> {
	await new Promise((resolve) => setTimeout(resolve, 40));
	await terminal.flush();
}

/** The visible rows, minus the indicator, trimmed for comparison. */
async function window(terminal: VirtualTerminal): Promise<string[]> {
	const viewport = await terminal.flushAndGetViewport();
	return viewport.slice(0, VIEW).map((row) => row.trimEnd());
}

async function statusRow(terminal: VirtualTerminal): Promise<string> {
	const viewport = await terminal.flushAndGetViewport();
	return viewport[HEIGHT - 1].trim();
}

/** One wheel notch up, as the terminal would deliver it. */
function wheelUp(terminal: VirtualTerminal, times = 1): void {
	for (let i = 0; i < times; i++) terminal.sendInput("\x1b[<64;1;1M");
}

function wheelDown(terminal: VirtualTerminal, times = 1): void {
	for (let i = 0; i < times; i++) terminal.sendInput("\x1b[<65;1;1M");
}

describe("pinning the view", () => {
	let harness: Awaited<ReturnType<typeof setup>>;
	beforeEach(async () => {
		harness = await setup();
	});

	it("does not pin while the view is already live", async () => {
		// Wheel-down at the bottom is a request for output that does not exist.
		// Treating it as a move would drop out of scroll mode — and force a full
		// repaint — on every notch.
		wheelDown(harness.terminal, 5);
		assert.equal(harness.tui.scrollPinned, false);
	});

	it("pins on the first wheel notch up", async () => {
		wheelUp(harness.terminal);
		assert.equal(harness.tui.scrollPinned, true);
	});

	it("moves three lines per notch, as every other window does", async () => {
		wheelUp(harness.terminal);
		const first = harness.tui.getScrollPosition();
		wheelUp(harness.terminal);
		assert.equal(first!.top - harness.tui.getScrollPosition()!.top, 3);
	});

	it("refuses to pin when the app says this is not the moment", async () => {
		// A picker on screen owns the arrow keys; a view that pinned underneath it
		// would take them away from the surface actually asking a question.
		harness.tui.canPinScroll = () => false;
		wheelUp(harness.terminal);
		assert.equal(harness.tui.scrollPinned, false);
	});

	it("keeps answering once pinned, even if pinning would now be refused", async () => {
		wheelUp(harness.terminal);
		harness.tui.canPinScroll = () => false;
		const before = harness.tui.getScrollPosition()!.top;
		harness.tui.scrollByLines(-1);
		assert.equal(harness.tui.getScrollPosition()!.top, before - 1);
		// And the way out still works, so a reader is never stranded.
		assert.equal(harness.tui.scrollToLive(), true);
	});
});

describe("staying put", () => {
	it("shows the rows the offset names", async () => {
		const { tui, terminal } = await setup();
		tui.scrollByLines(-20);
		const rows = await window(terminal);
		const top = tui.getScrollPosition()!.top;
		assert.equal(rows[0], `line ${top + 1}`);
		assert.equal(rows[VIEW - 1], `line ${top + VIEW}`);
	});

	it("does not move when output arrives underneath it", async () => {
		// The whole point. Before this, an append could change a row above the
		// fold, take the full-redraw path, and clear the scrollback out from under
		// a reader who had chosen where to look.
		const { tui, terminal, transcript } = await setup();
		tui.scrollByLines(-30);
		const before = await window(terminal);

		for (let i = 0; i < 40; i++) {
			transcript.append(`streamed ${i}`);
			tui.requestRender();
		}
		await settle(terminal);

		assert.deepEqual(await window(terminal), before);
	});

	it("reports the growing transcript while holding its place", async () => {
		const { tui, terminal, transcript } = await setup();
		tui.scrollByLines(-30);
		const top = tui.getScrollPosition()!.top;

		transcript.append("one more");
		tui.requestRender();
		await settle(terminal);

		assert.equal(tui.getScrollPosition()!.top, top, "the window stayed");
		assert.equal(tui.getScrollPosition()!.total, 101, "and knows there is more below it");
	});

	it("re-clamps rather than running off the end when the transcript shrinks", async () => {
		const { tui, terminal, transcript } = await setup(200);
		tui.scrollToTop();
		tui.scrollByLines(180);
		const deep = tui.getScrollPosition()!.top;
		assert.ok(deep > 20, "started well down the transcript");

		transcript.lines = transcript.lines.slice(0, 30);
		tui.requestRender();
		await settle(terminal);

		const position = tui.getScrollPosition();
		assert.ok(position !== null, "still pinned");
		assert.equal(position.top, 30 - VIEW, "clamped to the new end");
		assert.equal((await window(terminal))[0], `line ${30 - VIEW + 1}`);
	});
});

describe("moving and leaving", () => {
	it("pages by a screen less two rows, so the jump stays readable", async () => {
		const { tui } = await setup();
		tui.scrollByLines(-1);
		const from = tui.getScrollPosition()!.top;
		tui.scrollByPages(-1);
		assert.equal(from - tui.getScrollPosition()!.top, VIEW - 2);
	});

	it("goes to the very first row of the session", async () => {
		const { tui, terminal } = await setup();
		tui.scrollToTop();
		assert.equal(tui.getScrollPosition()!.top, 0);
		assert.equal((await window(terminal))[0], "line 1");
	});

	it("releases the pin on reaching the bottom rather than sticking there", async () => {
		// A pinned view of the tail looks exactly like a live one and silently
		// stops following, which is the confusion this whole change is about.
		const { tui } = await setup();
		tui.scrollByLines(-4);
		assert.equal(tui.scrollPinned, true);
		tui.scrollByLines(10);
		assert.equal(tui.scrollPinned, false);
	});

	it("releases on a wheel-down that overshoots the bottom", async () => {
		const { tui, terminal } = await setup();
		wheelUp(terminal, 2);
		wheelDown(terminal, 10);
		assert.equal(tui.scrollPinned, false);
	});

	it("does nothing at all when the transcript fits on screen", async () => {
		const { tui } = await setup(4);
		assert.equal(tui.scrollByLines(-3), false);
		assert.equal(tui.scrollByPages(-1), false);
		assert.equal(tui.scrollToTop(), false);
		assert.equal(tui.scrollPinned, false);
	});
});

describe("the indicator", () => {
	it("says where the window is", async () => {
		const { tui, terminal } = await setup();
		tui.scrollByLines(-20);
		const status = await statusRow(terminal);
		const { top } = tui.getScrollPosition()!;
		assert.match(status, new RegExp(`${top + 1}.*${top + VIEW}.*100`));
	});

	it("says so at the top", async () => {
		const { tui, terminal } = await setup();
		tui.scrollToTop();
		assert.match(await statusRow(terminal), /top/);
	});

	it("can be replaced by the app, in the app's own theme", async () => {
		const { tui, terminal } = await setup();
		tui.setScrollStatusFormatter((status) => `<<${status.top}/${status.total}>>`);
		tui.scrollToTop();
		assert.match(await statusRow(terminal), /<<1\/100>>/);
	});

	it("never costs a transcript row", async () => {
		// The window is height-1; a formatter that returned a taller string would
		// push the view off the top, so the row count is what is asserted.
		const { tui, terminal } = await setup();
		tui.scrollToTop();
		const viewport = await terminal.flushAndGetViewport();
		assert.equal(viewport.length, HEIGHT);
		assert.equal(viewport[VIEW - 1].trimEnd(), `line ${VIEW}`);
	});
});

describe("the screen it paints on", () => {
	it("takes the alternate screen while pinned and gives it back", async () => {
		// The normal screen — and the scrollback behind it — has to survive intact,
		// because that is what the terminal's own search and selection work on.
		const { tui, terminal } = await setup();
		tui.scrollByLines(-10);
		await terminal.flush();
		assert.equal(await isAlternate(terminal), true);

		tui.scrollToLive();
		await settle(terminal);
		assert.equal(await isAlternate(terminal), false);
	});

	it("shows what arrived while the reader was away, without replaying the session", async () => {
		// The exit is an ordinary differential frame against a snapshot taken on
		// the way in, not a clear-and-replay. On a long session the replay is a
		// visible flash and a burst of output, and it would throw away the
		// scrollback that leaving the alternate screen just handed back.
		const { tui, terminal, transcript } = await setup(12);
		const redrawsBefore = tui.fullRedraws;

		tui.scrollToTop();
		transcript.append("arrived while reading");
		tui.requestRender();
		await settle(terminal);

		tui.scrollToLive();
		await settle(terminal);

		const viewport = await terminal.flushAndGetViewport();
		assert.equal(viewport[viewport.length - 1].trimEnd(), "arrived while reading", "the new line is on screen");
		assert.equal(tui.fullRedraws, redrawsBefore, "and nothing was cleared to put it there");
	});

	it("does take the full redraw when the terminal was resized while pinned", async () => {
		// The normal screen that comes back was drawn at the old size, so a
		// differential frame would be addressing rows that have moved.
		const { tui, terminal } = await setup(60);
		tui.scrollToTop();
		await settle(terminal);
		const redrawsBefore = tui.fullRedraws;

		terminal.resize(WIDTH - 8, HEIGHT);
		await settle(terminal);
		tui.scrollToLive();
		await settle(terminal);

		assert.ok(tui.fullRedraws > redrawsBefore, "repainted from scratch at the new size");
	});

	it("leaves the alternate screen on stop, however it was left", async () => {
		const { tui, terminal } = await setup();
		tui.scrollByLines(-10);
		await terminal.flush();
		tui.stop();
		await terminal.flush();
		assert.equal(await isAlternate(terminal), false);
	});
});

/**
 * Whether the terminal is showing its alternate buffer.
 *
 * xterm.js names the active buffer, which is the only honest way to check this
 * — asserting on the escape bytes would pass for a sequence that was written
 * but rejected.
 */
async function isAlternate(terminal: VirtualTerminal): Promise<boolean> {
	await terminal.flush();
	return terminal.bufferType === "alternate";
}

describe("a component still gets its keys", () => {
	it("passes non-mouse input through to the focused component", async () => {
		const { tui, terminal } = await setup();
		const seen: string[] = [];
		const sink: Component & { handleInput(data: string): void } = {
			invalidate() {},
			render: () => ["sink"],
			handleInput(data: string) {
				seen.push(data);
			},
		};
		tui.addChild(new Text("x"));
		tui.setFocus(sink);
		terminal.sendInput("\x1b[<64;1;1M\x1b[A");
		assert.deepEqual(seen, ["\x1b[A"], "the wheel was eaten, the arrow was not");
	});

	it("never lets a mouse report reach a text field", async () => {
		const { tui, terminal } = await setup();
		const seen: string[] = [];
		tui.setFocus({
			invalidate() {},
			render: () => ["sink"],
			handleInput(data: string) {
				seen.push(data);
			},
		});
		terminal.sendInput("\x1b[<0;12;4M");
		terminal.sendInput("\x1b[<0;12;4m");
		assert.deepEqual(seen, []);
	});
});

describe("searching the pinned view", () => {
	/** A transcript where the rows carrying a term are known by construction. */
	async function searchable(): Promise<{ tui: TUI; terminal: VirtualTerminal }> {
		const terminal = new VirtualTerminal(WIDTH, HEIGHT);
		const tui = new TUI(terminal);
		const transcript = new Transcript(0);
		// needle on rows 5, 25 and 45; everything else is filler.
		transcript.lines = Array.from({ length: 60 }, (_, i) =>
			i === 5 || i === 25 || i === 45 ? `has the Needle here` : `filler ${i}`,
		);
		tui.addChild(transcript);
		tui.start();
		await settle(terminal);
		return { tui, terminal };
	}

	it("finds every row carrying the term, ignoring case", async () => {
		const { tui } = await searchable();
		assert.equal(tui.setScrollSearch("needle"), 3);
	});

	it("starts from the most recent match, as reverse-search should", async () => {
		// What you are looking for in a session is behind you, and the newest hit
		// is nearly always the one you meant.
		const { tui } = await searchable();
		tui.setScrollSearch("needle");
		const { top, viewHeight } = tui.getScrollPosition()!;
		assert.ok(top <= 45 && 45 < top + viewHeight, `row 45 not in view (top ${top})`);
	});

	it("steps back through older matches and wraps", async () => {
		const { tui } = await searchable();
		tui.setScrollSearch("needle");
		assert.equal(tui.scrollSearchStep(-1), true);
		let position = tui.getScrollPosition()!;
		assert.ok(position.top <= 25 && 25 < position.top + position.viewHeight, "stepped to row 25");

		tui.scrollSearchStep(-1);
		position = tui.getScrollPosition()!;
		assert.ok(position.top <= 5 && 5 < position.top + position.viewHeight, "stepped to row 5");

		// Wrapping matters: a search that stops dead makes you retype it.
		tui.scrollSearchStep(-1);
		position = tui.getScrollPosition()!;
		assert.ok(position.top <= 45 && 45 < position.top + position.viewHeight, "wrapped to row 45");
	});

	it("reports a query that matches nothing without moving the view", async () => {
		const { tui } = await searchable();
		tui.scrollToTop();
		const before = tui.getScrollPosition()!.top;
		assert.equal(tui.setScrollSearch("nothingmatchesthis"), 0);
		assert.equal(tui.getScrollPosition()!.top, before);
	});

	it("marks the matches on screen", async () => {
		const { tui, terminal } = await searchable();
		tui.setScrollSearch("needle");
		await terminal.flush();
		// Reverse video is what the marker is; asserting on the bytes is the only
		// way to know the highlight survived the slice-by-column reassembly.
		assert.match(terminal.getLastWrite(), /\x1b\[7m/);
	});

	it("tells the indicator where in the matches it is", async () => {
		const { tui, terminal } = await searchable();
		tui.setScrollStatusFormatter((status) =>
			status.search ? `q=${status.search.query} ${status.search.index}/${status.search.count}` : "no search",
		);
		tui.setScrollSearch("needle");
		assert.match(await statusRow(terminal), /q=needle 3\/3/);
		tui.scrollSearchStep(-1);
		assert.match(await statusRow(terminal), /q=needle 2\/3/);
	});

	it("re-runs itself when the transcript grows underneath it", async () => {
		// Matches are measured once per query, not per frame. Skipping the
		// re-measure is cheap and wrong in a way people notice.
		const terminal = new VirtualTerminal(WIDTH, HEIGHT);
		const tui = new TUI(terminal);
		const transcript = new Transcript(60, "filler");
		tui.addChild(transcript);
		tui.start();
		await settle(terminal);

		tui.setScrollSearch("arrived");
		assert.equal(tui.getScrollPosition(), null, "nothing matched, so nothing pinned");

		tui.scrollToTop();
		transcript.append("arrived later");
		tui.requestRender();
		await settle(terminal);

		assert.equal(tui.scrollSearchStep(1), true, "the new row is findable");
	});

	it("ends with the pinned view it belongs to", async () => {
		const { tui } = await searchable();
		tui.setScrollSearch("needle");
		assert.equal(tui.scrollSearchActive, true);
		tui.scrollToLive();
		assert.equal(tui.scrollSearchActive, false);
	});
});
