import { describe, expect, it } from "vitest";
import {
	isSessionColorSlot,
	SESSION_COLOR_SLOTS,
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
