/**
 * Per-session memo of the miner's output, keyed on file content.
 *
 * This is what makes an LLM-read-everything pipeline affordable. A closed
 * session transcript never changes again, so the model needs to read it exactly
 * once in its life. Hash the bytes, keep the candidates, and a routine `/learn`
 * pays for the one or two sessions written since the last run while the other
 * eighteen come back for free.
 *
 * It also happens to be the right answer to "incremental vs. full history",
 * which an earlier design tried to solve with an mtime cursor. A cursor breaks
 * the counting: if a run only *reads* sessions newer than the cursor, a
 * directive said once today has a count of one, because the four earlier
 * occurrences were never in the scan. Caching moves the skipping to the
 * expensive step only — the reduce step still runs over every cached session
 * every time, so the cross-session counts stay exact no matter how little was
 * mined this run.
 *
 * The cache is disposable. Deleting it costs one re-mine and nothing else, so
 * every failure path here degrades to "mine it again" rather than to an error.
 */

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join } from "node:path";
import { writeFileAtomicSync } from "../../utils/atomic-file.js";
import type { MinedCandidate } from "./mine.js";

/**
 * Bump when the miner prompt, the candidate shape, or what the miner is shown
 * changes.
 *
 * The version is part of the cache key, not a field inside the entry, so a bump
 * invalidates every entry at once without a migration or a sweep — old files
 * simply stop being looked up, and the pruner reclaims them on age.
 *
 * v2: transcripts no longer carry successful tool output or replayed
 * slash-command bodies, and candidates are dropped when their quote cannot be
 * found in what the user said. Entries mined before that were read from a
 * different transcript than the one the pipeline now produces, so keeping them
 * would mean counting evidence the current rules would have rejected.
 *
 * v3: candidates no longer carry a label. Naming moved to a global pass that
 * sees the whole window, which is also what makes this file model-independent:
 * a cached label was frozen at mining time, so changing the `fast` tier forked
 * the vocabulary permanently and split every count across the seam.
 */
const CACHE_VERSION = 3;

/** Entries untouched for this long are reclaimed. */
const CACHE_RETENTION_DAYS = 180;

export interface CachedMining {
	/** Session identity, carried so a cache hit does not need the transcript reparsed. */
	sessionId: string;
	/** Session start time, ISO. */
	timestamp: string;
	candidates: MinedCandidate[];
	/** When this entry was written, ISO. */
	minedAt: string;
}

export function getLearnCacheDir(agentDir: string): string {
	return join(agentDir, "learn", "cache");
}

/**
 * Content hash of a session file.
 *
 * Content, not mtime: a resumed session gets a fresh mtime with identical
 * bytes, and a file copied between machines gets a new mtime too. Both would
 * force a needless re-mine. Content also makes the reverse mistake impossible —
 * a file whose bytes changed always misses the cache, which matters because the
 * live session is appended to between runs.
 */
export function hashSessionFile(file: string): string | undefined {
	try {
		return createHash("sha256").update(readFileSync(file)).digest("hex").slice(0, 32);
	} catch {
		return undefined;
	}
}

function entryPath(agentDir: string, hash: string): string {
	return join(getLearnCacheDir(agentDir), `v${CACHE_VERSION}-${hash}.json`);
}

/** Look up a previously mined session. Any unreadable entry reads as a miss. */
export function readCachedMining(agentDir: string, hash: string): CachedMining | undefined {
	const path = entryPath(agentDir, hash);
	try {
		if (!existsSync(path)) return undefined;
		const parsed = JSON.parse(readFileSync(path, "utf-8")) as Partial<CachedMining>;
		if (!Array.isArray(parsed.candidates) || typeof parsed.sessionId !== "string") return undefined;
		return {
			sessionId: parsed.sessionId,
			timestamp: typeof parsed.timestamp === "string" ? parsed.timestamp : new Date(0).toISOString(),
			candidates: parsed.candidates as MinedCandidate[],
			minedAt: typeof parsed.minedAt === "string" ? parsed.minedAt : new Date(0).toISOString(),
		};
	} catch {
		return undefined;
	}
}

/** Store a mined session. Failing to cache is never worth failing the run over. */
export function writeCachedMining(agentDir: string, hash: string, entry: CachedMining): void {
	try {
		mkdirSync(getLearnCacheDir(agentDir), { recursive: true });
		writeFileAtomicSync(entryPath(agentDir, hash), `${JSON.stringify(entry, null, 2)}\n`);
	} catch {
		// The cost is re-mining this session next run.
	}
}

/**
 * Drop entries nothing has referenced in a long time, so a machine that has
 * been running this for a year does not keep every session it ever saw.
 */
export function pruneLearnCache(agentDir: string, now: Date = new Date()): void {
	const dir = getLearnCacheDir(agentDir);
	if (!existsSync(dir)) return;
	const cutoff = now.getTime() - CACHE_RETENTION_DAYS * 24 * 60 * 60 * 1000;
	try {
		for (const name of readdirSync(dir)) {
			const path = join(dir, name);
			try {
				if (statSync(path).mtime.getTime() < cutoff) rmSync(path, { force: true });
			} catch {
				// Concurrent run reclaimed it first.
			}
		}
	} catch {
		// An unreadable cache directory is not an error worth surfacing.
	}
}

/**
 * Counting what a run still owes the model lives in `extract.ts:planMining`,
 * not here: the answer depends on which sessions the window actually selects,
 * and duplicating that selection is how the confirmation prompt ends up
 * quoting a number the run does not honour.
 */
