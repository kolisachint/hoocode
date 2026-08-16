/**
 * The eval the label design never had.
 *
 * Every other test in this suite injects a clusterer that already agrees with
 * itself, so none of them can fail on the one assumption the pipeline rests on:
 * that two phrasings of the same point end up under one name. That assumption
 * was false in production for months — 188 distinct labels out of 191
 * candidates — and no test noticed, because the fakes handed the labels over
 * pre-agreed.
 *
 * This measures it instead. See `support/learn-eval.ts` for the harness and
 * `fixtures/learn-eval.json` for the hand-grouped corpus.
 */

import { describe, expect, it } from "vitest";
import type { Clusterer } from "../src/core/learn/cluster.js";
import { isReplayedTurn, replayFingerprints, verifyCandidates } from "../src/core/learn/mine.js";
import { evalCorpus, evaluateClustering, perTextClusterer, referenceClusterer } from "./support/learn-eval.js";

describe("clustering eval", () => {
	it("scores a clusterer that groups correctly", async () => {
		const score = await evaluateClustering(referenceClusterer);
		expect(score.merged).toBe(score.total);
		expect(score.collided).toBe(0);
		expect(score.clusters).toBe(score.total);
		// The production failure measured 0.98 here.
		expect(score.ratio).toBeLessThan(0.5);
	});

	it("catches a clusterer that merges nothing, which is the failure that shipped", async () => {
		const score = await evaluateClustering(perTextClusterer);
		expect(score.ratio).toBe(1);
		expect(score.merged).toBe(0);
	});

	it("catches a clusterer that merges too much", async () => {
		// The decoys exist for this: same shape, different subject.
		const overEager: Clusterer = async (inputs) =>
			new Map(inputs.map((input) => [input.id, /off by default|stay off/.test(input.text) ? "tools-off" : "other"]));
		expect((await evaluateClustering(overEager)).collided).toBeGreaterThan(0);
	});

	it("is offered the labels already on record", async () => {
		let offered: string[] = [];
		const spy: Clusterer = async (inputs, knownLabels) => {
			offered = knownLabels;
			return referenceClusterer(inputs, knownLabels);
		};
		await evaluateClustering(spy, ["use-bun-not-npm"]);
		expect(offered).toContain("use-bun-not-npm");
	});
});

describe("input-quality eval", () => {
	it("rejects replayed slash-command bodies, and nothing the user said", () => {
		const fingerprints = replayFingerprints([
			{
				content:
					"Open a release PR with bump type: **$1**. Identify ONLY the files you changed in this session and stage those.",
			},
		]);
		for (const text of evalCorpus.replayed) expect(isReplayedTurn(text, fingerprints)).toBe(true);
		for (const group of evalCorpus.groups) {
			for (const text of group.phrasings) expect(isReplayedTurn(text, fingerprints)).toBe(false);
		}
	});

	it("drops quotes that cannot be found in what the user said", () => {
		const spoken = evalCorpus.groups.flatMap((group) => group.phrasings).join(" ");
		const candidates = [
			...evalCorpus.groups.map((group) => ({ kind: "directive" as const, text: group.phrasings[0] ?? "" })),
			...evalCorpus.unverifiable.map((text) => ({ kind: "directive" as const, text })),
		];
		expect(verifyCandidates(candidates, spoken)).toHaveLength(evalCorpus.groups.length);
	});
});
