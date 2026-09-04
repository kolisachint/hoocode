import { describe, expect, it } from "vitest";
import {
	cycleSessionColorSlot,
	isSessionColorSlot,
	parseSessionColorSlot,
	SESSION_COLOR_NAME_LIST,
	SESSION_COLOR_SLOTS,
	sessionColorName,
	sessionColorSlotFor,
	sessionSlugFor,
} from "../src/core/session-identity.js";

/** uuidv7s minted in the same millisecond: identical but for their random tail. */
function sameInstantIds(count: number): string[] {
	const prefix = "0195b3c1-8a40-7";
	return Array.from(
		{ length: count },
		(_, i) => `${prefix}${(i + 1).toString(16).padStart(3, "0")}-8f2a-${i.toString(16).padStart(12, "0")}`,
	);
}

describe("session identity", () => {
	it("gives the same session the same slug and colour every time", () => {
		const id = "0195b3c1-8a40-7001-8f2a-2d9c4e6b1a77";
		expect(sessionSlugFor(id)).toBe(sessionSlugFor(id));
		expect(sessionColorSlotFor(id)).toBe(sessionColorSlotFor(id));
	});

	it("always lands on a real colour slot", () => {
		for (const id of sameInstantIds(200)) {
			expect(isSessionColorSlot(sessionColorSlotFor(id))).toBe(true);
		}
	});

	it("reads as two hyphenated words", () => {
		for (const id of sameInstantIds(50)) {
			expect(sessionSlugFor(id)).toMatch(/^[a-z]+-[a-z]+$/);
		}
	});

	it("stays inside the chip's width cap", () => {
		for (const id of sameInstantIds(200)) {
			expect(sessionSlugFor(id).length).toBeLessThanOrEqual(20);
		}
	});

	// The whole point of hashing the full id: uuidv7 is time-ordered, so sessions
	// started moments apart share a long prefix. Those are exactly the sessions a
	// person has open side by side and most needs to tell apart, and a hash over a
	// prefix would drop them all onto one slug and one colour.
	it("spreads sessions started in the same instant across slugs and colours", () => {
		const ids = sameInstantIds(60);
		const slugs = new Set(ids.map(sessionSlugFor));
		const slots = new Set(ids.map(sessionColorSlotFor));
		expect(slugs.size).toBeGreaterThan(30);
		expect(slots.size).toBe(SESSION_COLOR_SLOTS);
	});

	it("does not tie the noun to the adjective", () => {
		// One hash for both words would give every "amber-" session the same noun.
		const ids = sameInstantIds(200).map(sessionSlugFor);
		const byAdjective = new Map<string, Set<string>>();
		for (const slug of ids) {
			const [adjective, noun] = slug.split("-");
			const nouns = byAdjective.get(adjective!) ?? new Set<string>();
			nouns.add(noun!);
			byAdjective.set(adjective!, nouns);
		}
		const largest = Math.max(...[...byAdjective.values()].map((nouns) => nouns.size));
		expect(largest).toBeGreaterThan(1);
	});

	it("rejects slots outside the palette", () => {
		expect(isSessionColorSlot(0)).toBe(false);
		expect(isSessionColorSlot(SESSION_COLOR_SLOTS + 1)).toBe(false);
		expect(isSessionColorSlot(2.5)).toBe(false);
		expect(isSessionColorSlot(Number.NaN)).toBe(false);
	});
});

