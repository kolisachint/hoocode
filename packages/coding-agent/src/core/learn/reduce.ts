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

import type { MinedCandidate } from "./mine.js";

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

export interface WorkflowCandidate extends Proposable {
	label: string;
	/** Tool names in order. */
	steps: string[];
	count: number;
	sessions: number;
}

/** One session's mining output, tagged with the identity the counts need. */
export interface MinedSession {
	sessionId: string;
	timestamp: string;
	candidates: MinedCandidate[];
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
 * `lastSeen` takes the session timestamp rather than anything the model
 * reports: the model is reading a transcript and has no reliable clock, and
 * `lastSeen` drives suppression, where a wrong value silently hides a live
 * signal or resurfaces a dead one.
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
				if (session.timestamp > existing.lastSeen) existing.lastSeen = session.timestamp;
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
					lastSeen: session.timestamp,
					samples: [candidate],
				});
			}
		}
	}

	return [...acc.values()];
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

export function reduceDirectives(sessions: MinedSession[], minRepeats: number): DirectiveCluster[] {
	return groupByLabel(sessions, "directive")
		.filter((entry) => entry.count >= minRepeats)
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
		.filter((entry) => entry.count >= minRepeats)
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

export function reduceWorkflows(sessions: MinedSession[], minRepeats: number): WorkflowCandidate[] {
	return groupByLabel(sessions, "workflow")
		.filter((entry) => entry.count >= minRepeats)
		.map((entry) => ({
			key: `workflow:${entry.label}`,
			label: entry.label,
			steps: entry.samples.find((s) => s.steps && s.steps.length > 0)?.steps ?? [],
			count: entry.count,
			sessions: entry.sessions.size,
			lastSeen: entry.lastSeen,
		}))
		.sort(byEvidence);
}
