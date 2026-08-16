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
import type { Clusterer, ClusterInput } from "./cluster.js";
import { fallbackLabel } from "./cluster.js";
import type { CoverageIndex, CoverageJudge, CoverageQuery } from "./coverage.js";
import { noCoverageJudge } from "./coverage.js";
import type { MinableSession, MinedCandidate, Miner } from "./mine.js";
import type { DirectiveCluster, FixCandidate, MinedSession, Proposable, RequestCandidate } from "./reduce.js";
import { reduceDirectives, reduceFixes, reduceRequests } from "./reduce.js";
import { judge, type LearnState } from "./state.js";

export type { CoverageIndex, CoverageMatch } from "./coverage.js";
export { LEARN_DIGEST_MARKER } from "./mine.js";
export type { DirectiveCluster, DirectiveStatus, FixCandidate, RequestCandidate } from "./reduce.js";

/** Sessions considered, newest first. */
const DEFAULT_MAX_SESSIONS = 20;
/** Sessions older than this are ignored — a pattern that stopped is not a rule. */
const DEFAULT_MAX_AGE_DAYS = 30;
/** Entries parsed per session file, as a guard against pathological transcripts. */
const MAX_ENTRIES_PER_SESSION = 8000;
/** Occurrences a directive needs before it is proposed. */
const DEFAULT_MIN_DIRECTIVE_COUNT = 2;
/**
 * Sessions a request needs before it is worth proposing as a slash command.
 *
 * Higher than the directive bar. A rule you stated twice is a rule; a job you
 * asked for twice may just be a job that came up twice. Three separate sessions
 * is the point at which typing it again is the expensive option.
 */
const DEFAULT_MIN_REQUEST_COUNT = 3;
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
	/**
	 * The run stopped before reading the whole window, so the counts below are
	 * computed from part of it. Callers must not record these as surfaced: a
	 * partial count can fall under the repeat threshold, and bookmarking it would
	 * hide the item on the next run, when the evidence is complete.
	 */
	aborted: boolean;
	/**
	 * The coverage judge failed, so every directive reads `new` whether or not it
	 * is written down. Callers must not record these as surfaced either: the
	 * bookmark stores whether an item was covered when shown, and a wrong `false`
	 * there tells a later run you passed over a proposal you were never given.
	 */
	coverageFailed: boolean;
	oldestSession?: string;
	newestSession?: string;
	agentsFilePath?: string;
	agentsFileTokens?: number;
	directives: DirectiveCluster[];
	fixes: FixCandidate[];
	requests: RequestCandidate[];
	/** Items held back because nothing new has happened since they were last shown. */
	suppressed: number;
	/** Items that cleared every threshold but lost the ranking to `maxProposals`. */
	cut: number;
	/**
	 * What the window contained before the thresholds, so an empty digest can be
	 * read.
	 *
	 * The pipeline filters hard — replayed slash-command bodies, tool output,
	 * quotes that cannot be found in the transcript, then a distinct-session bar
	 * — and every one of those is silent. Without these numbers "nothing to
	 * propose" is unreadable: it could mean the sessions taught nothing, or that
	 * the bar is one session too high, and the reader has no way to tell which
	 * knob to reach for.
	 */
	funnel: {
		/** Occurrences the miner reported and the quote check accepted. */
		candidates: number;
		/** Distinct points after naming — how much the clustering pass actually merged. */
		points: number;
		/** Points that were named and counted but did not clear the repeat threshold. */
		belowThreshold: number;
	};
	/** Everything this run put on screen, for the caller to persist. */
	surfaced: Array<{ key: string; lastSeen: string; covered: boolean; text?: string }>;
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
	minRequestRepeats?: number;
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
	/**
	 * Names the whole window at once, deciding which occurrences are the same
	 * point. Without one, each candidate is named after its own wording, which
	 * groups identical sentences and nothing else.
	 */
	clusterer?: Clusterer;
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
	/** When the session was opened. Describes the window, not what is in it. */
	timestamp: string;
	/**
	 * When the session was last written to.
	 *
	 * This is the clock suppression runs on, and it must not be the session's
	 * start. A session opened yesterday and worked in today would date everything
	 * said in it to yesterday, which can be older than the last `/learn` run — so
	 * something said minutes ago reads as "nothing new since you were last shown
	 * this" and is held back. Per-candidate timestamps would be finer, but the
	 * miner sees an untimestamped blob and would have to invent them; the
	 * session's last activity is deterministic, free, and errs toward showing an
	 * item again rather than hiding it.
	 */
	lastActivity: string;
	entries: EntryLike[];
}

