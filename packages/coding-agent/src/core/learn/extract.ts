/**
 * Session mining for `/learn`.
 *
 * Reads session `.jsonl` files straight off disk rather than the live context.
 * That is the whole point: the on-disk transcript is complete even when the
 * in-context one has been compacted away, and it spans every past session
 * instead of only this one. Cross-session repetition is the signal that decides
 * whether something is a durable rule or a one-off, and it is the one thing a
 * prompt reading its own context cannot see.
 *
 * This module is the orchestrator, and the split of labour inside it is
 * deliberate:
 *
 * - **Gathering** is deterministic. Finding session files, resolving which cwd
 *   they belong to, walking the active branch of a forked session — all exact,
 *   all cheap, all here.
 * - **Judgement** is the model's, in `mine.ts` and `coverage.ts`. What counts as
 *   a directive, what two phrasings have in common, whether a rule already
 *   covers something — none of that survives contact with a regex, and it used
 *   to be decided by one.
 * - **Counting** is deterministic again, in `reduce.ts`. The number is the
 *   product, and a model asked to count over a long context will be
 *   approximately right.
 *
 * The expensive step is memoized per session file (`cache.ts`), so a session is
 * read by the model exactly once in its life and the counts are still computed
 * over every session in the window on every run.
 */

import { existsSync, readdirSync, readFileSync, realpathSync, statSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import { getUserAgentsDir } from "../../config.js";
import { getSessionDirPath } from "../session-manager.js";
import { loadSkills } from "../skills.js";
import { hashSessionFile, pruneLearnCache, readCachedMining, writeCachedMining } from "./cache.js";
import type { CoverageIndex, CoverageJudge, CoverageQuery } from "./coverage.js";
import { noCoverageJudge } from "./coverage.js";
import type { MinableSession, Miner } from "./mine.js";
import { LEARN_DIGEST_MARKER } from "./mine.js";
import type { DirectiveCluster, FixCandidate, MinedSession, Proposable, WorkflowCandidate } from "./reduce.js";
import { reduceDirectives, reduceFixes, reduceWorkflows } from "./reduce.js";
import { judge, type LearnState } from "./state.js";

export type { CoverageIndex, CoverageMatch } from "./coverage.js";
export { LEARN_DIGEST_MARKER } from "./mine.js";
export type { DirectiveCluster, DirectiveStatus, FixCandidate, WorkflowCandidate } from "./reduce.js";

/** Sessions considered, newest first. */
const DEFAULT_MAX_SESSIONS = 20;
/** Sessions older than this are ignored — a pattern that stopped is not a rule. */
const DEFAULT_MAX_AGE_DAYS = 30;
/** Entries parsed per session file, as a guard against pathological transcripts. */
const MAX_ENTRIES_PER_SESSION = 8000;
/** Occurrences a directive needs before it is proposed. */
const DEFAULT_MIN_DIRECTIVE_COUNT = 2;
/** Repeats before a tool sequence is worth proposing as a skill. */
const DEFAULT_MIN_WORKFLOW_COUNT = 3;
/** Cap on each list in the digest, so the model's budget goes to the top signals. */
const DEFAULT_MAX_PER_CATEGORY = 8;

/**
 * Why a session file on disk did not make it into the digest.
 *
 * "No recent sessions" is the one outcome a user cannot act on without this:
 * an empty session directory, a directory full of month-old sessions, and a
 * directory full of sessions belonging to another checkout all produce the same
 * sentence, and the fix differs in each case.
 */
export interface SessionScanReport {
	/** Directories actually searched, in order. */
	dirs: string[];
	/** Directories that do not exist on disk. */
	missingDirs: string[];
	/** `.jsonl` files found across all searched directories. */
	files: number;
	/** Skipped for being older than the age window. */
	tooOld: number;
	/** Skipped because the session header records a different working directory. */
	otherCwd: number;
	/** Skipped for being beyond `maxSessions`. */
	overLimit: number;
	/** Skipped for being unreadable, unparseable, or empty. */
	unreadable: number;
}

/** What the run cost, so the price of an LLM-read pipeline is visible rather than hidden. */
export interface MiningReport {
	/** Sessions whose candidates came from cache, free. */
	cached: number;
	/** Sessions sent to the model this run. */
	mined: number;
	/** Sessions the model failed on. Their signals are missing from the counts. */
	failed: number;
}

export interface LearnDigest {
	scannedSessions: number;
	skippedSessions: number;
	/** Where the sessions came from, and what was passed over. */
	scan: SessionScanReport;
	/** What was read by the model versus reused. */
	mining: MiningReport;
	oldestSession?: string;
	newestSession?: string;
	agentsFilePath?: string;
	agentsFileTokens?: number;
	directives: DirectiveCluster[];
	fixes: FixCandidate[];
	workflows: WorkflowCandidate[];
	/** Items held back because nothing new has happened since they were last shown. */
	suppressed: number;
	/** Everything this run put on screen, for the caller to persist. */
	surfaced: Array<{ key: string; lastSeen: string; covered: boolean }>;
}

export interface ExtractOptions {
	cwd: string;
	agentDir: string;
	/**
	 * An extra directory to scan, normally the live session manager's. The
	 * per-cwd default directory is always scanned as well, so a session manager
	 * pointing somewhere unusual cannot hide this directory's history.
	 */
	sessionDir?: string;
	maxSessions?: number;
	maxAgeDays?: number;
	/** Occurrences a directive needs before it is proposed. The signal/noise dial. */
	minRepeats?: number;
	/** Repeats a tool sequence needs before it is proposed as a skill. */
	minWorkflowRepeats?: number;
	/** Cap on each list in the digest. */
	maxProposals?: number;
	/**
	 * What previous runs already showed. Items with no new occurrences since are
	 * held back. Omit (or pass `ignoreState`) to propose everything in the window.
	 */
	state?: LearnState;
	/** Re-propose everything, ignoring what previous runs surfaced (`/learn all`). */
	ignoreState?: boolean;
	/**
	 * Skills a directive can already be covered by. Defaults to the ones loaded
	 * from disk; injectable so tests do not read the developer's real skills.
	 */
	skills?: Array<{ name: string; description: string }>;
	/** Injectable clock, for tests. */
	now?: Date;
}

/** Everything the async pipeline needs beyond the window settings. */
export interface MineOptions extends ExtractOptions {
	/** Reads one session and reports what it saw. */
	miner: Miner;
	/** Decides which proposals are already written down. Defaults to "none are". */
	coverageJudge?: CoverageJudge;
	/** Progress callback, so a cold-cache run is not a silent wait. */
	onProgress?: (progress: { done: number; total: number; cached: number }) => void;
	signal?: AbortSignal;
}

interface SessionHeaderLike {
	type: "session";
	id?: string;
	timestamp?: string;
	cwd?: string;
}

interface EntryLike {
	type: string;
	id?: string;
	parentId?: string | null;
	timestamp?: string;
	message?: AgentMessage;
}

/** One session, reduced to the branch that was actually taken. */
interface ParsedSession {
	file: string;
	id: string;
	timestamp: string;
	entries: EntryLike[];
}

/**
 * Reduce a session's raw entries to the branch that was actually taken.
 *
 * Session files are trees — forks and clones append entries that were never
 * part of the same conversation. Walking parent links back from the last entry
 * keeps the miner from reading two turns that never happened in sequence as if
 * they did. Sessions written before entry ids existed are flat, and for those
 * file order *is* the branch.
 */
function activeBranch(entries: EntryLike[]): EntryLike[] {
	const withIds = entries.filter((e) => typeof e.id === "string");
	if (withIds.length === 0) return entries;

	const byId = new Map<string, EntryLike>();
	for (const entry of withIds) byId.set(entry.id as string, entry);

	const branch: EntryLike[] = [];
	const seen = new Set<string>();
	let cursor: EntryLike | undefined = withIds[withIds.length - 1];
	while (cursor?.id && !seen.has(cursor.id)) {
		seen.add(cursor.id);
		branch.push(cursor);
		cursor = cursor.parentId ? byId.get(cursor.parentId) : undefined;
	}
	return branch.reverse();
}

/**
 * Compare two directory paths the way the filesystem does.
 *
 * A session header stores the cwd as it was typed, and the same directory can
 * be spelled several ways: through a symlink (`/tmp` is `/private/tmp` on
 * macOS), with a trailing separator, or in different case on the
 * case-insensitive filesystems that macOS and Windows ship by default. String
 * equality on `resolve()` alone rejects every one of those, and rejecting them
 * here means silently discarding the whole history the command exists to read.
 */
function normalizeDirPath(path: string): string {
	let resolved = resolve(path);
	try {
		resolved = realpathSync.native(resolved);
	} catch {
		// Deleted or never-created directory: the textual form is all we have.
	}
	// `resolve` already drops a trailing separator except at a filesystem root,
	// where dropping it would turn "/" into "".
	if (resolved.length > 1 && resolved.endsWith(sep)) resolved = resolved.slice(0, -1);
	return process.platform === "win32" || process.platform === "darwin" ? resolved.toLowerCase() : resolved;
}

function sameDirectory(a: string, b: string): boolean {
	return normalizeDirPath(a) === normalizeDirPath(b);
}

/** Reason a candidate file produced no session, for the scan report. */
type SkipReason = "otherCwd" | "unreadable";

function parseSessionFile(file: string, cwd: string, onSkip: (reason: SkipReason) => void): ParsedSession | undefined {
	let raw: string;
	try {
		raw = readFileSync(file, "utf-8");
	} catch {
		onSkip("unreadable");
		return undefined;
	}

	const lines = raw.split("\n");
	let header: SessionHeaderLike | undefined;
	const entries: EntryLike[] = [];
	for (const line of lines) {
		if (!line.trim()) continue;
		if (entries.length >= MAX_ENTRIES_PER_SESSION) break;
		let parsed: EntryLike | SessionHeaderLike;
		try {
			parsed = JSON.parse(line);
		} catch {
			// A partially-flushed final line is normal for a live session.
			continue;
		}
		if (parsed.type === "session") {
			header ??= parsed as SessionHeaderLike;
			continue;
		}
		entries.push(parsed as EntryLike);
	}

	// An explicit `--session` path can put a session for another directory in
	// this directory, so trust the header over the file's location.
	if (header?.cwd && !sameDirectory(header.cwd, cwd)) {
		onSkip("otherCwd");
		return undefined;
	}
	if (entries.length === 0) {
		onSkip("unreadable");
		return undefined;
	}

	return {
		file,
		id: header?.id ?? file,
		timestamp: header?.timestamp ?? statSync(file).mtime.toISOString(),
		entries: activeBranch(entries),
	};
}

/**
 * Every directory this cwd's sessions could be sitting in.
 *
 * The caller passes the live session manager's directory, which is the right
 * answer almost always — but not quite always, and each exception silently
 * emptied the digest. An in-memory session (`--no-session`) reports `""`; an
 * explicit `--session <path>` reports wherever that file lives; a custom
 * `sessionDir` setting points at one shared directory. In every one of those
 * cases the per-cwd default directory still holds the history worth mining, so
 * search both and let the header check sort out what belongs to this cwd.
 */
export function candidateSessionDirs(options: Pick<ExtractOptions, "cwd" | "agentDir" | "sessionDir">): string[] {
	const dirs: string[] = [];
	const seen = new Set<string>();
	for (const dir of [options.sessionDir, getSessionDirPath(options.cwd, options.agentDir)]) {
		if (!dir) continue;
		const key = normalizeDirPath(dir);
		if (seen.has(key)) continue;
		seen.add(key);
		dirs.push(dir);
	}
	return dirs;
}

function listSessions(options: ExtractOptions): {
	sessions: ParsedSession[];
	skipped: number;
	scan: SessionScanReport;
} {
	const dirs = candidateSessionDirs(options);
	const scan: SessionScanReport = {
		dirs,
		missingDirs: [],
		files: 0,
		tooOld: 0,
		otherCwd: 0,
		overLimit: 0,
		unreadable: 0,
	};

	const maxSessions = options.maxSessions ?? DEFAULT_MAX_SESSIONS;
	const maxAgeDays = options.maxAgeDays ?? DEFAULT_MAX_AGE_DAYS;
	const now = options.now ?? new Date();
	const cutoff = now.getTime() - maxAgeDays * 24 * 60 * 60 * 1000;

	const files: string[] = [];
	for (const dir of dirs) {
		if (!existsSync(dir)) {
			scan.missingDirs.push(dir);
			continue;
		}
		try {
			for (const name of readdirSync(dir)) {
				if (name.endsWith(".jsonl")) files.push(join(dir, name));
			}
		} catch {
			scan.missingDirs.push(dir);
		}
	}
	scan.files = files.length;

	// Newest first across all directories, so `maxSessions` keeps the most recent
	// history rather than whichever directory happened to be searched first.
	const dated = files
		.map((file) => {
			try {
				return { file, mtime: statSync(file).mtime.getTime() };
			} catch {
				scan.unreadable++;
				return undefined;
			}
		})
		.filter((f): f is { file: string; mtime: number } => !!f)
		.sort((a, b) => b.mtime - a.mtime);

	const sessions: ParsedSession[] = [];
	const seenIds = new Set<string>();
	let skipped = 0;
	for (const { file, mtime } of dated) {
		if (sessions.length >= maxSessions) {
			scan.overLimit++;
			skipped++;
			continue;
		}
		if (mtime < cutoff) {
			scan.tooOld++;
			skipped++;
			continue;
		}
		const parsed = parseSessionFile(file, options.cwd, (reason) => {
			scan[reason]++;
		});
		if (!parsed) {
			skipped++;
			continue;
		}
		// Searching two directories can turn up the same session twice (an explicit
		// `--session` path inside the default directory). Counting it twice would
		// inflate the cross-session repetition that decides what gets proposed.
		if (seenIds.has(parsed.id)) {
			skipped++;
			continue;
		}
		seenIds.add(parsed.id);
		sessions.push(parsed);
	}
	return { sessions, skipped, scan };
}

/**
 * Hold back items already shown that have not recurred since, then cap the rest.
 *
 * Order matters: suppression runs *before* the cap, or an item you already
 * decided on would occupy one of the few slots the digest has and push a live
 * signal off the list.
 */
function applySuppression<T extends Proposable>(
	items: T[],
	state: LearnState | undefined,
	maxProposals: number,
	covered: (item: T) => boolean,
	onDeclined?: (item: T) => void,
): { kept: T[]; suppressed: number } {
	if (!state) return { kept: items.slice(0, maxProposals), suppressed: 0 };

	const kept: T[] = [];
	let suppressed = 0;
	for (const item of items) {
		const verdict = judge(state, { key: item.key, lastSeen: item.lastSeen, covered: covered(item) });
		if (verdict.suppressed) {
			suppressed++;
			continue;
		}
		if (verdict.previouslyDeclined) onDeclined?.(item);
		kept.push(item);
	}
	return { kept: kept.slice(0, maxProposals), suppressed };
}

/**
 * Where this cwd's sessions were found and what was passed over, without
 * mining anything. `/learn settings` and `/learn stats` report on the window
 * without paying for a model call.
 */
export function scanSessions(options: ExtractOptions): SessionScanReport {
	return listSessions(options).scan;
}

/** Nearest AGENTS.md walking up from cwd, so proposals can be checked against it. */
function findAgentsFile(cwd: string): string | undefined {
	let dir = resolve(cwd);
	while (true) {
		for (const name of ["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"]) {
			const candidate = join(dir, name);
			if (existsSync(candidate)) return candidate;
		}
		const parent = dirname(dir);
		if (parent === dir) return undefined;
		dir = parent;
	}
}

/** Assemble the coverage index for a directory. */
export function buildCoverageIndex(options: {
	cwd: string;
	agentDir: string;
	skills?: Array<{ name: string; description: string }>;
}): CoverageIndex {
	const corpus = coverageCorpus(options.agentDir, findAgentsFile(options.cwd));
	return {
		ruleLines: corpus
			.split("\n")
			.map((line) => line.trim())
			.filter((line) => line.length > 0 && !line.startsWith("#")),
		skills: options.skills ?? loadSkillIndex(options.cwd, options.agentDir),
	};
}

/**
 * Text a proposal is checked against to decide whether it is already written
 * down — the nearest repo context file plus both user scopes.
 *
 * All three matter for suppression, because `/learn` can route a rule to the
 * user scope. Checking only the repo file would report a rule you accepted into
 * `~/.agents/AGENTS.md` as declined.
 */
function coverageCorpus(agentDir: string, repoFile: string | undefined): string {
	const parts: string[] = [];
	for (const candidate of [repoFile, join(getUserAgentsDir(), "AGENTS.md"), join(agentDir, "AGENTS.md")]) {
		if (!candidate || !existsSync(candidate)) continue;
		try {
			parts.push(readFileSync(candidate, "utf-8"));
		} catch {
			// Unreadable context file: treat as absent rather than failing the run.
		}
	}
	return parts.join("\n");
}

/**
 * Skills a proposal could already have become.
 *
 * `/learn` routes long or conditional guidance to a skill rather than a rule, so
 * without this a proposal you adopted *as a skill* would read as declined —
 * looking only at context files sees an unchanged `AGENTS.md` and concludes you
 * passed. Reuses the real loader rather than a second SKILL.md scanner so the
 * set of locations cannot drift from what the session actually loads.
 */
function loadSkillIndex(cwd: string, agentDir: string): Array<{ name: string; description: string }> {
	try {
		return loadSkills({ cwd, agentDir, skillPaths: [], includeDefaults: true }).skills.map((skill) => ({
			name: skill.name,
			description: skill.description ?? "",
		}));
	} catch {
		// Skills are an enrichment here, not the point of the command.
		return [];
	}
}

/**
 * Run the miner over the window, reusing cached results wherever the file has
 * not changed.
 *
 * A session that fails to mine is counted and skipped rather than aborting the
 * run: one provider hiccup on one transcript should cost that transcript's
 * signals, not the whole digest. The failure count is reported so the reader
 * knows the numbers are short.
 */
async function mineSessions(
	sessions: ParsedSession[],
	options: MineOptions,
): Promise<{ mined: MinedSession[]; report: MiningReport }> {
	const mined: MinedSession[] = [];
	const report: MiningReport = { cached: 0, mined: 0, failed: 0 };

	let done = 0;
	for (const session of sessions) {
		if (options.signal?.aborted) break;

		const hash = hashSessionFile(session.file);
		const cached = hash ? readCachedMining(options.agentDir, hash) : undefined;
		if (cached) {
			mined.push({ sessionId: session.id, timestamp: session.timestamp, candidates: cached.candidates });
			report.cached++;
			done++;
			options.onProgress?.({ done, total: sessions.length, cached: report.cached });
			continue;
		}

		const minable: MinableSession = { id: session.id, timestamp: session.timestamp, entries: session.entries };
		try {
			const candidates = await options.miner(minable, options.signal);
			mined.push({ sessionId: session.id, timestamp: session.timestamp, candidates });
			report.mined++;
			if (hash) {
				writeCachedMining(options.agentDir, hash, {
					sessionId: session.id,
					timestamp: session.timestamp,
					candidates,
					minedAt: new Date().toISOString(),
				});
			}
		} catch {
			report.failed++;
		}
		done++;
		options.onProgress?.({ done, total: sessions.length, cached: report.cached });
	}

	return { mined, report };
}

/** Apply the coverage verdicts to the clusters they were asked about. */
function applyCoverage(directives: DirectiveCluster[], verdicts: Map<string, { rule?: string; skill?: string }>): void {
	for (const cluster of directives) {
		const verdict = verdicts.get(cluster.label);
		if (!verdict) continue;
		if (verdict.rule) {
			cluster.status = "restated";
			cluster.existingRule = verdict.rule;
		} else if (verdict.skill) {
			cluster.status = "has-skill";
			cluster.existingSkill = verdict.skill;
		}
	}
}

/** Mine the recent sessions for this cwd and return the ranked digest. */
export async function mineLearnDigest(options: MineOptions): Promise<LearnDigest> {
	const { sessions, skipped, scan } = listSessions(options);

	const agentsFilePath = findAgentsFile(options.cwd);
	let agentsContent: string | undefined;
	if (agentsFilePath) {
		try {
			agentsContent = readFileSync(agentsFilePath, "utf-8");
		} catch {
			agentsContent = undefined;
		}
	}

	const { mined, report } = await mineSessions(sessions, options);
	pruneLearnCache(options.agentDir, options.now);

	const minRepeats = options.minRepeats ?? DEFAULT_MIN_DIRECTIVE_COUNT;
	const maxProposals = options.maxProposals ?? DEFAULT_MAX_PER_CATEGORY;
	const directives = reduceDirectives(mined, minRepeats);
	const fixes = reduceFixes(mined, minRepeats);
	const workflows = reduceWorkflows(mined, options.minWorkflowRepeats ?? DEFAULT_MIN_WORKFLOW_COUNT);

	// Coverage is asked only about what survived the repeat threshold. Judging
	// everything would mean sending the context file alongside a long tail of
	// one-off observations that are never going to be proposed.
	const coverage = buildCoverageIndex({ cwd: options.cwd, agentDir: options.agentDir, skills: options.skills });
	const queries: CoverageQuery[] = directives.map((d) => ({ label: d.label, text: d.text }));
	try {
		const verdicts = await (options.coverageJudge ?? noCoverageJudge)(queries, coverage, options.signal);
		applyCoverage(directives, verdicts);
	} catch {
		// A failed coverage call leaves everything `new`, which over-proposes
		// slightly. That is the right way to fail: the reader can reject a
		// duplicate, but cannot recover a proposal that was wrongly withheld.
	}

	const timestamps = sessions.map((s) => s.timestamp).sort();
	const state = options.ignoreState ? undefined : options.state;

	// Directives carry a real coverage signal — is this written down as a rule or
	// a skill right now? — which is what separates an adopted proposal from a
	// declined one. Fixes and workflows do not: a fix may have become a rule, a
	// skill, or a habit, and which one is not recoverable here, so they get
	// suppression only and are never labelled declined.
	const keptDirectives = applySuppression(
		directives,
		state,
		maxProposals,
		(item) => item.status !== "new",
		(item) => {
			item.previouslyDeclined = true;
		},
	);
	const keptFixes = applySuppression(fixes, state, maxProposals, () => false);
	const keptWorkflows = applySuppression(workflows, state, maxProposals, () => false);

	const surfaced = [
		...keptDirectives.kept.map((d) => ({ key: d.key, lastSeen: d.lastSeen, covered: d.status !== "new" })),
		...keptFixes.kept.map((f) => ({ key: f.key, lastSeen: f.lastSeen, covered: false })),
		...keptWorkflows.kept.map((w) => ({ key: w.key, lastSeen: w.lastSeen, covered: false })),
	];

	return {
		scannedSessions: sessions.length,
		skippedSessions: skipped,
		scan,
		mining: report,
		oldestSession: timestamps[0],
		newestSession: timestamps[timestamps.length - 1],
		agentsFilePath,
		agentsFileTokens:
			agentsContent === undefined ? undefined : Math.round(Buffer.byteLength(agentsContent, "utf-8") / 4),
		directives: keptDirectives.kept,
		fixes: keptFixes.kept,
		workflows: keptWorkflows.kept,
		suppressed: keptDirectives.suppressed + keptFixes.suppressed + keptWorkflows.suppressed,
		surfaced,
	};
}

/** Re-exported so callers do not need to know the marker lives in the miner. */
export const DIGEST_MARKER = LEARN_DIGEST_MARKER;
