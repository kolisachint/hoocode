import { MAX_TIMER_DELAY_MS } from "@kolisachint/hoocode-ai";

/**
 * Sleep helper that respects abort signal.
 *
 * The delay is clamped, because `setTimeout` holds it in a signed 32-bit
 * integer and anything larger is not rejected but silently replaced with 1ms.
 * A caller that computed a long wait — a backoff, a server-requested
 * retry-after — would get the shortest possible one instead, turning a pause
 * into a hot loop at exactly the moment the pause mattered most. Waiting 24
 * days instead of 28 is wrong too, but it is wrong in the direction that does
 * no damage, and callers with a genuinely long wait should be declining to
 * wait rather than sleeping through it.
 */
export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		if (signal?.aborted) {
			reject(new Error("Aborted"));
			return;
		}

		const timeout = setTimeout(resolve, Math.min(Math.max(0, ms), MAX_TIMER_DELAY_MS));

		signal?.addEventListener("abort", () => {
			clearTimeout(timeout);
			reject(new Error("Aborted"));
		});
	});
}
