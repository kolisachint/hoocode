/**
 * Scoring harness for the `/learn` naming pass.
 *
 * Lives outside the test file so it can be pointed at a real model. The suite
 * runs it with a deterministic reference clusterer to lock the pipeline's
 * behaviour; scoring an actual model is the same call with
 * `createLlmClusterer(deps)` in its place, which is the only way to know whether
 * the assumption the design rests on currently holds.
 *
 * The corpus is hand-grouped: phrasings inside a group mean one thing and must
 * merge, groups must stay apart, and the decoy pair shares a sentence shape
 * while meaning different things — the failure mode a clusterer that matches on
 * wording falls into.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { Clusterer, ClusterInput } from "../../src/core/learn/cluster.js";

export interface EvalGroup {
	label: string;
	phrasings: string[];
}

export interface EvalCorpus {
	groups: EvalGroup[];
	decoys: EvalGroup[];
	unverifiable: string[];
	replayed: string[];
}

const here = dirname(fileURLToPath(import.meta.url));

export const evalCorpus: EvalCorpus = JSON.parse(
	readFileSync(join(here, "..", "fixtures", "learn-eval.json"), "utf-8"),
);

export interface ClusteringScore {
	candidates: number;
	clusters: number;
	/** clusters / candidates. 1.0 means nothing merged; the target is groups / candidates. */
	ratio: number;
	/** Groups whose every phrasing landed on one label. */
	merged: number;
	/** Groups that ended up sharing a label with another group. */
	collided: number;
	total: number;
}

/** Run a clusterer over the corpus and score it against the hand-grouping. */
export async function evaluateClustering(clusterer: Clusterer, knownLabels: string[] = []): Promise<ClusteringScore> {
	const groups = [...evalCorpus.groups, ...evalCorpus.decoys];
	const inputs: ClusterInput[] = [];
	const owner: number[] = [];
	for (const [index, group] of groups.entries()) {
		for (const text of group.phrasings) {
			inputs.push({ id: inputs.length, kind: "directive", text });
			owner.push(index);
		}
	}

	const labels = await clusterer(inputs, knownLabels);
	const assigned = inputs.map((input) => labels.get(input.id) ?? `unnamed-${input.id}`);

	const perGroup = groups.map(() => new Set<string>());
	for (const [index, label] of assigned.entries()) perGroup[owner[index] ?? 0]?.add(label);

	const merged = perGroup.filter((set) => set.size === 1).length;
	let collided = 0;
	for (const [index, set] of perGroup.entries()) {
		const others = perGroup.filter((_, other) => other !== index).flatMap((other) => [...other]);
		if ([...set].some((label) => others.includes(label))) collided++;
	}

	const clusters = new Set(assigned).size;
	return {
		candidates: inputs.length,
		clusters,
		ratio: clusters / inputs.length,
		merged,
		collided,
		total: groups.length,
	};
}

/**
 * A clusterer that gets the answer right by construction.
 *
 * Stands in for a competent model so the assertions measure the *pipeline*.
 */
export const referenceClusterer: Clusterer = async (inputs) => {
	const out = new Map<number, string>();
	for (const input of inputs) {
		const group = [...evalCorpus.groups, ...evalCorpus.decoys].find((candidate) =>
			candidate.phrasings.includes(input.text),
		);
		out.set(input.id, group?.label ?? "unmatched");
	}
	return out;
};

/** Names each item after its own words — the behaviour that shipped, as a control. */
export const perTextClusterer: Clusterer = async (inputs) =>
	new Map(inputs.map((input) => [input.id, input.text.toLowerCase().replace(/[^a-z0-9]+/g, "-")]));