/** Newest entry timestamp on the branch, falling back to when the session opened. */
function lastActivityOf(entries: EntryLike[], fallback: string): string {
	let latest = "";
	for (const entry of entries) {
		if (typeof entry.timestamp === "string" && entry.timestamp > latest) latest = entry.timestamp;
	}
	return latest || fallback;
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

	const branch = activeBranch(entries);
	const opened = header?.timestamp ?? statSync(file).mtime.toISOString();
	return {
		file,
		id: header?.id ?? file,
		timestamp: opened,
		lastActivity: lastActivityOf(branch, opened),
		entries: branch,
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
): { kept: T[]; suppressed: number; cut: number } {
	if (!state) {
		return { kept: items.slice(0, maxProposals), suppressed: 0, cut: Math.max(0, items.length - maxProposals) };
	}

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
	// Anything past the cap cleared every bar and lost on rank alone. It is not
	// suppressed and it is not bookmarked, so it will be back next run — but a
	// digest that silently shows eight of twenty reads as "twenty is all there
	// was", and the reader tunes the wrong knob.
	return { kept: kept.slice(0, maxProposals), suppressed, cut: Math.max(0, kept.length - maxProposals) };
}

/**
 * Where this cwd's sessions were found and what was passed over, without
 * mining anything. `/learn settings` and `/learn stats` report on the window
 * without paying for a model call.
 */
export function scanSessions(options: ExtractOptions): SessionScanReport {
	return listSessions(options).scan;
}

/**
 * What a run would read, without reading it.
 *
 * Runs the real selection — the same age, cwd, cap and de-duplication rules
 * `mineLearnDigest` applies — and then asks the cache about each survivor. It
 * has to be the same selection: this number is what the confirmation prompt
 * quotes, and a prompt that says twelve before reading three is worse than no
 * prompt at all. Hashing the chosen files is cheap next to sending them to a
 * model.
 */
export function planMining(options: ExtractOptions): { total: number; cached: number; pending: number } {
	const { sessions } = listSessions(options);
	let cached = 0;
	let pending = 0;
	for (const session of sessions) {
		const hash = hashSessionFile(session.file);
		if (hash && readCachedMining(options.agentDir, hash)) cached++;
		else pending++;
	}
	return { total: sessions.length, cached, pending };
}

/** One session's candidates before the naming pass has seen them. */
interface RawMinedSession {
	sessionId: string;
	lastActivity: string;
	candidates: MinedCandidate[];
}

/**
 * Name every candidate in the window in one place.
 *
 * The pass runs over the whole window at once rather than per session, which is
 * the entire point: "is this the same point as that" is unanswerable from
 * inside one transcript. Labels already on record are offered as vocabulary so
 * an item you decided on keeps the key it was bookmarked under — without that,
 * a renamed cluster reads as brand new and suppression quietly stops working.
 *
 * A failed call falls back to naming each candidate after its own wording,
 * which groups identical sentences and nothing else. That is the behaviour the
 * pipeline had before this stage existed, so a clustering outage costs recall,
 * not the run.
 */
async function labelSessions(
	raw: RawMinedSession[],
	clusterer: Clusterer | undefined,
	knownLabels: string[],
	signal?: AbortSignal,
): Promise<MinedSession[]> {
	const inputs: ClusterInput[] = [];
	const origin: Array<{ session: number; candidate: number }> = [];
	for (const [sessionIndex, session] of raw.entries()) {
		for (const [candidateIndex, candidate] of session.candidates.entries()) {
			inputs.push({ id: inputs.length, kind: candidate.kind, text: candidate.text });
			origin.push({ session: sessionIndex, candidate: candidateIndex });
		}
	}

	let labels = new Map<number, string>();
	if (clusterer && inputs.length > 0) {
		try {
			labels = await clusterer(inputs, knownLabels, signal);
		} catch {
			// Fall through to per-text labels below.
		}
	}

	const labelled: MinedSession[] = raw.map((session) => ({
		sessionId: session.sessionId,
		lastActivity: session.lastActivity,
		candidates: [],
	}));
	for (const [index, input] of inputs.entries()) {
		const where = origin[index];
		const candidate = where ? raw[where.session]?.candidates[where.candidate] : undefined;
		if (!where || !candidate) continue;
		labelled[where.session]?.candidates.push({
			...candidate,
			label: labels.get(input.id) ?? fallbackLabel(input.text),
		});
	}
	return labelled;
}

/** Labels already on record, so the naming pass can reuse rather than reinvent them. */
function knownLabelsFrom(state: LearnState | undefined): string[] {
	if (!state) return [];
	return Object.keys(state.surfaced).map((key) => key.slice(key.indexOf(":") + 1));
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

/**
 * Turn one context file into rule lines the coverage judge can reason about.
 *
 * Headings were dropped and the lines under them sent bare, which asks the
 * model to decide whether a proposal is in scope using text with the scope
 * removed — "stage only your own files" reads very differently under "Git Rules
 * for Parallel Agents" than on its own. So each line carries its heading path,
 * and the scope it came from, since the corpus spans a repo file and two user
 * ones and a rule's home decides who it binds.
 *
 * Fenced blocks go: a code sample illustrates a rule, it is not one, and on a
 * real file it is a large share of the non-bullet text.
 */
function ruleLinesOf(content: string, scope: string): string[] {
	const lines: string[] = [];
	const headings: string[] = [];
	let inFence = false;

	for (const raw of content.split("\n")) {
		const line = raw.trim();
		if (line.startsWith("```")) {
			inFence = !inFence;
			continue;
		}
		if (inFence || line.length === 0) continue;

		const heading = /^(#{1,6})\s+(.*)$/.exec(line);
		if (heading) {
			const depth = heading[1]?.length ?? 1;
			headings.length = Math.min(headings.length, depth - 1);
			headings[depth - 1] = heading[2] ?? "";
			continue;
		}

		const path = headings.filter(Boolean).join(" > ");
		lines.push(path ? `[${scope}] ${path} > ${line}` : `[${scope}] ${line}`);
	}
	return lines;
}

/** Assemble the coverage index for a directory. */
export function buildCoverageIndex(options: {
	cwd: string;
	agentDir: string;
	skills?: Array<{ name: string; description: string }>;
}): CoverageIndex {
	const ruleLines: string[] = [];
	for (const file of coverageFiles(options.agentDir, findAgentsFile(options.cwd))) {
		ruleLines.push(...ruleLinesOf(file.content, file.scope));
	}
	return { ruleLines, skills: options.skills ?? loadSkillIndex(options.cwd, options.agentDir) };
}

/**
 * Text a proposal is checked against to decide whether it is already written
 * down — the nearest repo context file plus both user scopes.
 *
 * All three matter for suppression, because `/learn` can route a rule to the
 * user scope. Checking only the repo file would report a rule you accepted into
 * `~/.agents/AGENTS.md` as declined.
 */
function coverageFiles(agentDir: string, repoFile: string | undefined): Array<{ scope: string; content: string }> {
	const files: Array<{ scope: string; content: string }> = [];
	const candidates: Array<{ scope: string; path: string | undefined }> = [
		{ scope: "repo", path: repoFile },
		{ scope: "user", path: join(getUserAgentsDir(), "AGENTS.md") },
		{ scope: "user", path: join(agentDir, "AGENTS.md") },
	];
	for (const candidate of candidates) {
		if (!candidate.path || !existsSync(candidate.path)) continue;
		try {
			files.push({ scope: candidate.scope, content: readFileSync(candidate.path, "utf-8") });
		} catch {
			// Unreadable context file: treat as absent rather than failing the run.
		}
	}
	return files;
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
 *
 * Cancellation is different from failure and is reported separately. A run
 * stopped half way has counted only some of the window, so its numbers are not
 * merely short — they are wrong in a way that would poison the bookmark if the
 * digest were treated as a completed run.
 */
async function mineSessions(
	sessions: ParsedSession[],
	options: MineOptions,
): Promise<{ mined: RawMinedSession[]; report: MiningReport; aborted: boolean }> {
	const mined: RawMinedSession[] = [];
	const report: MiningReport = { cached: 0, mined: 0, failed: 0 };

	let done = 0;
	for (const session of sessions) {
		if (options.signal?.aborted) return { mined, report, aborted: true };

		const hash = hashSessionFile(session.file);
		const cached = hash ? readCachedMining(options.agentDir, hash) : undefined;
		if (cached) {
			mined.push({ sessionId: session.id, lastActivity: session.lastActivity, candidates: cached.candidates });
			report.cached++;
			done++;
			options.onProgress?.({ done, total: sessions.length, cached: report.cached });
			continue;
		}

		const minable: MinableSession = { id: session.id, timestamp: session.timestamp, entries: session.entries };
		try {
			const candidates = await options.miner(minable, options.signal);
			mined.push({ sessionId: session.id, lastActivity: session.lastActivity, candidates });
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
			// A cancelled request surfaces here as a rejection. That is not the
			// provider failing on this transcript, so it must not be counted as one.
			if (options.signal?.aborted) return { mined, report, aborted: true };
			report.failed++;
		}
		done++;
		options.onProgress?.({ done, total: sessions.length, cached: report.cached });
	}

	return { mined, report, aborted: false };
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

	const { mined, report, aborted } = await mineSessions(sessions, options);
	pruneLearnCache(options.agentDir, options.now);

	const state = options.ignoreState ? undefined : options.state;
	// Named against the labels already on record — including in `all` mode, where
	// suppression is off but the bookmark still has to line up next run.
	const labelled = await labelSessions(mined, options.clusterer, knownLabelsFrom(options.state), options.signal);

	const minRepeats = options.minRepeats ?? DEFAULT_MIN_DIRECTIVE_COUNT;
	const maxProposals = options.maxProposals ?? DEFAULT_MAX_PER_CATEGORY;
	const directives = reduceDirectives(labelled, minRepeats);
	const fixes = reduceFixes(labelled, minRepeats);
	const requests = reduceRequests(labelled, options.minRequestRepeats ?? DEFAULT_MIN_REQUEST_COUNT);

	// Counted with the threshold at 1, which is the same reduce over the same
	// input — so the difference is exactly what the threshold cost, rather than an
	// estimate of it.
	const everyPoint =
		reduceDirectives(labelled, 1).length + reduceFixes(labelled, 1).length + reduceRequests(labelled, 1).length;
	const funnel = {
		candidates: labelled.reduce((sum, session) => sum + session.candidates.length, 0),
		points: everyPoint,
		belowThreshold: everyPoint - directives.length - fixes.length - requests.length,
	};

	// Coverage is asked only about what survived the repeat threshold. Judging
	// everything would mean sending the context file alongside a long tail of
	// one-off observations that are never going to be proposed.
	const coverage = buildCoverageIndex({ cwd: options.cwd, agentDir: options.agentDir, skills: options.skills });
	const queries: CoverageQuery[] = directives.map((d) => ({ label: d.label, text: d.text }));
	let coverageFailed = false;
	try {
		const verdicts = await (options.coverageJudge ?? noCoverageJudge)(queries, coverage, options.signal);
		applyCoverage(directives, verdicts);
	} catch {
		// A failed coverage call leaves everything `new`, which over-proposes
		// slightly. That is the right way to fail: the reader can reject a
		// duplicate, but cannot recover a proposal that was wrongly withheld. What
		// must not happen is writing that guess down as if it were a reading.
		coverageFailed = true;
	}

	const timestamps = sessions.map((s) => s.timestamp).sort();
	// Directives carry a real coverage signal — is this written down as a rule or
	// a skill right now? — which is what separates an adopted proposal from a
	// declined one. Fixes and requests do not: a fix may have become a rule, a
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
	const keptRequests = applySuppression(requests, state, maxProposals, () => false);

	const surfaced = [
		// Directives carry their wording forward so a later `/learn stats` can ask
		// about coverage using the sentence rather than the slug that names it.
		...keptDirectives.kept.map((d) => ({
			key: d.key,
			lastSeen: d.lastSeen,
			covered: d.status !== "new",
			text: d.text,
		})),
		...keptFixes.kept.map((f) => ({ key: f.key, lastSeen: f.lastSeen, covered: false })),
		...keptRequests.kept.map((r) => ({ key: r.key, lastSeen: r.lastSeen, covered: false })),
	];

	return {
		scannedSessions: sessions.length,
		skippedSessions: skipped,
		scan,
		mining: report,
		aborted,
		coverageFailed,
		oldestSession: timestamps[0],
		newestSession: timestamps[timestamps.length - 1],
		agentsFilePath,
		agentsFileTokens:
			agentsContent === undefined ? undefined : Math.round(Buffer.byteLength(agentsContent, "utf-8") / 4),
		directives: keptDirectives.kept,
		fixes: keptFixes.kept,
		requests: keptRequests.kept,
		suppressed: keptDirectives.suppressed + keptFixes.suppressed + keptRequests.suppressed,
		cut: keptDirectives.cut + keptFixes.cut + keptRequests.cut,
		funnel,
		surfaced,
	};
}
