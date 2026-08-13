/**
 * Per-directory memory of what `/learn` has already shown you.
 *
 * The extractor computes a sliding-window snapshot — "in your last N sessions
 * you said this M times" — which is stateless by design and correct to
 * recompute from scratch every run. What it cannot know on its own is whether
 * you have already *seen* a given item and made a call on it. Without that, a
 * rule you accepted last week comes back forever (its occurrences are still
 * inside the window), and worse, it comes back flagged `restated` — accusing a
 * rule that is working of not working. An item you declined comes back too,
 * unchanged, every single run.
 *
 * So one bookmark per directory: what was surfaced, and when. An item is shown
 * again only once something new has happened — you said it again *after* it was
 * last put in front of you. That single rule fixes both cases, and it keeps the
 * `restated` label honest, because the only way to earn it is to repeat yourself
 * after the rule already existed.
 *
 * This is a bookmark, not an index. No cards, no retrieval, no embeddings — the
 * transcripts remain the source of truth and this file can be deleted at any
 * time with no loss beyond re-proposing things once.
 */

import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { writeFileAtomicSync } from "../../utils/atomic-file.js";

/** Bump when the on-disk shape changes; an unknown version is discarded, not migrated. */
const STATE_VERSION = 1;

/** Surfaced keys older than this are forgotten, so the file cannot grow without bound. */
const STATE_RETENTION_DAYS = 180;

export interface SurfacedItem {
	/** When this item was last put in front of the user, ISO. */
	surfacedAt: string;
	/** Newest occurrence the run knew about at that point, ISO. */
	lastOccurrence: string;
	/**
	 * Whether the item was covered by a context file the last time it was
	 * surfaced. Comparing that against coverage now is how an adopted proposal is
	 * told from a declined one, without asking.
	 */
	coveredWhenSurfaced: boolean;
}

export interface LearnState {
	version: number;
	lastRun?: string;
	surfaced: Record<string, SurfacedItem>;
}

function emptyState(): LearnState {
	return { version: STATE_VERSION, surfaced: {} };
}

/**
 * State file for a directory.
 *
 * Keyed off the *name* of the cwd's session directory, which encodes the cwd —
 * so the bookmark is per working directory, matching what the digest describes.
 * Callers pass the cwd-derived path rather than whatever directory the live
 * session happens to be reading: a session with no directory of its own would
 * otherwise land every project on one nameless state file, and projects sharing
 * a custom `sessionDir` would share one bookmark between them.
 */
export function getLearnStatePath(agentDir: string, sessionDir: string): string {
	return join(agentDir, "learn", `${basename(sessionDir)}.json`);
}

/** Read state, treating any unreadable or unknown-version file as empty. */
export function readLearnState(path: string): LearnState {
	try {
		if (!existsSync(path)) return emptyState();
		const parsed = JSON.parse(readFileSync(path, "utf-8")) as Partial<LearnState>;
		if (parsed.version !== STATE_VERSION || typeof parsed.surfaced !== "object" || parsed.surfaced === null) {
			return emptyState();
		}
		return { version: STATE_VERSION, lastRun: parsed.lastRun, surfaced: parsed.surfaced };
	} catch {
		// A corrupt bookmark is not worth failing a command over; the cost of
		// starting fresh is proposing a few things a second time.
		return emptyState();
	}
}

/** Persist state, pruning entries nothing has referenced in a long time. */
export function writeLearnState(path: string, state: LearnState, now: Date = new Date()): void {
	const cutoff = now.getTime() - STATE_RETENTION_DAYS * 24 * 60 * 60 * 1000;
	const surfaced: Record<string, SurfacedItem> = {};
	for (const [key, item] of Object.entries(state.surfaced)) {
		const at = Date.parse(item.surfacedAt);
		if (Number.isNaN(at) || at >= cutoff) surfaced[key] = item;
	}

	try {
		mkdirSync(join(path, ".."), { recursive: true });
		writeFileAtomicSync(path, `${JSON.stringify({ ...state, surfaced }, null, 2)}\n`);
	} catch {
		// Losing the bookmark degrades /learn to its previous behaviour (proposing
		// already-decided items again). Never worth failing the command.
	}
}

