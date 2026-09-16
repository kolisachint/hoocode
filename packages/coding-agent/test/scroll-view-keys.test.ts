/**
 * The keys the pinned transcript view answers to, and the ones it must not eat.
 *
 * The mechanism is the tui's (`tui/test/scroll-viewport.test.ts`); what is
 * asserted here is the app's half — which keys are live at the prompt, which
 * only once the view is pinned, and that the pin lets go of everything else
 * rather than swallowing it. The last part is the important one: a scrolled-back
 * view that quietly ate keystrokes would be the same "where did that go"
 * complaint in a new place.
 */

import { type Component, type Terminal, TUI } from "@kolisachint/hoocode-tui";
import { beforeEach, describe, expect, it } from "vitest";
import { KeybindingsManager } from "../src/core/keybindings.js";
import { CustomEditor } from "../src/modes/interactive/components/custom-editor.js";
import { installScrollView } from "../src/modes/interactive/scroll-view.js";
import { getEditorTheme, initTheme } from "../src/modes/interactive/theme/theme.js";

const WIDTH = 60;
const HEIGHT = 12;
/** The view is one row shorter than the screen; the last row is the indicator. */
const VIEW = HEIGHT - 1;

class SilentTerminal implements Terminal {
	columns = WIDTH;
	rows = HEIGHT;
	kittyProtocolActive = false;
	alternate = false;
	start(): void {}
	stop(): void {}
	async drainInput(): Promise<void> {}
	write(): void {}
	moveBy(): void {}
	hideCursor(): void {}
	showCursor(): void {}
	clearLine(): void {}
	clearFromCursor(): void {}
	clearScreen(): void {}
	setTitle(): void {}
	setProgress(): void {}
	get mouseReporting(): boolean {
		return true;
	}
	setAlternateScreen(active: boolean): void {
		this.alternate = active;
	}
}

/** A transcript tall enough to have somewhere to scroll to. */
class Transcript implements Component {
	constructor(private readonly count: number) {}
	invalidate(): void {}
	render(): string[] {
		return Array.from({ length: this.count }, (_, i) => `line ${i + 1}`);
	}
}

interface Harness {
	ui: TUI;
	terminal: SilentTerminal;
	editor: CustomEditor;
	send(data: string): void;
}

function setup(lines = 120): Harness {
	initTheme("dark");
	const terminal = new SilentTerminal();
	const ui = new TUI(terminal);
	const keybindings = new KeybindingsManager();
	const editor = new CustomEditor(ui, getEditorTheme(), keybindings);
	ui.addChild(new Transcript(lines));
	ui.addChild(editor);
	installScrollView(ui, editor, keybindings);
	ui.setFocus(editor);
	// One frame, so there is a line buffer to scroll through.
	(ui as unknown as { doRender(): void }).doRender();
	return {
		ui,
		terminal,
		editor,
		send: (data: string) => (ui as unknown as { handleInput(d: string): void }).handleInput(data),
	};
}

const PAGE_UP = "\x1b[5~";
const PAGE_DOWN = "\x1b[6~";
const UP = "\x1b[A";
const DOWN = "\x1b[B";
const ESCAPE = "\x1b";
const WHEEL_UP = "\x1b[<64;1;1M";

describe("at the prompt", () => {
	let harness: Harness;
	beforeEach(() => {
		harness = setup();
	});

	it("pins the view on page up", () => {
		harness.send(PAGE_UP);
		expect(harness.ui.scrollPinned).toBe(true);
	});

	it("leaves the arrows to prompt history", () => {
		// Losing history to scrolling would trade one surprise for another, and
		// the arrows are the prompt's own keys.
		harness.editor.addToHistory("an earlier message");
		harness.send(UP);
		expect(harness.ui.scrollPinned).toBe(false);
		expect(harness.editor.getText()).toBe("an earlier message");
	});

	it("does nothing on page down, having nowhere below to go", () => {
		harness.send(PAGE_DOWN);
		expect(harness.ui.scrollPinned).toBe(false);
	});

	it("will not pin while another surface holds focus", () => {
		const other: Component = { invalidate() {}, render: () => ["picker"] };
		harness.ui.setFocus(other);
		harness.send(WHEEL_UP);
		expect(harness.ui.scrollPinned).toBe(false);
	});
});

describe("once pinned", () => {
	let harness: Harness;
	beforeEach(() => {
		harness = setup();
		harness.send(PAGE_UP);
	});

	it("moves a line at a time on the arrows", () => {
		const before = harness.ui.getScrollPosition()!.top;
		harness.send(UP);
		expect(harness.ui.getScrollPosition()!.top).toBe(before - 1);
		harness.send(DOWN);
		expect(harness.ui.getScrollPosition()!.top).toBe(before);
	});

	it("does not type the arrows into the prompt", () => {
		harness.send(UP);
		harness.send(DOWN);
		expect(harness.editor.getText()).toBe("");
	});

	it("pages further back", () => {
		const before = harness.ui.getScrollPosition()!.top;
		harness.send(PAGE_UP);
		expect(harness.ui.getScrollPosition()!.top).toBe(before - (VIEW - 2));
	});

	it("goes back to live on escape", () => {
		harness.send(ESCAPE);
		expect(harness.ui.scrollPinned).toBe(false);
	});

	it("goes back to live on paging past the bottom", () => {
		for (let i = 0; i < 20; i++) harness.send(PAGE_DOWN);
		expect(harness.ui.scrollPinned).toBe(false);
	});

	it("returns to live when you start typing, and types the character", () => {
		// The rule that makes the mode invisible: you stop reading by doing
		// something else, not by remembering to press escape first. Typing into a
		// pinned view would echo into a prompt that is scrolled off screen.
		harness.send("h");
		expect(harness.ui.scrollPinned).toBe(false);
		expect(harness.editor.getText()).toBe("h");
	});

	it("takes the alternate screen and gives it back", () => {
		expect(harness.terminal.alternate).toBe(true);
		harness.send(ESCAPE);
		expect(harness.terminal.alternate).toBe(false);
	});
});
