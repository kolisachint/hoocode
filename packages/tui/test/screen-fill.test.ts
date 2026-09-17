/**
 * The app is the size of the screen.
 *
 * The bug this exists for: this renderer appends, so a frame was written from
 * wherever the cursor happened to be and was exactly as tall as its content. On
 * a fresh session that meant the banner halfway up the terminal, the prompt
 * under it, and the user's shell history above — and the prompt walking down
 * the screen over the next few turns until the session was finally long enough
 * to scroll. Two layouts for one app, and the one you meet first is the one
 * that does not look like an app at all.
 *
 * So the property under test throughout is *the last row*: whatever the session
 * has in it, the last row of the buffer is the last row of the screen, and the
 * first row of the tree is the first row of the screen until there is too much
 * to fit.
 */

import assert from "node:assert";
import { describe, it } from "node:test";
import { FlexSpacer } from "../src/components/spacer.js";
import { Text } from "../src/components/text.js";
import { type Component, TUI } from "../src/tui.js";
import { VirtualTerminal } from "./virtual-terminal.js";

const WIDTH = 30;
const HEIGHT = 12;

/** A stand-in transcript whose height the test drives. */
class Body implements Component {
	private lines: string[] = [];
	constructor(count: number) {
		this.setCount(count);
	}
	setCount(count: number): void {
		this.lines = Array.from({ length: count }, (_, i) => `body ${i + 1}`);
	}
	invalidate(): void {}
	render(_width: number): string[] {
		return this.lines;
	}
}

/** Header at the top, fill, then the chrome that has to end up on the floor. */
async function setup(bodyRows: number, rows = HEIGHT) {
	const terminal = new VirtualTerminal(WIDTH, rows);
	const tui = new TUI(terminal);
	const body = new Body(bodyRows);
	const fill = new FlexSpacer();
	// `chrome` stands in for everything below the fill in the real tree — the
	// ledger, the notification band, the prompt, the footer — and grows the way a
	// picker taking the prompt's place does.
	const chrome = new Body(0);
	tui.addChild(new Text("header", 0, 0));
	tui.addChild(body);
	tui.addChild(fill);
	tui.addChild(chrome);
	tui.addChild(new Text("prompt", 0, 0));
	tui.setFlexSpacer(fill);
	tui.start();
	await settle(terminal);
	return { tui, terminal, body, chrome, fill };
}

async function settle(terminal: VirtualTerminal): Promise<void> {
	await new Promise((resolve) => setTimeout(resolve, 40));
	await terminal.flush();
}

async function screen(terminal: VirtualTerminal): Promise<string[]> {
	const viewport = await terminal.flushAndGetViewport();
	return viewport.map((row) => row.trimEnd());
}