/** What the caller needs to decide whether an item is worth showing again. */
export interface SuppressionInput {
	key: string;
	/** Newest occurrence of this item in the current window, ISO. */
	lastSeen: string;
	/** Whether a context file covers it right now. */
	covered: boolean;
}

export interface SuppressionVerdict {
	/** Drop it: already surfaced, and nothing new has happened since. */
	suppressed: boolean;
	/**
	 * Surfaced before, still not covered by any context file — you saw it and
	 * chose not to write it down. Worth telling the model so it proposes again
	 * tentatively instead of pressing the same case twice.
	 */
	previouslyDeclined: boolean;
}

/**
 * Decide whether an item should be shown again.
 *
 * An item is suppressed when it was surfaced before and has not recurred since:
 * `lastSeen <= surfacedAt` means every occurrence backing it was already on
 * screen when you made your call. Say it again and `lastSeen` moves past
 * `surfacedAt`, and it returns — which is exactly when it is worth returning.
 */
export function judge(state: LearnState, input: SuppressionInput): SuppressionVerdict {
	const previous = state.surfaced[input.key];
	if (!previous) return { suppressed: false, previouslyDeclined: false };

	const surfacedAt = Date.parse(previous.surfacedAt);
	const lastSeen = Date.parse(input.lastSeen);
	const recurredSince = !Number.isNaN(surfacedAt) && !Number.isNaN(lastSeen) && lastSeen > surfacedAt;

	return {
		suppressed: !recurredSince,
		// Covered now but not when it was surfaced means the proposal was taken up.
		// Still uncovered means it was passed over.
		previouslyDeclined: !input.covered && !previous.coveredWhenSurfaced,
	};
}

export interface LearnStats {
	/** Proposals on record, across all categories. */
	total: number;
	directives: number;
	fixes: number;
	workflows: number;
	/**
	 * Directive proposals that were genuinely open questions — not already
	 * written down when they were shown. Only these can be adopted or passed
	 * over, so they are the denominator of the rate.
	 */
	open: number;
	adopted: number;
	declined: number;
	earliest?: string;
	latest?: string;
	lastRun?: string;
}

/**
 * Reconstruct what happened to past proposals.
 *
 * Nothing extra is recorded to make this work: each entry already stores whether
 * it was covered when it was shown, and coverage now is recomputable, so the
 * pair is enough to tell an adopted proposal from one that was passed over.
 *
 * Directives only. Fixes and workflows have no coverage signal — a fix may have
 * become a rule, a skill, or a habit, and which one is not recoverable — so
 * counting them here would mean inventing an outcome.
 */
export function summarizeLearnState(state: LearnState, isCoveredNow: (normalized: string) => boolean): LearnStats {
	const stats: LearnStats = { total: 0, directives: 0, fixes: 0, workflows: 0, open: 0, adopted: 0, declined: 0 };

	for (const [key, item] of Object.entries(state.surfaced)) {
		stats.total++;
		if (!stats.earliest || item.surfacedAt < stats.earliest) stats.earliest = item.surfacedAt;
		if (!stats.latest || item.surfacedAt > stats.latest) stats.latest = item.surfacedAt;

		if (key.startsWith("fix:")) {
			stats.fixes++;
			continue;
		}
		if (key.startsWith("workflow:")) {
			stats.workflows++;
			continue;
		}
		if (!key.startsWith("directive:")) continue;

		stats.directives++;
		// Already written down when shown, so there was no decision to make.
		if (item.coveredWhenSurfaced) continue;

		stats.open++;
		if (isCoveredNow(key.slice("directive:".length))) stats.adopted++;
		else stats.declined++;
	}

	stats.lastRun = state.lastRun;
	return stats;
}

/** Record everything this run put on screen, so the next run can suppress it. */
export function recordSurfaced(
	state: LearnState,
	items: Array<{ key: string; lastSeen: string; covered: boolean }>,
	now: Date = new Date(),
): LearnState {
	const surfaced = { ...state.surfaced };
	const at = now.toISOString();
	for (const item of items) {
		surfaced[item.key] = { surfacedAt: at, lastOccurrence: item.lastSeen, coveredWhenSurfaced: item.covered };
	}
	return { version: STATE_VERSION, lastRun: at, surfaced };
}
