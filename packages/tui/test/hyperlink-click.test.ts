/**
 * Clicking a link while the app has the mouse.
 *
 * The bug: mouse reporting is on for the wheel's sake, and a terminal whose
 * mouse is captured stops resolving OSC 8 clicks itself — the report comes to
 * the app instead. So every hyperlink in the transcript quietly stopped being
 * clickable the day the wheel started working. The app owes the same answer the
 * terminal used to give, which means finding the link under a *screen cell* and
 * handing its URL out.
 */

import assert from "node:assert";
import { describe, it } from "node:test";
import { Text } from "../src/components/text.js";
import { type Component, TUI } from "../src/tui.js";
import { hyperlinkAt } from "../src/utils.js";
import { VirtualTerminal } from "./virtual-terminal.js";

const link = (url: string, label: string) => `\x1b]8;;${url}\x07${label}\x1b]8;;\x07`;

describe("hyperlinkAt", () => {
	it("finds the link under a column and nothing outside it", () => {
		const line = `see ${link("https://example.com", "docs")} now`;
		assert.equal(hyperlinkAt(line, 0), undefined);
		assert.equal(hyperlinkAt(line, 3), undefined);
		assert.equal(hyperlinkAt(line, 4), "https://example.com");
		assert.equal(hyperlinkAt(line, 7), "https://example.com");
		assert.equal(hyperlinkAt(line, 8), undefined);
		assert.equal(hyperlinkAt(line, 99), undefined);
	});

	it("counts cells, not characters", () => {
		// A wide glyph is two columns to the mouse and one code point to the
		// string. Counting the string would put the link two cells left of where
		// the pointer actually was.
		const line = `🙂🙂${link("https://example.com", "x")}`;
		assert.equal(hyperlinkAt(line, 3), undefined);
		assert.equal(hyperlinkAt(line, 4), "https://example.com");
	});

	it("keeps two links on one line apart", () => {
		const line = `${link("https://a.test", "aa")}--${link("https://b.test", "bb")}`;
		assert.equal(hyperlinkAt(line, 0), "https://a.test");
		assert.equal(hyperlinkAt(line, 2), undefined);
		assert.equal(hyperlinkAt(line, 4), "https://b.test");
	});

	it("declines a line with no link at all without scanning it", () => {
		assert.equal(hyperlinkAt("plain text", 2), undefined);
		assert.equal(hyperlinkAt(`\x1b[31mred\x1b[0m`, 1), undefined);
	});
});

/** Fills the screen, with one link on a row the test knows the number of. */
class Page implements Component {
	constructor(
		private readonly rows: number,
		private readonly linkRow: number,
	) {}
	invalidate(): void {}
	render(_width: number): string[] {
		return Array.from({ length: this.rows }, (_, i) =>
			i === this.linkRow ? `go ${link("https://example.com", "here")}` : `row ${i}`,
		);
	}
}

const HEIGHT = 10;

async function settle(terminal: VirtualTerminal): Promise<void> {
	await new Promise((resolve) => setTimeout(resolve, 40));
	await terminal.flush();
}

async function setup(linkRow: number) {
	const terminal = new VirtualTerminal(40, HEIGHT);
	const tui = new TUI(terminal);
	const opened: string[] = [];
	tui.onHyperlink = (url) => opened.push(url);
	tui.addChild(new Page(HEIGHT, linkRow));
	tui.start();
	await settle(terminal);
	return { terminal, tui, opened };
}

/** A press and a release of the left button on the same cell: one click. */
function click(terminal: VirtualTerminal, row: number, column: number): void {
	terminal.sendInput(`\x1b[<0;${column};${row}M`);
	terminal.sendInput(`\x1b[<0;${column};${row}m`);
}

describe("clicking a link", () => {
	it("opens the URL under the pointer", async () => {
		const { terminal, opened } = await setup(4);
		// Row 5 on screen is buffer row 4; the label starts at column 4.
		click(terminal, 5, 5);
		assert.deepEqual(opened, ["https://example.com"]);
	});

	it("does nothing for a click beside the link", async () => {
		const { terminal, opened } = await setup(4);
		click(terminal, 5, 1);
		click(terminal, 4, 5);
		assert.deepEqual(opened, []);
	});

	it("does not take a drag for a click", async () => {
		// Press on the link, release three cells away: that is a selection, and
		// opening a browser at the end of it is the worst possible reading.
		const { terminal, opened } = await setup(4);
		terminal.sendInput("\x1b[<0;5;5M");
		terminal.sendInput("\x1b[<0;8;5m");
		assert.deepEqual(opened, []);
	});

	it("ignores the right button", async () => {
		const { terminal, opened } = await setup(4);
		terminal.sendInput("\x1b[<2;5;5M");
		terminal.sendInput("\x1b[<2;5;5m");
		assert.deepEqual(opened, []);
	});

	it("leaves nothing of the report in the input stream", async () => {
		// The report must be consumed whether or not it hit a link, or it is
		// typed into whatever has focus.
		const terminal = new VirtualTerminal(40, HEIGHT);
		const tui = new TUI(terminal);
		const typed: string[] = [];
		tui.addChild(new Text("x", 0, 0));
		tui.addInputListener((data) => {
			typed.push(data);
			return undefined;
		});
		tui.start();
		await settle(terminal);
		click(terminal, 5, 5);
		assert.deepEqual(typed, []);
	});
});
