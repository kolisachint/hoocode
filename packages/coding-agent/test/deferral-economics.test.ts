/**
 * Pricing the deferral trade-off (§6.3).
 *
 * The number these tests pin down is the one nobody had: how many requests a
 * session must run *before* the model resolves a deferred tool for deferral to
 * have been worth it. It is much larger than intuition suggests, because the
 * saving accrues at the cache-read rate while the penalty is a full prefix
 * re-write.
 */

import { describe, expect, it } from "vitest";
import { analyzeDeferral, formatDeferral, type TokenPrices } from "../src/core/capabilities/deferral.js";

/** Opus-shaped pricing: $5/Mtok in, cache read a tenth of that, write 1.25x. */
const OPUS: TokenPrices = { input: 5, cacheRead: 0.5, cacheWrite: 6.25 };

describe("analyzeDeferral", () => {
	it("computes break-even from the cache rates, not the input rate", () => {
		const verdict = analyzeDeferral({ deferredTokens: 5_000, prefixTokens: 20_000, prices: OPUS });
		expect(verdict.kind).toBe("break-even");
		if (verdict.kind !== "break-even") return;

		// saving  = 5,000 tokens @ $0.50/Mtok            = $0.0025
		// penalty = 20,000 tokens @ ($6.25 - $0.50)/Mtok = $0.115
		expect(verdict.savingPerRequest).toBeCloseTo(0.0025, 6);
		expect(verdict.resolvePenalty).toBeCloseTo(0.115, 6);
		expect(verdict.requestsBeforeResolve).toBeCloseTo(46, 5);
	});

	it("gets worse as the conversation grows, not better", () => {
		// The finding that inverts the intuition behind deferral: the penalty
		// scales with the whole prefix, which grows every turn, while the saving
		// is fixed to the withheld schemas.
		const small = analyzeDeferral({ deferredTokens: 5_000, prefixTokens: 20_000, prices: OPUS });
		const large = analyzeDeferral({ deferredTokens: 5_000, prefixTokens: 200_000, prices: OPUS });
		if (small.kind !== "break-even" || large.kind !== "break-even") throw new Error("expected break-even");
		expect(large.requestsBeforeResolve).toBeGreaterThan(small.requestsBeforeResolve);
		expect(large.requestsBeforeResolve).toBeCloseTo(460, 4);
	});

	it("improves with more deferred schema", () => {
		const thin = analyzeDeferral({ deferredTokens: 1_000, prefixTokens: 20_000, prices: OPUS });
		const fat = analyzeDeferral({ deferredTokens: 20_000, prefixTokens: 20_000, prices: OPUS });
		if (thin.kind !== "break-even" || fat.kind !== "break-even") throw new Error("expected break-even");
		expect(fat.requestsBeforeResolve).toBeLessThan(thin.requestsBeforeResolve);
	});

	it("calls deferral a pure saving when the provider does not price caching", () => {
		// No cached prefix means a resolve invalidates nothing, so the trade-off
		// the break-even math exists to weigh simply is not present.
		const verdict = analyzeDeferral({
			deferredTokens: 5_000,
			prefixTokens: 20_000,
			prices: { input: 3, cacheRead: 0, cacheWrite: 0 },
		});
		expect(verdict.kind).toBe("no-cache");
	});

	it("declines to answer rather than returning a misleading number", () => {
		// Each of these would produce 0 or Infinity from the formula; none of them
		// mean "resolve immediately" or "never resolve".
		expect(analyzeDeferral({ deferredTokens: 0, prefixTokens: 20_000, prices: OPUS }).kind).toBe("nothing-deferred");
		expect(
			analyzeDeferral({
				deferredTokens: 5_000,
				prefixTokens: 20_000,
				prices: { input: 0, cacheRead: 0, cacheWrite: 0 },
			}).kind,
		).toBe("unpriced");
	});
});

describe("formatDeferral", () => {
	it("rounds break-even up and flags a floor-only prefix", () => {
		const text = formatDeferral(analyzeDeferral({ deferredTokens: 5_000, prefixTokens: 20_000, prices: OPUS }), {
			prefixIsFloor: true,
		});
		expect(text).toContain("46+ request(s) before resolving");
		// Honesty about the measurement: startup-only prefix understates the
		// penalty, so the printed number is optimistic for deferral.
		expect(text).toContain("this is a floor");
	});

	it("states the reason when the question does not apply", () => {
		const text = formatDeferral(analyzeDeferral({ deferredTokens: 0, prefixTokens: 1, prices: OPUS }));
		expect(text).toContain("no deferrable tools");
		expect(text).not.toContain("break-even");
	});
});