describe("filling the screen", () => {
	it("puts the header on the first row and the prompt on the last", async () => {
		const { terminal } = await setup(2);
		const rows = await screen(terminal);
		assert.equal(rows[0], "header");
		assert.equal(rows[HEIGHT - 1], "prompt");
		// The gap is blank, not a repeat of anything: the fill is empty rows.
		assert.deepEqual(rows.slice(3, HEIGHT - 1), Array(HEIGHT - 4).fill(""));
		assert.deepEqual(rows.slice(1, 3), ["body 1", "body 2"]);
	});

	it("keeps the prompt on the last row as the session grows", async () => {
		const { terminal, body, tui } = await setup(2);
		for (const count of [3, 5, 8]) {
			body.setCount(count);
			tui.requestRender();
			await settle(terminal);
			const rows = await screen(terminal);
			assert.equal(rows[HEIGHT - 1], "prompt", `at ${count} body rows`);
			assert.equal(rows[0], "header", `at ${count} body rows`);
		}
	});

	it("gives up the fill once the content is taller than the screen", async () => {
		const { terminal, body, tui, fill } = await setup(2);
		body.setCount(HEIGHT * 2);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, 0);
		// Still the last row — now because the transcript scrolled, not because
		// anything was padded.
		const rows = await screen(terminal);
		assert.equal(rows[HEIGHT - 1], "prompt");
	});

	it("takes the fill back when the content shrinks again", async () => {
		const { terminal, body, tui, fill } = await setup(HEIGHT * 2);
		assert.equal(fill.currentHeight, 0);
		body.setCount(1);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, HEIGHT - 3);
	});

	it("refits on a resize", async () => {
		const { terminal, tui, fill } = await setup(2);
		assert.equal(fill.currentHeight, HEIGHT - 4);
		terminal.resize(WIDTH, HEIGHT + 6);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, HEIGHT + 6 - 4);
	});

	it("gives its rows to a picker and takes them back when it closes", async () => {
		// Every surface that asks the user something replaces the prompt inside
		// the same container, so it is below the fill: it grows downward from a
		// floor that does not move, rather than pushing the prompt off the bottom
		// and leaving it stranded mid-screen when it closes.
		const { terminal, tui, chrome, fill } = await setup(2);
		const before = fill.currentHeight;
		chrome.setCount(6);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, before - 6);
		assert.equal((await screen(terminal))[HEIGHT - 1], "prompt");

		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, before);
		assert.equal((await screen(terminal))[HEIGHT - 1], "prompt");
	});

	it("is not transcript, so a screenful of nothing cannot be scrolled", async () => {
		// The fill makes the buffer exactly as tall as the screen. Counting it as
		// transcript would leave a session with two rows in it one row short of
		// the window height and therefore pinnable — onto a screen of blanks.
		const { tui } = await setup(2);
		assert.equal(tui.scrollByLines(-1), false);
		assert.equal(tui.scrollPinned, false);
	});

	it("keeps the screen the tail of the buffer as the session grows", async () => {
		// The fill changes the buffer's length on frames that used to leave it
		// alone, and this renderer's diff is positional — so the property worth
		// asserting is not the fill's arithmetic but that what is on screen is
		// still the tail of the buffer afterwards. Growth is the session's own
		// path: a transcript only gets longer until something clears it.
		const { terminal, tui, body } = await setup(2);
		for (const count of [2, 6, HEIGHT - 4, HEIGHT, HEIGHT + 1, HEIGHT * 2, HEIGHT * 3]) {
			body.setCount(count);
			tui.requestRender();
			await settle(terminal);
			const expected = tui
				.render(WIDTH)
				.slice(-HEIGHT)
				.map((line) => line.trimEnd());
			assert.deepEqual(await screen(terminal), expected, `at ${count} body rows`);
		}
	});

	it("leaves no stale rows behind when a long session is cleared", async () => {
		// Without a fill this is the renderer's known weak spot: shrinking the
		// buffer leaves whatever was under the old content on screen, because
		// clearing on shrink is off by default. A buffer that never gets shorter
		// than the screen cannot have rows below it to leave behind, so `/clear`
		// on a long session lands on a clean screen rather than a short app with
		// the tail of the last one still under it.
		const { terminal, tui, body } = await setup(HEIGHT * 3);
		body.setCount(1);
		tui.requestRender();
		await settle(terminal);
		assert.deepEqual(await screen(terminal), ["header", "body 1", ...Array(HEIGHT - 3).fill(""), "prompt"]);
	});

	it("costs no more full redraws than the same tree without a fill", async () => {
		// A full redraw is `\x1b[2J\x1b[H\x1b[3J` — it throws the reader's
		// scrollback away. The fill must not be a new reason to reach for one.
		const steps = [2, 6, HEIGHT, HEIGHT * 2, HEIGHT + 1, 4, 1];

		const filled = await setup(2);
		for (const count of steps) {
			filled.body.setCount(count);
			filled.tui.requestRender();
			await settle(filled.terminal);
		}

		const plainTerminal = new VirtualTerminal(WIDTH, HEIGHT);
		const plain = new TUI(plainTerminal);
		const plainBody = new Body(2);
		plain.addChild(new Text("header", 0, 0));
		plain.addChild(plainBody);
		plain.addChild(new Text("prompt", 0, 0));
		plain.start();
		await settle(plainTerminal);
		for (const count of steps) {
			plainBody.setCount(count);
			plain.requestRender();
			await settle(plainTerminal);
		}

		assert.ok(
			filled.tui.fullRedraws <= plain.fullRedraws,
			`filled=${filled.tui.fullRedraws} plain=${plain.fullRedraws}`,
		);
	});

	it("has nothing to do for a tree with no fill in it", async () => {
		// Embedders that predate this — and every test that renders the tree
		// directly — get the old append-only frame back by simply not setting one.
		const terminal = new VirtualTerminal(WIDTH, HEIGHT);
		const tui = new TUI(terminal);
		tui.addChild(new Text("header", 0, 0));
		tui.addChild(new Text("prompt", 0, 0));
		tui.start();
		await settle(terminal);
		const rows = await screen(terminal);
		assert.equal(rows[0], "header");
		assert.equal(rows[1], "prompt");
		assert.equal(rows[HEIGHT - 1], "");
	});
});

