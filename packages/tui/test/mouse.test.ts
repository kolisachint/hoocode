/**
 * Reading mouse reports.
 *
 * Two encodings have to work. SGR is what any terminal from this decade sends
 * once `?1006h` is accepted; X10 is what the ones that ignored it keep sending,
 * and a terminal that silently stayed on X10 would otherwise type `\x1b[M` and
 * three high bytes into whatever field has focus.
 */

import assert from "node:assert";
import { describe, it } from "node:test";
import { isMouseSequence, mouseSequenceLength, parseMouseEvent } from "../src/mouse.js";

describe("parseMouseEvent, SGR", () => {
	it("reads the wheel in both directions", () => {
		assert.equal(parseMouseEvent("\x1b[<64;10;5M")?.kind, "wheelUp");
		assert.equal(parseMouseEvent("\x1b[<65;10;5M")?.kind, "wheelDown");
	});

	it("names no button for a wheel event", () => {
		// A wheel report reuses the button bits for direction, so reporting a
		// button here would be reporting the direction under another name.
		assert.equal(parseMouseEvent("\x1b[<64;1;1M")?.button, -1);
	});

	it("reads horizontal wheels without mistaking them for buttons", () => {
		assert.equal(parseMouseEvent("\x1b[<66;1;1M")?.kind, "wheelLeft");
		assert.equal(parseMouseEvent("\x1b[<67;1;1M")?.kind, "wheelRight");
	});

	it("keeps coordinates 1-based, as the terminal sends them", () => {
		const event = parseMouseEvent("\x1b[<0;42;7M");
		assert.equal(event?.column, 42);
		assert.equal(event?.row, 7);
	});

	it("survives a column past the X10 ceiling", () => {
		// The whole reason ?1006 exists: X10 packs a coordinate into one byte and
		// gives up at 223.
		assert.equal(parseMouseEvent("\x1b[<0;400;9M")?.column, 400);
	});

	it("tells press from release by the terminator", () => {
		assert.equal(parseMouseEvent("\x1b[<0;1;1M")?.kind, "press");
		assert.equal(parseMouseEvent("\x1b[<0;1;1m")?.kind, "release");
	});

	it("reads each modifier off its own bit", () => {
		assert.equal(parseMouseEvent("\x1b[<4;1;1M")?.shift, true);
		assert.equal(parseMouseEvent("\x1b[<8;1;1M")?.alt, true);
		assert.equal(parseMouseEvent("\x1b[<16;1;1M")?.ctrl, true);
		const all = parseMouseEvent("\x1b[<92;1;1M");
		assert.deepEqual([all?.kind, all?.shift, all?.alt, all?.ctrl], ["wheelUp", true, true, true]);
	});

	it("names the buttons", () => {
		assert.equal(parseMouseEvent("\x1b[<0;1;1M")?.button, 0);
		assert.equal(parseMouseEvent("\x1b[<1;1;1M")?.button, 1);
		assert.equal(parseMouseEvent("\x1b[<2;1;1M")?.button, 2);
	});
});

describe("parseMouseEvent, X10", () => {
	it("reads a wheel report from a terminal that ignored ?1006", () => {
		// 64 + 32 = 96 = "`"; coordinates are their value plus 32.
		const event = parseMouseEvent(`\x1b[M\x60${String.fromCharCode(32 + 10)}${String.fromCharCode(32 + 5)}`);
		assert.equal(event?.kind, "wheelUp");
		assert.equal(event?.column, 10);
		assert.equal(event?.row, 5);
	});

	it("reads button 3 as the release it is", () => {
		// X10 has no per-button release code, so the release arrives as button 3.
		const event = parseMouseEvent(`\x1b[M${String.fromCharCode(32 + 3)}!!`);
		assert.equal(event?.kind, "release");
	});
});

describe("parseMouseEvent, rejection", () => {
	it("returns null for input that is not a report", () => {
		for (const data of ["a", "\x1b[A", "\x1b[200~", "\x1b[<", "\x1b[M", "\x1b[Mab", ""]) {
			assert.equal(parseMouseEvent(data), null, JSON.stringify(data));
		}
	});
});

describe("mouseSequenceLength", () => {
	it("measures a report so the rest of the chunk survives it", () => {
		assert.equal(mouseSequenceLength("\x1b[<64;10;5M"), 11);
		assert.equal(mouseSequenceLength("\x1b[<64;10;5Mx"), 11);
		assert.equal(mouseSequenceLength("\x1b[Mabc"), 6);
	});

	it("is zero where there is no report to measure", () => {
		assert.equal(mouseSequenceLength("x\x1b[<64;10;5M"), 0);
		assert.equal(mouseSequenceLength("\x1b[A"), 0);
	});

	it("walks a coalesced run of reports", () => {
		// A flick of the wheel arrives as one read, and a keystroke pressed during
		// the flick rides along behind it.
		let rest = "\x1b[<64;1;1M\x1b[<64;1;2M\x1b[<64;1;3M\x1b[A";
		let events = 0;
		while (mouseSequenceLength(rest) > 0) {
			const length = mouseSequenceLength(rest);
			assert.equal(parseMouseEvent(rest.slice(0, length))?.kind, "wheelUp");
			rest = rest.slice(length);
			events++;
		}
		assert.equal(events, 3);
		assert.equal(rest, "\x1b[A");
	});
});

describe("isMouseSequence", () => {
	it("recognises both encodings and nothing else", () => {
		assert.equal(isMouseSequence("\x1b[<0;1;1M"), true);
		assert.equal(isMouseSequence("\x1b[Mabc"), true);
		assert.equal(isMouseSequence("\x1b[Ma"), false);
		assert.equal(isMouseSequence("\x1b[A"), false);
	});
});
