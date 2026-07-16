import { describe, expect, it } from "vitest";
import { BELL, CompletionChime } from "../src/modes/interactive/completion-chime.js";

/**
 * Drives a CompletionChime with a fake clock and a ring counter so timing-based
 * behaviour (threshold, debounce) is deterministic and no real terminal is touched.
 */
function createHarness(opts: { enabled?: boolean } = {}) {
	let clock = 0;
	let enabled = opts.enabled ?? true;
	let rings = 0;
	const chime = new CompletionChime({
		isEnabled: () => enabled,
		ring: () => {
			rings++;
		},
		thresholdMs: 10_000,
		debounceMs: 5_000,
		now: () => clock,
	});
	return {
		chime,
		advance: (ms: number) => {
			clock += ms;
		},
		setEnabled: (value: boolean) => {
			enabled = value;
		},
		get rings() {
			return rings;
		},
	};
}

describe("CompletionChime", () => {
	it("exposes the terminal bell byte as the cue", () => {
		expect(BELL).toBe("\x07");
	});

	it("fires when a turn runs longer than the threshold", () => {
		const h = createHarness();
		h.chime.onTurnStart();
		h.advance(11_000);
		h.chime.onTurnComplete({ aborted: false });
		expect(h.rings).toBe(1);
	});

	it("does not fire for a turn shorter than the threshold", () => {
		const h = createHarness();
		h.chime.onTurnStart();
		h.advance(5_000);
		h.chime.onTurnComplete({ aborted: false });
		expect(h.rings).toBe(0);
	});

	it("does not fire when disabled, even for a long turn", () => {
		const h = createHarness({ enabled: false });
		h.chime.onTurnStart();
		h.advance(30_000);
		h.chime.onTurnComplete({ aborted: false });
		expect(h.rings).toBe(0);
	});

	it("reflects a live enable toggle (read fresh on each ring)", () => {
		const h = createHarness({ enabled: false });
		h.chime.onTurnStart();
		h.advance(11_000);
		h.chime.onTurnComplete({ aborted: false });
		expect(h.rings).toBe(0);

		h.setEnabled(true);
		h.chime.onTurnStart();
		h.advance(11_000);
		h.chime.onTurnComplete({ aborted: false });
		expect(h.rings).toBe(1);
	});

	it("stays silent when the turn was user-aborted", () => {
		const h = createHarness();
		h.chime.onTurnStart();
		h.advance(30_000);
		h.chime.onTurnComplete({ aborted: true });
		expect(h.rings).toBe(0);
	});

	it("fires immediately when blocked for input, bypassing the threshold", () => {
		const h = createHarness();
		// No turn duration accrued at all — the blocked cue does not wait for 10s.
		h.chime.onBlockedForInput();
		expect(h.rings).toBe(1);
	});

	it("debounces rapid chimes", () => {
		const h = createHarness();
		h.chime.onBlockedForInput();
		expect(h.rings).toBe(1);

		// Within the 5s debounce window → suppressed.
		h.advance(4_000);
		h.chime.onBlockedForInput();
		expect(h.rings).toBe(1);

		// Past the debounce window → allowed again.
		h.advance(2_000);
		h.chime.onBlockedForInput();
		expect(h.rings).toBe(2);
	});

	it("shares one debounce window across the complete and blocked paths", () => {
		const h = createHarness();
		h.chime.onTurnStart();
		h.advance(11_000);
		h.chime.onTurnComplete({ aborted: false });
		expect(h.rings).toBe(1);

		// A blocked cue moments later is coalesced with the completion chime.
		h.advance(2_000);
		h.chime.onBlockedForInput();
		expect(h.rings).toBe(1);
	});

	it("measures duration from the first start, so a retried turn still fires", () => {
		const h = createHarness();
		h.chime.onTurnStart(); // t=0, real start of the turn
		h.advance(6_000);
		h.chime.onTurnStart(); // t=6000, retry re-enters agent_start — must not reset the anchor
		h.advance(6_000); // t=12000
		h.chime.onTurnComplete({ aborted: false });
		// Elapsed from the original start is 12s (> 10s). Had the retry reset the
		// anchor, only 6s would have accrued and nothing would ring.
		expect(h.rings).toBe(1);
	});

	it("re-anchors for the next turn after one completes", () => {
		const h = createHarness();
		h.chime.onTurnStart();
		h.advance(11_000);
		h.chime.onTurnComplete({ aborted: false });
		expect(h.rings).toBe(1);

		// New short turn well past the debounce window: must not ring on stale timing.
		h.advance(60_000);
		h.chime.onTurnStart();
		h.advance(2_000);
		h.chime.onTurnComplete({ aborted: false });
		expect(h.rings).toBe(1);
	});
});
