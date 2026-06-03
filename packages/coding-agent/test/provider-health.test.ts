import { afterEach, describe, expect, it } from "vitest";
import {
	clearProviderExhaustion,
	getProviderExhaustion,
	isProviderQuotaError,
	markProviderExhausted,
	resetProviderHealthForTesting,
} from "../src/core/provider-health.js";

afterEach(() => {
	resetProviderHealthForTesting();
});

describe("provider-health", () => {
	it("records and returns an active exhaustion signal", () => {
		markProviderExhausted("anthropic", "Usage limit reached");
		const record = getProviderExhaustion("anthropic");
		expect(record).toBeDefined();
		expect(record?.provider).toBe("anthropic");
		expect(record?.message).toContain("Usage limit reached");
	});

	it("returns undefined for an unflagged provider", () => {
		expect(getProviderExhaustion("openai")).toBeUndefined();
	});

	it("clears a signal on demand", () => {
		markProviderExhausted("anthropic", "boom");
		clearProviderExhaustion("anthropic");
		expect(getProviderExhaustion("anthropic")).toBeUndefined();
	});

	it("expires a signal after its TTL and prunes it", () => {
		markProviderExhausted("anthropic", "boom");
		// A zero-length TTL means the record is already expired on read.
		expect(getProviderExhaustion("anthropic", 0)).toBeUndefined();
		// Pruned: even a normal read no longer finds it.
		expect(getProviderExhaustion("anthropic")).toBeUndefined();
	});

	it("truncates very long messages", () => {
		markProviderExhausted("anthropic", "x".repeat(500));
		const record = getProviderExhaustion("anthropic");
		expect(record!.message.length).toBeLessThanOrEqual(200);
		expect(record!.message.endsWith("\u2026")).toBe(true);
	});

	it("classifies quota/rate-limit errors but not transient blips", () => {
		expect(isProviderQuotaError("Usage limit reached")).toBe(true);
		expect(isProviderQuotaError("rate_limit_error: too many requests")).toBe(true);
		expect(isProviderQuotaError("HTTP 429")).toBe(true);
		expect(isProviderQuotaError("insufficient_quota")).toBe(true);
		expect(isProviderQuotaError("credit balance is too low")).toBe(true);

		expect(isProviderQuotaError("socket hang up")).toBe(false);
		expect(isProviderQuotaError("overloaded_error")).toBe(false);
		expect(isProviderQuotaError(undefined)).toBe(false);
	});
});
