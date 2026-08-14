import assert from "node:assert";
import { describe, it } from "node:test";
import type { Terminal as XtermTerminalType } from "@xterm/headless";
import { type Component, CURSOR_MARKER, TUI } from "../src/tui.js";
import { VirtualTerminal } from "./virtual-terminal.js";

/** Component whose lines the test drives directly. */
class TestComponent implements Component {
	constructor(public lines: string[] = []) {}
	render(_width: number): string[] {
		return this.lines;
	}
	invalidate(): void {}
}

class LoggingVirtualTerminal extends VirtualTerminal {
	writes: string[] = [];

	override write(data: string): void {
		this.writes.push(data);
		super.write(data);
	}

	clearWrites(): void {
		this.writes = [];
	}
}

function getCursor(terminal: VirtualTerminal): { row: number; col: number } {
	const xterm = (terminal as unknown as { xterm: XtermTerminalType }).xterm;
	const buffer = xterm.buffer.active;
	return { row: buffer.cursorY, col: buffer.cursorX };
}

const SYNC_END = "\x1b[?2026l";

/** A full-width line, the shape every padded component line has. */
function padded(text: string, width: number): string {
	return text + " ".repeat(Math.max(0, width - text.length));
}

describe("hardware cursor parking", () => {
	it("moves the cursor inside the frame's synchronized-output block", async () => {
		const width = 40;
		const terminal = new LoggingVirtualTerminal(width, 10);
		const tui = new TUI(terminal, true);
		// A status line that animates above a focused editor line, which is the
		// layout that made the stray cursor visible.
		const status = new TestComponent([padded("O Working...", width)]);
		const editor = new TestComponent([padded(`> hi${CURSOR_MARKER}`, width)]);
		tui.addChild(status);
		tui.addChild(editor);

		tui.start();
		await terminal.waitForRender();
		terminal.clearWrites();

		// Animate the status line only, exactly as the Loader does.
		status.lines = [padded("* Working...", width)];
		tui.requestRender();
		await terminal.waitForRender();

		const frames = terminal.writes.filter((w) => w.length > 0);
		assert.equal(frames.length, 1, "an animation tick should be a single write, not frame-then-cursor-move");

		const frame = frames[0];
		const syncEnd = frame.indexOf(SYNC_END);
		assert.ok(syncEnd !== -1, "frame should be wrapped in synchronized output");
		assert.ok(
			frame.indexOf("\x1b[3G", syncEnd) === -1 && frame.slice(syncEnd + SYNC_END.length).trim() === "",
			"nothing may be emitted after synchronized output ends",
		);
		// The column move that parks the cursor must land inside the block.
		assert.ok(frame.slice(0, syncEnd).includes("\x1b[5G"), "cursor column move should be inside the block");
	});

	it("leaves the cursor on the focused line, not at the end of the animated line", async () => {
		const width = 40;
		const terminal = new LoggingVirtualTerminal(width, 10);
		const tui = new TUI(terminal, true);
		const status = new TestComponent([padded("O Working...", width)]);
		const editor = new TestComponent([padded(`> hi${CURSOR_MARKER}`, width)]);
		tui.addChild(status);
		tui.addChild(editor);

		tui.start();
		await terminal.waitForRender();

		for (const frame of ["* Working...", "O Working...", "* Working..."]) {
			status.lines = [padded(frame, width)];
			tui.requestRender();
			await terminal.waitForRender();

			const cursor = getCursor(terminal);
			assert.deepEqual(cursor, { row: 1, col: 4 }, `cursor drifted after redrawing "${frame}"`);
		}
	});

	it("parks the cursor at column 0 when nothing is focused", async () => {
		const width = 40;
		const terminal = new LoggingVirtualTerminal(width, 10);
		const tui = new TUI(terminal, false);
		const status = new TestComponent([padded("O Working...", width)]);
		tui.addChild(status);

		tui.start();
		await terminal.waitForRender();

		status.lines = [padded("* Working...", width)];
		tui.requestRender();
		await terminal.waitForRender();

		// Writing a full-width line leaves the cursor in the terminal's pending
		// wrap state at the right margin, where it can render on the next row.
		const cursor = getCursor(terminal);
		assert.equal(cursor.col, 0, "cursor should be parked at column 0, not at the right margin");
		assert.equal(cursor.row, 0, "cursor should stay on the row it was left on");
	});
});
