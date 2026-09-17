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
 * has in it, the last row of the buffer is the last row of the screen. The fill
 * is the first child, so a session too short to reach the top gets its leftover
 * rows up there — the conversation and the prompt always touch.
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

/** Fill, header, body, then the chrome that has to end up on the floor. */
async function setup(bodyRows: number, rows = HEIGHT) {
	const terminal = new VirtualTerminal(WIDTH, rows);
	const tui = new TUI(terminal);
	const body = new Body(bodyRows);
	const fill = new FlexSpacer();
	// `chrome` stands in for everything between the body and the prompt in the
	// real tree — the ledger, the notification band — and grows the way a picker
	// taking the prompt's place does.
	const chrome = new Body(0);
	tui.addChild(fill);
	tui.addChild(new Text("header", 0, 0));
	tui.addChild(body);
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
	it("packs a short session against the floor, with the leftover rows above it", async () => {
		const { terminal } = await setup(2);
		const rows = await screen(terminal);
		assert.equal(rows[HEIGHT - 1], "prompt");
		// Header, body and prompt are one run against the bottom: nothing is held
		// back between the last thing said and the box you answer it in.
		assert.deepEqual(rows.slice(HEIGHT - 4), ["header", "body 1", "body 2", "prompt"]);
		// What is left over is blank, and it is all above the header.
		assert.deepEqual(rows.slice(0, HEIGHT - 4), Array(HEIGHT - 4).fill(""));
	});

	it("keeps the prompt on the last row as the session grows", async () => {
		const { terminal, body, tui } = await setup(2);
		for (const count of [3, 5, 8]) {
			body.setCount(count);
			tui.requestRender();
			await settle(terminal);
			const rows = await screen(terminal);
			assert.equal(rows[HEIGHT - 1], "prompt", `at ${count} body rows`);
			assert.equal(rows[HEIGHT - 2], `body ${count}`, `at ${count} body rows`);
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
		assert.deepEqual(await screen(terminal), [...Array(HEIGHT - 3).fill(""), "header", "body 1", "prompt"]);
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

	it("leaves no band between the transcript and the prompt when a view folds", async () => {
		// The tool-output dial folds a run of calls to one line, which takes a
		// long transcript down by most of its height in a single frame. The buffer
		// is still taller than the screen afterwards, so the fill has nothing to
		// give: what has to happen is that the window moves *back* over the buffer
		// and the screen becomes its new tail.
		const { terminal, tui, body } = await setup(HEIGHT * 4);
		body.setCount(Math.floor(HEIGHT * 1.5));
		tui.requestRender();
		await settle(terminal);
		const rows = await screen(terminal);
		assert.equal(rows[HEIGHT - 1], "prompt");
		assert.equal(rows[HEIGHT - 2], `body ${Math.floor(HEIGHT * 1.5)}`);
		assert.ok(
			!rows.slice(0, HEIGHT - 1).includes(""),
			`no blank band on a buffer taller than the screen: ${JSON.stringify(rows)}`,
		);
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
	 * the ledger emptying — and the append-only frame clears the rows that came
	 * off the end, leaving the prompt stranded mid-screen with a band of blanks
	 * under it. Nothing can scroll the transcript back down into them: those rows
	 * belong to the terminal's scrollback.
	 *
	 * The fill cannot answer this one — there is nothing to fill, the buffer is
	 * already taller than the screen — and the version that tried, by banking the
	 * rows the chrome gave up, paid for the floor with a blank band between the
	 * conversation and the prompt. So the window is repainted instead: the screen
	 * becomes the buffer's new tail, which is the same prompt on the same row
	 * with more of the transcript above it and no band anywhere.
	 */
	const longSession = () => setup(HEIGHT * 2);

	/** No blank row anywhere above the prompt. */
	function assertNoBand(rows: string[]): void {
		assert.equal(rows[HEIGHT - 1], "prompt", `prompt on the bottom row: ${JSON.stringify(rows)}`);
		assert.ok(!rows.slice(0, HEIGHT - 1).includes(""), `no blank band: ${JSON.stringify(rows)}`);
	}

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
		// The six rows the pane gave back are spent on six more rows of transcript,
		// not banked as blanks above the prompt.
		assertNoBand(rows);
		assert.equal(fill.currentHeight, 0);
		assert.equal(rows[HEIGHT - 2], `body ${HEIGHT * 2}`);
	});

	it("keeps the prompt on the floor when a notification fades", async () => {
		const { terminal, tui, chrome } = await longSession();
		chrome.setCount(2);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		assertNoBand(await screen(terminal));
	});

	it("appends normally again once the window has moved back", async () => {
		// The repaint is one frame, not a mode: the next row of output is an
		// append like any other, with the screen already sitting on the tail.
		const { terminal, tui, body, chrome } = await longSession();
		chrome.setCount(6);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);

		body.setCount(HEIGHT * 2 + 4);
		tui.requestRender();
		await settle(terminal);
		const rows = await screen(terminal);
		assertNoBand(rows);
		assert.equal(rows[HEIGHT - 2], `body ${HEIGHT * 2 + 4}`);
	});

	it("handles a fold bigger than the screen", async () => {
		// A pane worth two screens closing moves the window back further than the
		// screen is tall, which is the case a held fill could never have covered.
		const { terminal, tui, chrome, fill } = await setup(HEIGHT * 4);
		chrome.setCount(HEIGHT * 2);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, 0);
		assertNoBand(await screen(terminal));
	});

	it("lands on the floor after a resize too", async () => {
		const { terminal, tui, chrome, fill } = await longSession();
		chrome.setCount(6);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);

		terminal.resize(WIDTH, HEIGHT - 2);
		tui.requestRender();
		await settle(terminal);
		assert.equal(fill.currentHeight, 0);
		const rows = await screen(terminal);
		assert.equal(rows[HEIGHT - 3], "prompt");
		assert.equal(rows[HEIGHT - 4], `body ${HEIGHT * 2}`);
	});

	it("keeps a short session packed against the floor whatever the chrome does", async () => {
		// A session that fits on the screen is fitted to it exactly, so the prompt
		// is on the bottom row and the leftover rows are above the header.
		const { terminal, tui, chrome } = await setup(2);
		chrome.setCount(6);
		tui.requestRender();
		await settle(terminal);
		chrome.setCount(0);
		tui.requestRender();
		await settle(terminal);
		const rows = await screen(terminal);
		assert.equal(rows[0], "");
		assert.deepEqual(rows.slice(HEIGHT - 4), ["header", "body 1", "body 2", "prompt"]);
	});
});