describe("session colour names", () => {
	it("names every slot, and only real slots", () => {
		expect(SESSION_COLOR_NAME_LIST).toHaveLength(SESSION_COLOR_SLOTS);
		for (let slot = 1; slot <= SESSION_COLOR_SLOTS; slot++) {
			expect(sessionColorName(slot)).toBe(SESSION_COLOR_NAME_LIST[slot - 1]);
		}
		expect(sessionColorName(0)).toBeUndefined();
		expect(sessionColorName(SESSION_COLOR_SLOTS + 1)).toBeUndefined();
	});

	it("gives each slot a name of its own", () => {
		expect(new Set(SESSION_COLOR_NAME_LIST).size).toBe(SESSION_COLOR_SLOTS);
	});

	it("still takes the slot numbers", () => {
		for (let slot = 1; slot <= SESSION_COLOR_SLOTS; slot++) {
			expect(parseSessionColorSlot(String(slot))).toBe(slot);
		}
		expect(parseSessionColorSlot(" 3 ")).toBe(3);
	});

	it("takes a name, whatever its case", () => {
		for (const name of SESSION_COLOR_NAME_LIST) {
			const slot = parseSessionColorSlot(name);
			expect(slot).toBeDefined();
			expect(sessionColorName(slot!)).toBe(name);
		}
		expect(parseSessionColorSlot("GREEN")).toBe(parseSessionColorSlot("green"));
		expect(parseSessionColorSlot("  Blue ")).toBe(parseSessionColorSlot("blue"));
	});

	// The shorthands people actually type. Each name's initial has to reach its
	// own slot — two names starting with the same letter would make one of them
	// unreachable, which is the failure this guards.
	it("takes each name's initial, unambiguously", () => {
		for (const name of SESSION_COLOR_NAME_LIST) {
			expect(parseSessionColorSlot(name[0]!)).toBe(parseSessionColorSlot(name));
		}
		expect(parseSessionColorSlot("g")).toBe(parseSessionColorSlot("green"));
		expect(parseSessionColorSlot("b")).toBe(parseSessionColorSlot("blue"));
		expect(parseSessionColorSlot("y")).toBe(parseSessionColorSlot("yellow"));
	});

	// The palette has no red of its own, so `red` lands on the nearest slot
	// rather than being rejected — refusing it would be pedantry.
	it("resolves the hues between slots to the nearest one", () => {
		expect(parseSessionColorSlot("red")).toBe(parseSessionColorSlot("magenta"));
		expect(parseSessionColorSlot("r")).toBe(parseSessionColorSlot("magenta"));
		expect(parseSessionColorSlot("orange")).toBe(parseSessionColorSlot("yellow"));
		expect(parseSessionColorSlot("teal")).toBe(parseSessionColorSlot("cyan"));
		expect(parseSessionColorSlot("violet")).toBe(parseSessionColorSlot("purple"));
	});

	it("always lands on a slot the session can actually be set to", () => {
		const spellings = [...SESSION_COLOR_NAME_LIST, "red", "r", "g", "b", "y", "m", "orange", "pink", "1", "6"];
		for (const spelling of spellings) {
			expect(isSessionColorSlot(parseSessionColorSlot(spelling)!)).toBe(true);
		}
	});

	it("names nothing for input that names no slot", () => {
		for (const junk of ["", "   ", "0", String(SESSION_COLOR_SLOTS + 1), "2.5", "-1", "chartreuse", "z"]) {
			expect(parseSessionColorSlot(junk)).toBeUndefined();
		}
	});

	// The keyboard cycle. Stepping is what tells two terminals apart, so it has to
	// visit every slot, come back to where it started, and never stall.
	it("steps through every slot and returns to where it started", () => {
		let slot = 1;
		const seen = new Set<number>([slot]);
		for (let i = 0; i < SESSION_COLOR_SLOTS - 1; i++) {
			slot = cycleSessionColorSlot(slot, "forward");
			expect(isSessionColorSlot(slot)).toBe(true);
			seen.add(slot);
		}
		expect(seen.size).toBe(SESSION_COLOR_SLOTS);
		expect(cycleSessionColorSlot(slot, "forward")).toBe(1);
	});

	it("undoes a forward step with a backward one", () => {
		for (let slot = 1; slot <= SESSION_COLOR_SLOTS; slot++) {
			expect(cycleSessionColorSlot(cycleSessionColorSlot(slot, "forward"), "backward")).toBe(slot);
			expect(cycleSessionColorSlot(cycleSessionColorSlot(slot, "backward"), "forward")).toBe(slot);
		}
	});

	it("wraps at both ends", () => {
		expect(cycleSessionColorSlot(SESSION_COLOR_SLOTS, "forward")).toBe(1);
		expect(cycleSessionColorSlot(1, "backward")).toBe(SESSION_COLOR_SLOTS);
	});

	// A session wearing a slot this build has no colour for — a downgrade, a
	// hand-edited session file — still has to move when the key is pressed.
	it("starts from the first slot when the current one is not a slot at all", () => {
		for (const junk of [0, -1, 2.5, SESSION_COLOR_SLOTS + 1, Number.NaN]) {
			expect(cycleSessionColorSlot(junk, "forward")).toBe(1);
			expect(cycleSessionColorSlot(junk, "backward")).toBe(1);
		}
	});
});
