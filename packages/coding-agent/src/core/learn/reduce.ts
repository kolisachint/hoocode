/**
 * The reduce half of `/learn`: turn per-session observations into counts.
 *
 * The split with `mine.ts` is the whole design. Deciding that "we're on bun
 * now" and "stop using npm" mean the same thing is semantics, and the model is
 * better at it than any normalizer — so the model does it, by emitting a shared
 * `label`. Deciding that the shared label occurred nine times across five
 * sessions is arithmetic, and arithmetic stays here, because a model asked to
 * count over a long context will be approximately right, and an approximately
 * right number is worse than none when the number is the thing the reader acts
 * on.
 *
 * Nothing in this file filters on content. The only gate is the repeat
 * threshold, which is a dial the user sets and the digest reports, not a
 * whitelist they cannot see.
 */

import type { LabelledCandidate, MinedCandidate } from "./mine.js";

/** Where a repeated directive already lives, if anywhere. */
export type DirectiveStatus = "new" | "restated" | "has-skill";

/** Fields every proposable item shares, so suppression can be applied uniformly. */
export interface Proposable {
	/** Stable identity across runs — what the state file remembers. */
	key: string;
	/** Newest occurrence in the window, ISO. */
	lastSeen: string;
}

export interface DirectiveCluster extends Proposable {
	/** The model's canonical name for what was meant. The clustering key. */
	label: string;
	/** Representative verbatim quote, the longest seen in the cluster. */
	text: string;
	/** Why it is durable, in the model's words. */
	rationale?: string;
	/** Total times said. */
	count: number;
	/** Distinct sessions it was said in — the stronger of the two counts. */
	sessions: number;
	status: DirectiveStatus;
	/** The existing rule line matched, when status is `restated`. */
	existingRule?: string;
	/** The skill that already covers this, when status is `has-skill`. */
	existingSkill?: string;
	/**
	 * Shown before and still not written down anywhere — neither as a rule nor as
	 * a skill — so the reader saw this proposal and passed on it.
	 */
	previouslyDeclined: boolean;
}

export interface FixCandidate extends Proposable {
	label: string;
	/** The failing command. */
	command: string;
	/** Short excerpt of the real error text. */
	errorExcerpt: string;
	/** Commands run between the failure and the pass. */
	interveningCommands: string[];
	/** Files edited between the failure and the pass. */
	editedFiles: string[];
	count: number;
	sessions: number;
}

/**
 * A piece of work the user keeps asking for by name.
 *
 * This is the slash-command signal, and it was being thrown away: the miner
 * used to be told not to report task requests at all. But a request repeated
 * across sessions is exactly what a slash command is for, and the evidence for
 * it is stronger than for most rules — the same job, asked for again, in
 * someone's own words.
 */
export interface RequestCandidate extends Proposable {
	label: string;
	/** How the user phrased it, so the proposal can quote rather than invent. */
	text: string;
	count: number;
	sessions: number;
}

/** One session's mining output, named and tagged with the identity the counts need. */
export interface MinedSession {
	sessionId: string;
	/** When the session was last written to — the clock suppression runs on. */
	lastActivity: string;
	/** Named by the global clustering pass; grouping here is exact-label arithmetic. */
	candidates: LabelledCandidate[];
}

interface Acc {
	label: string;
	text: string;
	rationale?: string;
	count: number;
	sessions: Set<string>;
	lastSeen: string;
	samples: MinedCandidate[];
}

/**
 * Group every candidate of one kind by its label.
 *
 * `lastSeen` takes the session's last activity rather than anything the model
 * reports: the model is reading a transcript and has no reliable clock, and
 * `lastSeen` drives suppression, where a wrong value silently hides a live
 * signal or resurfaces a dead one. It is deliberately not the session's start
 * either — a long-running session would date today's words to the day it was
 * opened, and suppression would call them old news.
 */
function groupByLabel(sessions: MinedSession[], kind: MinedCandidate["kind"]): Acc[] {
	const acc = new Map<string, Acc>();

	for (const session of sessions) {
		for (const candidate of session.candidates) {
			if (candidate.kind !== kind) continue;
			const existing = acc.get(candidate.label);
			if (existing) {
				existing.count++;
				existing.sessions.add(session.sessionId);
				if (session.lastActivity > existing.lastSeen) existing.lastSeen = session.lastActivity;
				// Keep the fullest quote: a longer one carries more of the reasoning.
				if (candidate.text.length > existing.text.length) existing.text = candidate.text;
				existing.rationale ??= candidate.rationale;
				existing.samples.push(candidate);
			} else {
				acc.set(candidate.label, {
					label: candidate.label,
					text: candidate.text,
					rationale: candidate.rationale,
					count: 1,
					sessions: new Set([session.sessionId]),
					lastSeen: session.lastActivity,
					samples: [candidate],
				});
			}
		}
	}

	return [...acc.values()];
}

