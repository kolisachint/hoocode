/**
 * Server-requested retry delays, and the cap that keeps them sane.
 *
 * A provider that has run out of quota answers `429` with a `Retry-After`
 * naming when the quota resets — which for a monthly plan is weeks away, not
 * seconds. The vendor SDKs take that header literally and sleep for it, and a
 * delay that large does not survive the trip: `setTimeout` holds a signed
 * 32-bit millisecond count, so anything past ~24.9 days is silently clamped to
 * 1ms. The "wait 28 days" becomes "retry immediately", the retry draws another
 * 429, and the SDK burns its whole retry budget in a few milliseconds against a
 * quota that will not move for a month.
 *
 * So a requested delay past the cap is not something to wait out — it is a
 * different kind of failure, and the caller needs to see it rather than sit in
 * a hot loop behind it.
 */

/**
 * The largest delay a timer can actually hold.
 *
 * `setTimeout` stores its delay in a signed 32-bit integer. Larger values do
 * not throw: Node warns and substitutes 1ms, turning the longest wait into the
 * shortest one.
 */
export const MAX_TIMER_DELAY_MS = 2 ** 31 - 1;

/** Longest server-requested delay worth waiting out, when the caller names none. */
export const DEFAULT_MAX_RETRY_DELAY_MS = 60_000;

/** Phrase every capped-delay message carries, so callers can recognise one. */
const LONG_DELAY_MARKER = "asked to wait";

/** Anything with a `get` — a `Headers`, or an SDK error's header bag. */
interface HeaderBag {
	get(name: string): string | null | undefined;
}

/**
 * The delay a response asks for, in milliseconds, or undefined if it asks for
 * none.
 *
 * Mirrors what the OpenAI and Anthropic SDKs do with these headers, because the
 * point is to predict the sleep they are about to take: `retry-after-ms` wins,
 * then `retry-after` as seconds, then `retry-after` as an HTTP date.
 */
export function parseRetryAfterMs(headers: HeaderBag | undefined): number | undefined {
	if (!headers) return undefined;

	const afterMs = headers.get("retry-after-ms");
	if (afterMs) {
		const ms = Number.parseFloat(afterMs);
		if (!Number.isNaN(ms)) return ms;
	}

	const after = headers.get("retry-after");
	if (!after) return undefined;

	const seconds = Number.parseFloat(after);
	if (!Number.isNaN(seconds)) return seconds * 1000;

	// The header's other legal form is an HTTP date.
	const at = Date.parse(after);
	return Number.isNaN(at) ? undefined : at - Date.now();
}

/**
 * A duration as a person would say it: the two largest units that carry
 * meaning, because "28d 15h" tells you to go do something else and
 * "2472352s" does not.
 */
export function formatDelay(ms: number): string {
	if (!Number.isFinite(ms) || ms < 0) return "an unknown time";

	const totalSeconds = Math.round(ms / 1000);
	if (totalSeconds < 60) return `${totalSeconds}s`;

	const units: Array<[number, string]> = [
		[86400, "d"],
		[3600, "h"],
		[60, "m"],
		[1, "s"],
	];

	const parts: string[] = [];
	let remaining = totalSeconds;
	for (const [size, suffix] of units) {
		const value = Math.floor(remaining / size);
		remaining -= value * size;
		if (value > 0) parts.push(`${value}${suffix}`);
		if (parts.length === 2) break;
	}
	return parts.join(" ");
}

/**
 * Wrap `fetch` so a response asking to wait longer than `maxRetryDelayMs` is
 * marked as one the SDK must not retry.
 *
 * `x-should-retry: false` is the escape hatch both the OpenAI and Anthropic
 * clients check before anything else, so stamping it is enough to stop the
 * retry loop before it computes an unholdable sleep. The response is otherwise
 * passed through untouched — same status, same body — so the error the caller
 * finally sees is still the provider's own.
 *
 * A cap of zero (or a nonsensical one) disables the wrapper entirely, which is
 * what the documented "set to 0 to disable" means.
 */
export function createRetryDelayCapFetch(maxRetryDelayMs: number, baseFetch: typeof fetch = fetch): typeof fetch {
	if (!Number.isFinite(maxRetryDelayMs) || maxRetryDelayMs <= 0) return baseFetch;

	return async (input: Parameters<typeof fetch>[0], init?: Parameters<typeof fetch>[1]): Promise<Response> => {
		const response = await baseFetch(input, init);

		// Only a failure is ever retried, and a server that already stated its
		// own preference outranks the cap.
		if (response.ok || response.headers.get("x-should-retry") !== null) return response;

		const delayMs = parseRetryAfterMs(response.headers);
		if (delayMs === undefined || delayMs <= maxRetryDelayMs) return response;

		const headers = new Headers(response.headers);
		headers.set("x-should-retry", "false");
		return new Response(response.body, {
			status: response.status,
			statusText: response.statusText,
			headers,
		});
	};
}

/**
 * The `fetch` override to spread into an SDK client's options, or nothing when
 * no cap applies.
 *
 * Spread rather than assigned so a disabled cap leaves the key absent
 * altogether: passing `fetch: undefined` explicitly would override the client's
 * own default instead of leaving it alone.
 */
export function retryDelayCapFetch(maxRetryDelayMs: number | undefined): { fetch?: typeof fetch } {
	if (maxRetryDelayMs === undefined) return {};
	const capped = createRetryDelayCapFetch(maxRetryDelayMs);
	return capped === fetch ? {} : { fetch: capped };
}

/** The header bag an SDK error carries, if it is that kind of error. */
function errorHeaders(error: unknown): HeaderBag | undefined {
	if (!error || typeof error !== "object") return undefined;
	const headers = (error as { headers?: unknown }).headers;
	if (!headers || typeof headers !== "object") return undefined;
	return typeof (headers as HeaderBag).get === "function" ? (headers as HeaderBag) : undefined;
}

/**
 * The provider's error message, plus how long it wants us gone.
 *
 * A bare "429 quota exceeded" is the same sentence whether the quota returns in
 * thirty seconds or four weeks, and those call for opposite responses from the
 * person reading it. The wait is already on the error; it just was never shown.
 */
export function describeProviderError(error: unknown, maxRetryDelayMs = DEFAULT_MAX_RETRY_DELAY_MS): string {
	const message = error instanceof Error ? error.message : JSON.stringify(error);

	const delayMs = parseRetryAfterMs(errorHeaders(error));
	if (delayMs === undefined || delayMs <= Math.max(0, maxRetryDelayMs)) return message;

	return `${message} (the provider ${LONG_DELAY_MARKER} ${formatDelay(delayMs)} before retrying, so no retry was attempted)`;
}

/**
 * Whether an error message describes a wait too long to sit through — the
 * signal that retrying is pointless rather than merely slow.
 */
export function isLongRetryDelayError(message: string | undefined): boolean {
	return message?.includes(`provider ${LONG_DELAY_MARKER}`) ?? false;
}