describe("the fill and the pinned view", () => {
	it("empties the fill while pinned, so blanks are never scrolled through", async () => {
		const { tui, terminal, body, fill } = await setup(2);
		assert.ok(fill.currentHeight > 0);
		body.setCount(HEIGHT * 3);
		tui.requestRender();
		await settle(terminal);
		assert.ok(tui.scrollByLines(-5));
		await settle(terminal);
		assert.equal(fill.currentHeight, 0);
	});

	it("puts the prompt back on the floor on the way out of a pinned view", async () => {
		// Scrolling down until the pin lets go is how you get back to live, and
		// the screen you land on has to be the screen you left — not the app
		// halfway up the terminal because the fill never came back.
		const { tui, terminal, body, fill } = await setup(2);
		body.setCount(HEIGHT * 3);
		tui.requestRender();
		await settle(terminal);
		tui.scrollByLines(-5);
		await settle(terminal);
		assert.equal(tui.scrollPinned, true);

		body.setCount(1);
		tui.scrollToLive();
		await settle(terminal);
		assert.equal(tui.scrollPinned, false);
		assert.equal(fill.currentHeight, HEIGHT - 3);
		const rows = await screen(terminal);
		assert.equal(rows[HEIGHT - 1], "prompt");
	});
});

describe("the floor holds when something above the prompt goes away", () => {
	/**
	 * The bug: on a session long enough to have scrolled, the buffer is taller
	 * than the screen and the *terminal's* scroll is what puts the prompt on the
	 * bottom row. Shrink the buffer — a picker closing, a notification fading,
	 * the ledger emptying — and the renderer clears the rows that came off the
	 * end, leaving the prompt stranded mid-screen with a band of blanks under
	 * it. Nothing can scroll the transcript back down into them: those rows
	 * belong to the terminal's scrollback.
	 *
	 * So the fill takes what the chrome gave up and the buffer keeps its length.
	 * The blank band lands above the prompt, where it reads as room, instead of
	 * below it, where it reads as a layout that came apart.
	 */
	const longSession = () => setup(HEIGHT * 2);

	it("keeps the prompt on the floor when a picker closes", async () => {
		const { terminal, tui, chrome, fill } = await longSession();
		assert.equal(fill.currentHeight, 0);
		chrome.setCount(6);
		tui.requestRender();
		await settle(terminal);
		assert.equal((await screen(terminal))[HEIGHT - 1], "prompt");

		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		const rows = await screen(terminal);
		assert.equal(rows[HEIGHT - 1], "prompt", "prompt is still on the bottom row");
		// The six rows the pane gave back are above the prompt, not below it.
		assert.equal(fill.currentHeight, 6);
		assert.deepEqual(rows.slice(HEIGHT - 7, HEIGHT - 1), Array(6).fill(""));
	});

	it("keeps the prompt on the floor when a notification fades", async () => {
		const { terminal, tui, chrome } = await longSession();
		chrome.setCount(2);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		assert.equal((await screen(terminal))[HEIGHT - 1], "prompt");
	});

	it("spends the held rows on the next output rather than scrolling", async () => {
		// The band is not a hole in the layout, it is room: what arrives next
		// lands in it, so the screen settles back to a full one without the
		// transcript jumping.
		const { terminal, tui, body, chrome, fill } = await longSession();
		chrome.setCount(6);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, 6);

		body.setCount(HEIGHT * 2 + 4);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, 2);
		const rows = await screen(terminal);
		assert.equal(rows[HEIGHT - 1], "prompt");
		assert.equal(rows[HEIGHT - 4], "body 28");
	});

	it("never holds more than a screenful", async () => {
		// A fill longer than the screen is rows nobody can see, and a buffer
		// that keeps growing them is a buffer that never stops growing.
		const { terminal, tui, chrome, fill } = await setup(HEIGHT * 4);
		chrome.setCount(HEIGHT * 2);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		assert.ok(fill.currentHeight <= HEIGHT, `held ${fill.currentHeight} rows`);
		assert.equal((await screen(terminal))[HEIGHT - 1], "prompt");
	});

	it("gives the held rows up on a resize, which repaints anyway", async () => {
		// A resize clears and reprints the screen, so there is no floor left to
		// hold — banking the old buffer's length would only put a band of blanks
		// into the middle of the screen it is about to draw.
		const { terminal, tui, chrome, fill } = await longSession();
		chrome.setCount(6);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, 6);

		terminal.resize(WIDTH, HEIGHT - 2);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, 0);
		const rows = await screen(terminal);
		assert.equal(rows[HEIGHT - 3], "prompt");
		assert.equal(rows[HEIGHT - 4], "body 24");
	});

	it("still starts a short session on the first row", async () => {
		// The hold is for a buffer the terminal has already scrolled. A session
		// that fits on the screen is fitted to it exactly, so the header stays
		// where it belongs whatever the chrome does.
		const { terminal, tui, chrome } = await setup(2);
		chrome.setCount(6);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		const rows = await screen(terminal);
		assert.equal(rows[0], "header");
		assert.equal(rows[HEIGHT - 1], "prompt");
	});
});
