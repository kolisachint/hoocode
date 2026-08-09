/**
 * Is deferring a tool's schema actually worth it?
 *
 * §6.3 said the threshold should be measurable rather than asserted, and until
 * now it was neither. The question turns out not to be "how many tokens do the
 * schemas cost" — `--print-token-surface` already answered that — but a
 * trade-off with a term nobody had priced:
 *
 *   Deferring withholds D tokens of schema from every request. But those tokens
 *   would have sat in the *cached* prefix, so what deferral saves per request is
 *   D at the cache-read rate, not at full price — an order of magnitude less
 *   than it looks. And the moment the model resolves a deferred tool, the
 *   harness adds it to `tools`, which renders at position 0 and invalidates the
 *   whole prefix P — so the next request pays P at the cache-*write* rate
 *   instead of the cache-read rate it would otherwise have paid.
 *
 * Break-even is therefore the number of requests before the first resolve:
 *
 *   N = P × (cacheWrite − cacheRead) / (D × cacheRead)
 *
 * The counterintuitive consequence is worth stating plainly: **P grows with the
 * conversation, D does not.** Deferral gets *worse* the longer a session runs,
 * which is the opposite of the intuition that motivated it.
 *
 * See docs/plugin-system-architecture.md §6.3 and §8.6 item 6.
 */

/** Per-million-token prices, as carried on a model. */
export interface TokenPrices {
	input: number;
	cacheRead: number;
	cacheWrite: number;
}

export interface DeferralInput {
	/** Tokens of schema that would be withheld (the deferrable tools). */
	deferredTokens: number;
	/**
	 * Tokens in the cached prefix a resolve would invalidate: system prompt plus
	 * every tool schema, and in a live session the conversation too. Callers that
	 * only know the startup surface are supplying a floor — see `prefixIsFloor`.
	 */
	prefixTokens: number;
	prices: TokenPrices;
}

export type DeferralVerdict =
	| { kind: "no-cache"; reason: string }
	| { kind: "nothing-deferred"; reason: string }
	| { kind: "unpriced"; reason: string }
	| { kind: "break-even"; requestsBeforeResolve: number; savingPerRequest: number; resolvePenalty: number };

/** Dollars for `tokens` at a per-million-token price. */
function priceOf(tokens: number, perMillion: number): number {
	return (tokens / 1_000_000) * perMillion;
}

/**
 * Price the deferral trade-off for one model.
 *
 * Deliberately returns a discriminated verdict rather than a bare number: three
 * of the four outcomes mean "the break-even question does not apply here", and
 * collapsing them into a number would invite reading 0 or Infinity as an answer.
 */
export function analyzeDeferral(input: DeferralInput): DeferralVerdict {
	const { deferredTokens, prefixTokens, prices } = input;

	if (deferredTokens <= 0) {
		return { kind: "nothing-deferred", reason: "no deferrable tools in this configuration" };
	}
	if (prices.input <= 0 && prices.cacheRead <= 0 && prices.cacheWrite <= 0) {
		return { kind: "unpriced", reason: "model carries no pricing, so the trade-off cannot be costed" };
	}
	// A provider that does not price caching separately never had a cached prefix
	// to lose, so a resolve costs nothing extra and deferral is unambiguously
	// good — the saving is the full input rate on every request.
	if (prices.cacheRead <= 0 || prices.cacheWrite <= 0) {
		return {
			kind: "no-cache",
			reason: "provider does not price prompt caching, so a resolve invalidates nothing — deferral is a pure saving",
		};
	}

	const savingPerRequest = priceOf(deferredTokens, prices.cacheRead);
	const resolvePenalty = priceOf(prefixTokens, prices.cacheWrite - prices.cacheRead);

	return {
		kind: "break-even",
		requestsBeforeResolve: resolvePenalty / savingPerRequest,
		savingPerRequest,
		resolvePenalty,
	};
}

function usd(amount: number): string {
	if (amount === 0) return "$0";
	if (amount < 0.01) return `$${amount.toFixed(6)}`;
	return `$${amount.toFixed(4)}`;
}

/**
 * Render the verdict for `--print-token-surface`.
 *
 * `prefixIsFloor` marks the common case where the caller measured the startup
 * surface only. The number is then a lower bound in the direction that matters:
 * the real prefix is larger, so the real break-even is higher and deferral looks
 * worse than printed, never better.
 */
export function formatDeferral(verdict: DeferralVerdict, options: { prefixIsFloor?: boolean } = {}): string {
	switch (verdict.kind) {
		case "nothing-deferred":
		case "unpriced":
		case "no-cache":
			return `  deferral: ${verdict.reason}`;
		case "break-even": {
			const n = Math.ceil(verdict.requestsBeforeResolve);
			const lines = [
				`  deferral saves ${usd(verdict.savingPerRequest)}/request (withheld schema at the cache-read rate)`,
				`  a resolve costs ${usd(verdict.resolvePenalty)} once (whole prefix re-written, not read)`,
				`  break-even: deferral pays off only if the model waits ${n}+ request(s) before resolving`,
			];
			if (options.prefixIsFloor) {
				lines.push(
					"  (prefix measured at startup, so this is a floor — the conversation grows it,",
					"   and with it the resolve penalty, making deferral worse as a session runs on)",
				);
			}
			return lines.join("\n");
		}
	}
}
