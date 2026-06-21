/**
 * Process-wide provider health signal.
 *
 * When the main session's turn fails with a usage/quota/rate-limit error that
 * does not recover (retries exhausted or disabled), the session records the
 * provider as "exhausted" for a short window. The subagent ExecuteTask tool reads this
 * to skip pointless spawns: subagents inherit the parent's provider, so they
 * would hit the same wall. The signal is cleared on the next successful response
 * and self-expires after a TTL so it never sticks longer than necessary.
 */

/** How long an exhaustion signal stays active before it self-expires. */
export const PROVIDER_EXHAUSTION_TTL_MS = 45_000;

export interface ProviderExhaustion {
	provider: string;
	/** Epoch ms when the exhaustion was recorded. */
	at: number;
	/** The provider error message that triggered it (truncated for display). */
	message: string;
}

const records = new Map<string, ProviderExhaustion>();

/**
 * Heuristic: does this provider error indicate quota/credit/rate-limit
 * exhaustion (as opposed to a transient network/overload blip)?
 */
export function isProviderQuotaError(message: string | undefined): boolean {
	if (!message) return false;
	return /usage limit|quota|rate.?limit|too many requests|429|insufficient|out of credit|credit balance|billing|payment required|402|exceeded/i.test(
		message,
	);
}

/** Record that a provider is currently exhausted/rate-limited. */
export function markProviderExhausted(provider: string, message: string): void {
	const trimmed = message.trim();
	records.set(provider, {
		provider,
		at: Date.now(),
		message: trimmed.length > 200 ? `${trimmed.slice(0, 199)}\u2026` : trimmed,
	});
}

/** Clear any exhaustion signal for a provider (e.g. after a successful response). */
export function clearProviderExhaustion(provider: string): void {
	records.delete(provider);
}

/**
 * Return the active exhaustion record for a provider, or undefined when none is
 * active or it has expired. Expired records are pruned on read.
 */
export function getProviderExhaustion(
	provider: string,
	ttlMs: number = PROVIDER_EXHAUSTION_TTL_MS,
): ProviderExhaustion | undefined {
	const record = records.get(provider);
	if (!record) return undefined;
	if (Date.now() - record.at >= ttlMs) {
		records.delete(provider);
		return undefined;
	}
	return record;
}

/** Test helper: clear all recorded exhaustion signals. */
export function resetProviderHealthForTesting(): void {
	records.clear();
}