/**
 * Merge accumulators whose representative quote is the same sentence.
 *
 * The model labels each session independently and has no way to see what it
 * called the same thing last time, so identical text routinely arrives under
 * two labels — and then reads as two separate proposals, the same words twice.
 * Matching on the quote costs nothing and is the one case where sameness is not
 * a judgement call.
 *
 * This runs before the repeat threshold on purpose: two occurrences split
 * across two labels are each below the bar, and merging them afterwards would
 * mean neither was ever considered.
 *
 * The surviving label is the lexicographically smallest of the merged set, not
 * the first one seen. First-seen follows session order, which changes whenever
 * a session is added — so the merged cluster would silently change its name
 * between runs, the state key with it, and every item the reader had already
 * decided on would come back as new. A merge has to be a pure function of what
 * was merged.
 */
function mergeIdenticalText(entries: Acc[]): Acc[] {
	const byText = new Map<string, Acc>();
	const order: Acc[] = [];

	for (const entry of entries) {
		const key = entry.text.replace(/\s+/g, " ").trim().toLowerCase();
		const existing = byText.get(key);
		if (!existing) {
			byText.set(key, entry);
			order.push(entry);
			continue;
		}
		existing.count += entry.count;
		for (const session of entry.sessions) existing.sessions.add(session);
		if (entry.lastSeen > existing.lastSeen) existing.lastSeen = entry.lastSeen;
		existing.rationale ??= entry.rationale;
		existing.samples.push(...entry.samples);
		if (entry.label < existing.label) existing.label = entry.label;
	}

	return order;
}

/** Distinct sessions first, then raw count: five sessions beats nine times in one. */
function byEvidence(a: { sessions: number; count: number; label: string }, b: typeof a): number {
	return b.sessions - a.sessions || b.count - a.count || a.label.localeCompare(b.label);
}

/** Merge repeated string fields across a cluster's samples, preserving order and dropping dupes. */
function mergeStrings(samples: MinedCandidate[], pick: (c: MinedCandidate) => string[] | undefined): string[] {
	const out: string[] = [];
	const seen = new Set<string>();
	for (const sample of samples) {
		for (const value of pick(sample) ?? []) {
			if (seen.has(value)) continue;
			seen.add(value);
			out.push(value);
		}
	}
	return out.slice(0, 12);
}

/**
 * The repeat threshold counts distinct sessions, not occurrences.
 *
 * Saying a thing twice inside one session is the commonest thing in a
 * transcript and usually means the opposite of durable: the agent ignored it
 * the first time, so it was restated. Two *sessions* is a claim about how you
 * work; two lines in one session is a claim about one afternoon.
 */
function repeatedEnough(entry: Acc, minRepeats: number): boolean {
	return entry.sessions.size >= minRepeats;
}

export function reduceDirectives(sessions: MinedSession[], minRepeats: number): DirectiveCluster[] {
	return mergeIdenticalText(groupByLabel(sessions, "directive"))
		.filter((entry) => repeatedEnough(entry, minRepeats))
		.map((entry) => ({
			key: `directive:${entry.label}`,
			label: entry.label,
			text: entry.text,
			rationale: entry.rationale,
			count: entry.count,
			sessions: entry.sessions.size,
			lastSeen: entry.lastSeen,
			// Coverage is decided later, by a model that can tell a paraphrase from a
			// coincidence. Everything starts `new` and is corrected in place.
			status: "new" as DirectiveStatus,
			previouslyDeclined: false,
		}))
		.sort(byEvidence);
}

export function reduceFixes(sessions: MinedSession[], minRepeats: number): FixCandidate[] {
	return groupByLabel(sessions, "fix")
		.filter((entry) => repeatedEnough(entry, minRepeats))
		.map((entry) => ({
			key: `fix:${entry.label}`,
			label: entry.label,
			command: entry.samples.find((s) => s.command)?.command ?? entry.text,
			errorExcerpt: entry.samples.find((s) => s.errorExcerpt)?.errorExcerpt ?? "",
			interveningCommands: mergeStrings(entry.samples, (s) => s.interveningCommands),
			editedFiles: mergeStrings(entry.samples, (s) => s.editedFiles),
			count: entry.count,
			sessions: entry.sessions.size,
			lastSeen: entry.lastSeen,
		}))
		.sort(byEvidence);
}

export function reduceRequests(sessions: MinedSession[], minRepeats: number): RequestCandidate[] {
	return mergeIdenticalText(groupByLabel(sessions, "request"))
		.filter((entry) => repeatedEnough(entry, minRepeats))
		.map((entry) => ({
			key: `request:${entry.label}`,
			label: entry.label,
			text: entry.text,
			count: entry.count,
			sessions: entry.sessions.size,
			lastSeen: entry.lastSeen,
		}))
		.sort(byEvidence);
}
