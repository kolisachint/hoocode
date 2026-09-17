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
