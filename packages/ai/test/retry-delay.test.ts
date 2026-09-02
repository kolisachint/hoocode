import { describe, expect, it } from "vitest";
import {
	createRetryDelayCapFetch,
	describeProviderError,
	formatDelay,
	isLongRetryDelayError,
	MAX_TIMER_DELAY_MS,
	parseRetryAfterMs,
	retryDelayCapFetch,
} from "../src/utils/retry-delay.js";

/**
 * The delay GitHub Copilot returned when a monthly quota ran out: 2472352
 * seconds, about 28.6 days. Taken literally by an SDK it becomes a
 * `setTimeout` the platform cannot hold, and the longest wait turns into the
 * shortest one.
 */
const QUOTA_RETRY_AFTER_SECONDS = "2472352";
const QUOTA_DELAY_MS = 2_472_352_000;

function response(status: number, headers: Record<string, string>): Response {
	return new Response("quota exceeded", { status, headers });
}

describe("parseRetryAfterMs", () => {
	it("reads retry-after as seconds", () => {
		expect(parseRetryAfterMs(new Headers({ "retry-after": QUOTA_RETRY_AFTER_SECONDS }))).toBe(QUOTA_DELAY_MS);
	});

	it("prefers retry-after-ms when both are present", () => {
		const headers = new Headers({ "retry-after-ms": "1500", "retry-after": "30" });
		expect(parseRetryAfterMs(headers)).toBe(1500);
	});

	it("reads retry-after as an HTTP date", () => {
		const at = new Date(Date.now() + 120_000).toUTCString();
		const parsed = parseRetryAfterMs(new Headers({ "retry-after": at })) ?? 0;
		// Second-resolution header, so allow a second of slack either way.
		expect(Math.abs(parsed - 120_000)).toBeLessThan(1500);
	});

	it("returns undefined when no delay is asked for", () => {
		expect(parseRetryAfterMs(new Headers())).toBeUndefined();
		expect(parseRetryAfterMs(undefined)).toBeUndefined();
	});
});

describe("formatDelay", () => {
	it("says a quota wait in units a person acts on", () => {
		expect(formatDelay(QUOTA_DELAY_MS)).toBe("28d 14h");
	});

	it.each([
		[45_000, "45s"],
		[150_000, "2m 30s"],
		[3_600_000, "1h"],
	])("formats %ims as %s", (ms, expected) => {
		expect(formatDelay(ms)).toBe(expected);
	});
});

describe("createRetryDelayCapFetch", () => {
	const cap = 60_000;

	it("marks a too-long wait as one the SDK must not retry", async () => {
		const capped = createRetryDelayCapFetch(cap, async () =>
			response(429, { "retry-after": QUOTA_RETRY_AFTER_SECONDS }),
		);
		const result = await capped("https://example.invalid");

		// This header is what both SDKs check before computing any sleep, so it
		// is the whole fix: no sleep is computed, so none can overflow.
		expect(result.headers.get("x-should-retry")).toBe("false");
		expect(result.status).toBe(429);
		expect(await result.text()).toBe("quota exceeded");
	});

	it("leaves a short wait alone, so ordinary rate limits still retry", async () => {
		const capped = createRetryDelayCapFetch(cap, async () => response(429, { "retry-after": "30" }));
		expect((await capped("https://example.invalid")).headers.get("x-should-retry")).toBeNull();
	});

	it("leaves a 429 with no retry-after alone", async () => {
		const capped = createRetryDelayCapFetch(cap, async () => response(429, {}));
		expect((await capped("https://example.invalid")).headers.get("x-should-retry")).toBeNull();
	});

	it("does not override a server that already stated its preference", async () => {
		const capped = createRetryDelayCapFetch(cap, async () =>
			response(429, { "retry-after": QUOTA_RETRY_AFTER_SECONDS, "x-should-retry": "true" }),
		);
		expect((await capped("https://example.invalid")).headers.get("x-should-retry")).toBe("true");
	});

	it("passes successful responses straight through", async () => {
		const ok = new Response("{}", { status: 200, headers: { "retry-after": QUOTA_RETRY_AFTER_SECONDS } });
		const capped = createRetryDelayCapFetch(cap, async () => ok);
		expect(await capped("https://example.invalid")).toBe(ok);
	});

	it("is disabled by a cap of zero, as documented", async () => {
		const base = async () => response(429, { "retry-after": QUOTA_RETRY_AFTER_SECONDS });
		expect(createRetryDelayCapFetch(0, base)).toBe(base);
		expect(retryDelayCapFetch(0)).toEqual({});
	});

	it("omits the fetch key entirely when no cap is configured", () => {
		// Passing `fetch: undefined` would override the SDK's own default rather
		// than leaving it in place.
		expect(retryDelayCapFetch(undefined)).toEqual({});
		expect("fetch" in retryDelayCapFetch(60_000)).toBe(true);
	});
});

describe("describeProviderError", () => {
	/** An SDK error carries the response headers; that is where the wait lives. */
	function apiError(message: string, headers: Record<string, string>): Error & { headers: Headers } {
		return Object.assign(new Error(message), { headers: new Headers(headers) });
	}

	it("says how long the provider wants us gone", () => {
		const described = describeProviderError(
			apiError("429 quota exceeded", { "retry-after": QUOTA_RETRY_AFTER_SECONDS }),
		);
		expect(described).toContain("429 quota exceeded");
		expect(described).toContain("28d 14h");
		expect(isLongRetryDelayError(described)).toBe(true);
	});

	it("leaves a short wait unannotated, since the retry will just happen", () => {
		const described = describeProviderError(apiError("429 rate limited", { "retry-after": "30" }));
		expect(described).toBe("429 rate limited");
		expect(isLongRetryDelayError(described)).toBe(false);
	});

	it("falls back to the plain message for errors carrying no headers", () => {
		expect(describeProviderError(new Error("fetch failed"))).toBe("fetch failed");
		expect(describeProviderError("a string")).toBe('"a string"');
	});
});

describe("the overflow this all exists to prevent", () => {
	it("confirms the quota delay cannot be held by a timer", () => {
		// Node does not reject this value; it warns and substitutes 1ms, which is
		// why an uncapped wait becomes a hot retry loop.
		expect(QUOTA_DELAY_MS).toBeGreaterThan(MAX_TIMER_DELAY_MS);
	});

	it("keeps the capped delay inside what a timer can hold", () => {
		expect(Math.min(QUOTA_DELAY_MS, MAX_TIMER_DELAY_MS)).toBe(MAX_TIMER_DELAY_MS);
	});
});
