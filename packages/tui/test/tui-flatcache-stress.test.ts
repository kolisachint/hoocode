import assert from "node:assert";
import { describe, it } from "node:test";
import { Text } from "../src/components/text.js";
import { Container, CURSOR_MARKER, TUI } from "../src/tui.js";
import { VirtualTerminal } from "./virtual-terminal.js";

/**
 * Differential-vs-oracle stress test for the root flat-line cache.
 *
 * Two TUIs receive identical component mutations. The subject renders with
 * the patched flat-cache path; the oracle has the cache disabled and runs the
 * legacy full-flatten + full-diff path, which the patched path must reproduce
 * exactly (a full-redraw oracle is NOT equivalent: it re-homes scrolled-out
 * content, which differential rendering never did). After every frame the
 * visible viewports must match exactly. Mutations are driven by a seeded PRNG
 * so failures reproduce.
 */

function mulberry32(seed: number): () => number {
	let a = seed;
	return () => {
		a |= 0;
		a = (a + 0x6d2b79f5) | 0;
		let t = Math.imul(a ^ (a >>> 15), 1 | a);
		t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

interface App {
	terminal: VirtualTerminal;
	tui: TUI;
	chat: Container;
	chatItems: Text[];
	chatTexts: string[];
	spinner: Text;
	editor: Text;
}

function buildApp(disableFlatCache = false): App {
	const terminal = new VirtualTerminal(60, 12);
	const tui = new TUI(terminal);
	if (disableFlatCache) {
		// Test-only reach into internals: sawImageLine=true routes every frame
		// through the legacy full-flatten + full-diff path (the cache also
		// disables itself this way when images are on screen).
		(tui as unknown as { sawImageLine: boolean }).sawImageLine = true;
	}
	const chat = new Container();
	const chatItems: Text[] = [];
	const chatTexts: string[] = [];
	for (let i = 0; i < 3; i++) {
		const t = new Text(`message ${i} line`, 0, 0);
		chatItems.push(t);
		chatTexts.push(`message ${i} line`);
		chat.addChild(t);
	}
	const spinner = new Text("* working", 0, 0);
	const editor = new Text(`> input${CURSOR_MARKER}`, 0, 0);
	tui.addChild(chat);
	tui.addChild(spinner);
	tui.addChild(editor);
	tui.start();
	return { terminal, tui, chat, chatItems, chatTexts, spinner, editor };
}

describe("TUI flat-cache differential stress", () => {
	it("patched differential rendering matches the legacy diff path over random mutations", async () => {
		const subject = buildApp();
		const oracle = buildApp(true);
		await subject.terminal.waitForRender();
		await oracle.terminal.waitForRender();

		const rand = mulberry32(0xc0ffee);
		const frames = ["*", "+", "x", "o"];
		let appended = 0;

		for (let step = 0; step < 150; step++) {
			const roll = rand();
			const apply = (app: App): void => {
				if (roll < 0.3) {
					// Spinner tick: single-line equal-length change.
					app.spinner.setText(`${frames[step % frames.length]} working ${step}`);
				} else if (roll < 0.55) {
					// Chat growth: append a new message (splice/append path).
					const text = `appended message ${appended} with some text`;
					const t = new Text(text, 0, 0);
					app.chatItems.push(t);
					app.chatTexts.push(text);
					app.chat.addChild(t);
				} else if (roll < 0.7) {
					// Streaming-style growth: last chat item gains a line (length change
					// inside an existing child).
					const idx = app.chatItems.length - 1;
					app.chatTexts[idx] = `${app.chatTexts[idx]}\nmore ${step}`;
					app.chatItems[idx].setText(app.chatTexts[idx]);
				} else if (roll < 0.8 && app.chatItems.length > 2) {
					// Shrink: remove a middle message.
					const idx = 1 + Math.floor(rand() * (app.chatItems.length - 2));
					const [removed] = app.chatItems.splice(idx, 1);
					app.chatTexts.splice(idx, 1);
					app.chat.removeChild(removed);
				} else if (roll < 0.92) {
					// Editor typing: content + marker move (cursor logic).
					app.editor.setText(`> input ${step}${CURSOR_MARKER} tail`);
				} else {
					// Mutate an older message in place (equal or changed length).
					const idx = Math.floor(rand() * app.chatItems.length);
					app.chatItems[idx].setText(`edited ${step} message ${idx}`);
				}
			};
			// rand() consumed inside apply must be identical for both apps: capture
			// the extra roll once.
			const extraRoll = rand();
			const applyDeterministic = (app: App): void => {
				if (roll >= 0.7 && roll < 0.8 && app.chatItems.length > 2) {
					const idx = 1 + Math.floor(extraRoll * (app.chatItems.length - 2));
					const [removed] = app.chatItems.splice(idx, 1);
					app.chatTexts.splice(idx, 1);
					app.chat.removeChild(removed);
				} else if (roll >= 0.92) {
					const idx = Math.floor(extraRoll * app.chatItems.length);
					app.chatTexts[idx] = `edited ${step} message ${idx}`;
					app.chatItems[idx].setText(app.chatTexts[idx]);
				} else {
					apply(app);
				}
			};
			applyDeterministic(subject);
			applyDeterministic(oracle);
			if (roll < 0.55) appended++;

			subject.tui.requestRender();
			oracle.tui.requestRender();
			await subject.terminal.waitForRender();
			await oracle.terminal.waitForRender();

			const got = await subject.terminal.flushAndGetViewport();
			const want = await oracle.terminal.flushAndGetViewport();
			assert.deepStrictEqual(
				got,
				want,
				`viewport mismatch at step ${step} (roll=${roll.toFixed(3)})\nsubject:\n${got.join("\n")}\n---\noracle:\n${want.join("\n")}`,
			);
		}
	});
});
