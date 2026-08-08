/**
 * Paired comparison between two configs in one run record.
 *
 * Aggregate means are the wrong instrument for "should we change this
 * default". Both configs score the *same* queries, so the informative
 * question is per-query: how many did B improve, how many did it hurt, and
 * could that split have come from chance? A 3pp mean gap can be two queries
 * out of sixty moving, which is not a reason to change anything.
 *
 * Reports a two-sided sign test over the queries where the two configs
 * differ. Ties carry no information about direction and are excluded, which
 * is what makes the test paired.
 */

import type { EvalQueryResult } from "./eval.js";

/** Per-query metrics that can be compared. */
export type ComparableMetric = "recallAt1" | "recallAt5" | "recallAt10" | "recallAt50" | "mrr";

export interface PairedQueryDelta {
	id: string;
	class: string;
	before: number;
	after: number;
}

export interface PairedComparison {
	metric: ComparableMetric;
	before: string;
	after: string;
	better: number;
	worse: number;
	tied: number;
	meanBefore: number;
	meanAfter: number;
	/** Two-sided sign-test p-value over non-tied queries. 1 when all tie. */
	pValue: number;
	deltas: PairedQueryDelta[];
}

export interface ComparableRun {
	perQuery: Array<{ id: string; class: string; results: EvalQueryResult[] }>;
}

function binomial(n: number, k: number): number {
	let result = 1;
	for (let i = 0; i < k; i++) result = (result * (n - i)) / (i + 1);
	return result;
}

/** Two-sided sign test: probability of a split at least this lopsided under
 *  a fair coin. */
export function signTestPValue(better: number, worse: number): number {
	const n = better + worse;
	if (n === 0) return 1;
	const tail = Math.min(better, worse);
	let cumulative = 0;
	for (let i = 0; i <= tail; i++) cumulative += binomial(n, i);
	return Math.min(1, (cumulative / 2 ** n) * 2);
}

/**
 * Pair the *same* config across two run records, so "did my change help?" is
 * answerable at all. `comparePaired` compares two configs inside one run;
 * this compares one config before and after a retrieval change.
 *
 * Only sound when both runs scored the same corpus and the same gold set —
 * otherwise the delta includes corpus drift. The caller checks provenance.
 */
export function mergeRunsForComparison(before: ComparableRun, after: ComparableRun, label: string): ComparableRun {
	const afterById = new Map(after.perQuery.map((q) => [q.id, q]));
	const perQuery: ComparableRun["perQuery"] = [];
	for (const q of before.perQuery) {
		const other = afterById.get(q.id);
		const a = q.results.find((r) => r.label === label);
		const b = other?.results.find((r) => r.label === label);
		if (!a || !b) continue;
		perQuery.push({
			id: q.id,
			class: q.class,
			results: [
				{ ...a, label: "before" },
				{ ...b, label: "after" },
			],
		});
	}
	return { perQuery };
}

export function comparePaired(
	run: ComparableRun,
	before: string,
	after: string,
	metric: ComparableMetric,
): PairedComparison {
	const pick = (results: readonly EvalQueryResult[], label: string): EvalQueryResult | undefined =>
		results.find((r) => r.label === label);

	let better = 0;
	let worse = 0;
	let tied = 0;
	let sumBefore = 0;
	let sumAfter = 0;
	let n = 0;
	const deltas: PairedQueryDelta[] = [];

	for (const query of run.perQuery) {
		const a = pick(query.results, before);
		const b = pick(query.results, after);
		if (!a || !b) continue;
		n++;
		sumBefore += a[metric];
		sumAfter += b[metric];
		if (b[metric] > a[metric]) {
			better++;
			deltas.push({ id: query.id, class: query.class, before: a[metric], after: b[metric] });
		} else if (b[metric] < a[metric]) {
			worse++;
			deltas.push({ id: query.id, class: query.class, before: a[metric], after: b[metric] });
		} else {
			tied++;
		}
	}

	return {
		metric,
		before,
		after,
		better,
		worse,
		tied,
		meanBefore: n > 0 ? sumBefore / n : 0,
		meanAfter: n > 0 ? sumAfter / n : 0,
		pValue: signTestPValue(better, worse),
		deltas,
	};
}

export function formatComparison(comparison: PairedComparison): string {
	const { metric, before, after, better, worse, tied, pValue } = comparison;
	const delta = comparison.meanAfter - comparison.meanBefore;
	const verdict =
		pValue <= 0.05
			? "significant at p<=0.05"
			: `NOT significant (p=${pValue.toFixed(3)}) — mean gap is not evidence of a real difference`;
	return (
		`${metric}: "${after}" vs "${before}"\n` +
		`  mean ${comparison.meanBefore.toFixed(3)} -> ${comparison.meanAfter.toFixed(3)} ` +
		`(${delta >= 0 ? "+" : ""}${delta.toFixed(3)})\n` +
		`  ${better} better, ${worse} worse, ${tied} tied\n` +
		`  ${verdict}`
	);
}
