/**
 * Compact token/count formatting shared by the surfaces that report numbers
 * (footer, task panel, startup resource summary).
 *
 * Numbers stay exact where they fit and degrade to one decimal of `k`/`M` so a
 * count never grows the fixed-width chrome it sits in.
 */
export function formatTokens(count: number): string {
	if (count < 1000) return count.toString();
	if (count < 10000) return `${(count / 1000).toFixed(1)}k`;
	if (count < 1000000) return `${Math.round(count / 1000)}k`;
	if (count < 10000000) return `${(count / 1000000).toFixed(1)}M`;
	return `${Math.round(count / 1000000)}M`;
}
