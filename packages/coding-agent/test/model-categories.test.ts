import type { Api, Model } from "@kolisachint/hoocode-ai";
import { describe, expect, it } from "vitest";
import {
	deriveDefaultModelCategories,
	isModelCategory,
	resolveModelCategory,
	resolveModelReference,
} from "../src/core/model-categories.js";
import type { Settings } from "../src/core/settings-manager.js";

/** Build a minimal available model for derivation tests. */
function model(provider: string, id: string, priceIn: number, priceOut = priceIn, contextWindow = 128_000): Model<Api> {
	return {
		id,
		name: id,
		api: "anthropic-messages" as Api,
		provider,
		baseUrl: "https://example.test",
		reasoning: false,
		input: ["text"],
		cost: { input: priceIn, output: priceOut, cacheRead: 0, cacheWrite: 0 },
		contextWindow,
		maxTokens: 8192,
	} as Model<Api>;
}

// Four models across two providers. By combined price: tiny(1) < solo(4) <
// mid(6) < big(30). `capable` is the priciest (acme/big); fast/standard come from
// everything priced at/under capable, cheapest-first: fast=tiny, standard=median.
const AVAILABLE: Model<Api>[] = [
	model("acme", "tiny", 0.5),
	model("acme", "mid", 3),
	model("acme", "big", 15),
	model("other", "solo", 2),
];

describe("model categories", () => {
	it("recognizes the three category names and nothing else", () => {
		expect(isModelCategory("fast")).toBe(true);
		expect(isModelCategory("standard")).toBe(true);
		expect(isModelCategory("capable")).toBe(true);
		expect(isModelCategory("opus")).toBe(false);
		expect(isModelCategory("anthropic/claude-haiku")).toBe(false);
	});

	it("resolves a configured category to its model id", () => {
		const settings: Settings = { modelCategories: { fast: "myprovider/tiny", capable: "myprovider/big" } };
		expect(resolveModelCategory("fast", settings)).toBe("myprovider/tiny");
		expect(resolveModelCategory("capable", settings)).toBe("myprovider/big");
	});

	it("returns undefined for an unconfigured category when no available models are supplied", () => {
		expect(resolveModelCategory("standard", { modelCategories: { fast: "x" } })).toBeUndefined();
		expect(resolveModelCategory("fast", {})).toBeUndefined();
		expect(resolveModelCategory("capable", undefined)).toBeUndefined();
	});

	it("passes non-category model references through unchanged", () => {
		expect(resolveModelReference("anthropic/claude-sonnet", {})).toBe("anthropic/claude-sonnet");
		expect(resolveModelReference("gpt-4o", undefined)).toBe("gpt-4o");
	});

	it("resolves category references via settings, undefined when unconfigured and no models", () => {
		const settings: Settings = { modelCategories: { standard: "vendor/mid" } };
		expect(resolveModelReference("standard", settings)).toBe("vendor/mid");
		expect(resolveModelReference("fast", settings)).toBeUndefined();
	});

	describe("derived defaults from available models", () => {
		it("derives a default per tier from the available set (no explicit config)", () => {
			expect(deriveDefaultModelCategories(AVAILABLE)).toEqual({
				capable: "acme/big", // most capable = highest combined price
				fast: "acme/tiny", // cheapest available
				standard: "acme/mid", // upper-median of {tiny, solo, mid, big}
			});
		});

		it("resolves an unconfigured tier to its derived default", () => {
			expect(resolveModelCategory("fast", {}, AVAILABLE)).toBe("acme/tiny");
			expect(resolveModelCategory("standard", {}, AVAILABLE)).toBe("acme/mid");
			expect(resolveModelCategory("capable", {}, AVAILABLE)).toBe("acme/big");
			expect(resolveModelReference("capable", undefined, AVAILABLE)).toBe("acme/big");
		});

		it("anchors capable to the configured default model, clamping cheaper tiers below it", () => {
			// other/solo (price 4) is the primary; only acme/tiny (price 1) is cheaper.
			const settings: Settings = { defaultProvider: "other", defaultModel: "solo" };
			// capable follows the user's primary/default model...
			expect(resolveModelCategory("capable", settings, AVAILABLE)).toBe("other/solo");
			// ...fast is still the genuinely cheapest available model at/under capable...
			expect(resolveModelCategory("fast", settings, AVAILABLE)).toBe("acme/tiny");
			// ...and standard never exceeds capable (upper-median of {tiny, solo}).
			expect(resolveModelCategory("standard", settings, AVAILABLE)).toBe("other/solo");
		});

		it("keeps tiers monotonic across providers (a single-model top provider does not collapse fast)", () => {
			// Cheap models live on a different provider than the most capable model.
			const mixed: Model<Api>[] = [
				model("oai", "mini", 0.15, 0.6),
				model("oai", "gpt", 2, 8),
				model("ant", "opus", 15, 75),
			];
			expect(deriveDefaultModelCategories(mixed)).toEqual({
				fast: "oai/mini",
				standard: "oai/gpt",
				capable: "ant/opus",
			});
		});

		it("keeps explicit config winning even when available models could derive a default", () => {
			const settings: Settings = { modelCategories: { fast: "explicit/pin" } };
			expect(resolveModelCategory("fast", settings, AVAILABLE)).toBe("explicit/pin");
			// Unconfigured tiers still derive.
			expect(resolveModelCategory("capable", settings, AVAILABLE)).toBe("acme/big");
		});

		it("returns undefined for every tier when no suitable model is available (no provider assumed)", () => {
			expect(deriveDefaultModelCategories([])).toEqual({});
			expect(resolveModelCategory("fast", {}, [])).toBeUndefined();
			expect(resolveModelCategory("standard", {}, [])).toBeUndefined();
			expect(resolveModelCategory("capable", {}, [])).toBeUndefined();
			expect(resolveModelReference("capable", {}, [])).toBeUndefined();
		});

		it("is deterministic: identical inputs (regardless of order) yield identical output", () => {
			const shuffled = [AVAILABLE[2], AVAILABLE[0], AVAILABLE[3], AVAILABLE[1]];
			expect(deriveDefaultModelCategories(shuffled)).toEqual(deriveDefaultModelCategories(AVAILABLE));
			expect(resolveModelCategory("fast", {}, shuffled)).toBe(resolveModelCategory("fast", {}, AVAILABLE));
			expect(resolveModelCategory("standard", {}, shuffled)).toBe(resolveModelCategory("standard", {}, AVAILABLE));
		});
	});
});
