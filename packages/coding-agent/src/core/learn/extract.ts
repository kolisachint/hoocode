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
 * The split of labour is deliberate. This module is entirely deterministic: it
 * parses, filters, normalizes, counts and ranks. Judgement — is this a rule, how
 * should it be phrased, which scope owns it — belongs to the model reading the
 * digest, which is why the output carries evidence (counts, sessions, dates)
 * rather than conclusions.
 */

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import type { TextContent, ToolCall } from "@kolisachint/hoocode-ai";
import { getUserAgentsDir } from "../../config.js";
import { getDefaultSessionDir } from "../session-manager.js";
import { loadSkills } from "../skills.js";
import {
	contentWords,
	isBenignFailure,
	isRuleShapedDirective,
	normalizeCommand,
	normalizeDirective,
	normalizeErrorSignature,
	wordOverlap,
} from "./normalize.js";
import { judge, type LearnState } from "./state.js";

/**
 * Prefix on the message `/learn` injects. The digest is persisted like any user
 * turn, so without this marker the next `/learn` would mine its own output and
 * every proposal would compound its own count.
 */
export const LEARN_DIGEST_MARKER = "[learn-digest]";

/** Sessions considered, newest first. */
const DEFAULT_MAX_SESSIONS = 20;
/** Sessions older than this are ignored — a pattern that stopped is not a rule. */
const DEFAULT_MAX_AGE_DAYS = 30;
/** Entries parsed per session file, as a guard against pathological transcripts. */
const MAX_ENTRIES_PER_SESSION = 8000;
/** Tool calls per session fed to the workflow detector. */
const MAX_TOOL_CALLS_PER_SESSION = 400;
/** How far forward the fix extractor looks for the same command succeeding. */
const FIX_LOOKAHEAD = 40;
/** Word overlap against an existing rule above which a directive counts as covered. */
const COVERED_OVERLAP = 0.6;
/**
 * The same bar for skills, set higher on purpose.
 *
 * A rule is one line, so overlap against it is a sharp signal. A skill is a name
 * plus a description written to attract matches, which is a far larger haystack
 * — a short directive's words turn up in it by chance much more readily. The
 * higher bar and the truncation below keep "you already have a skill for this"
 * from being said on a coincidence.
 */
const SKILL_COVERED_OVERLAP = 0.75;
/** Description characters considered. The opening says what a skill does; the rest is trigger bait. */
const SKILL_DESCRIPTION_CHARS = 300;
/** Directives must reach this many occurrences to be reported at all. */
const DEFAULT_MIN_DIRECTIVE_COUNT = 2;
/** Tool sequence lengths considered as workflow candidates. */
const WORKFLOW_MIN_LEN = 3;
const WORKFLOW_MAX_LEN = 5;
/** Repeats before a tool sequence is worth proposing as a skill. */
const MIN_WORKFLOW_COUNT = 3;
/** Cap on each list in the digest, so the model's budget goes to the top signals. */
const MAX_PER_CATEGORY = 8;

/**
 * Where a repeated directive already lives, if anywhere.
 *
 * A directive covered by a rule and said only once is simply dropped — the rule
 * exists and is working. What survives is one of three cases, and they want
 * different responses:
 *
 * - `new` — not written down anywhere. Propose it.
 * - `restated` — a context-file rule covers it and you said it anyway, so the
 *   rule is not working. Rewrite it; do not add a second one.
 * - `has-skill` — a *skill* covers it and you asked by hand anyway, which
 *   usually means the skill's `description` is not triggering. Sharpen the
 *   description rather than writing a rule that duplicates the skill.
 */
export type DirectiveStatus = "new" | "restated" | "has-skill";

/** Fields every proposable item shares, so suppression can be applied uniformly. */
interface Proposable {
	/** Stable identity across runs — what the state file remembers. */
	key: string;
	/** Newest occurrence in the window, ISO. */
	lastSeen: string;
}

export interface DirectiveCluster extends Proposable {
	/** Representative raw text, the longest seen in the cluster. */
	text: string;
	normalized: string;
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
	 * a skill — so you saw this proposal and passed on it. Only meaningful for
	 * directives, which are the only items with a real coverage signal.
	 */
	previouslyDeclined: boolean;
}

export interface FixCandidate extends Proposable {
	/** Normalized failing command. */
	command: string;
	/** Normalized error signature, the dedupe key. */
	signature: string;
	/** Short raw excerpt, so the model sees the real error text. */
	errorExcerpt: string;
	/** Commands run between the failure and the pass. */
	interveningCommands: string[];
	/** Files edited between the failure and the pass. */
	editedFiles: string[];
	/** Times this signature failed and was resolved across the window. */
	count: number;
	sessions: number;
}

export interface WorkflowCandidate extends Proposable {
	/** Tool-call signatures in order. */
	steps: string[];
	count: number;
	sessions: number;
}

export interface LearnDigest {
	scannedSessions: number;
	skippedSessions: number;
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
	/** Override the directory scanned. Defaults to the per-cwd session dir. */
	sessionDir?: string;
	maxSessions?: number;
	maxAgeDays?: number;
	/** Occurrences a directive needs before it is proposed. The signal/noise dial. */
	minRepeats?: number;
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

function textOf(content: unknown): string {
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	return content
		.map((block) =>
			block && typeof block === "object" && (block as TextContent).type === "text"
				? ((block as TextContent).text ?? "")
				: "",
		)
		.join("\n")
		.trim();
}

function isToolCall(block: unknown): block is ToolCall {
	return !!block && typeof block === "object" && (block as ToolCall).type === "toolCall";
}

/**
 * Reduce a session's raw entries to the branch that was actually taken.
 *
 * Session files are trees — forks and clones append entries that were never
 * part of the same conversation. Walking parent links back from the last entry
 * keeps the extractor from stitching a "fix" out of two turns that never
 * happened in sequence. Sessions written before entry ids existed are flat, and
 * for those file order *is* the branch.
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

function parseSessionFile(file: string, cwd: string): ParsedSession | undefined {
	let raw: string;
	try {
		raw = readFileSync(file, "utf-8");
	} catch {
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
	if (header?.cwd && resolve(header.cwd) !== resolve(cwd)) return undefined;
	if (entries.length === 0) return undefined;

	return {
		file,
		id: header?.id ?? file,
		timestamp: header?.timestamp ?? statSync(file).mtime.toISOString(),
		entries: activeBranch(entries),
	};
}

function listSessions(options: ExtractOptions): { sessions: ParsedSession[]; skipped: number } {
	const dir = options.sessionDir ?? getDefaultSessionDir(options.cwd, options.agentDir);
	if (!existsSync(dir)) return { sessions: [], skipped: 0 };

	const maxSessions = options.maxSessions ?? DEFAULT_MAX_SESSIONS;
	const maxAgeDays = options.maxAgeDays ?? DEFAULT_MAX_AGE_DAYS;
	const now = options.now ?? new Date();
	const cutoff = now.getTime() - maxAgeDays * 24 * 60 * 60 * 1000;

	let files: string[];
	try {
		files = readdirSync(dir)
			.filter((f) => f.endsWith(".jsonl"))
			.map((f) => join(dir, f));
	} catch {
		return { sessions: [], skipped: 0 };
	}

	const dated = files
		.map((file) => {
			try {
				return { file, mtime: statSync(file).mtime.getTime() };
			} catch {
				return undefined;
			}
		})
		.filter((f): f is { file: string; mtime: number } => !!f)
		.sort((a, b) => b.mtime - a.mtime);

	const sessions: ParsedSession[] = [];
	let skipped = 0;
	for (const { file, mtime } of dated) {
		if (sessions.length >= maxSessions) {
			skipped++;
			continue;
		}
		if (mtime < cutoff) {
			skipped++;
			continue;
		}
		const parsed = parseSessionFile(file, options.cwd);
		if (parsed) sessions.push(parsed);
		else skipped++;
	}
	return { sessions, skipped };
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
	covered: (item: T) => boolean,
	onDeclined?: (item: T) => void,
): { kept: T[]; suppressed: number } {
	if (!state) return { kept: items.slice(0, MAX_PER_CATEGORY), suppressed: 0 };

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
	return { kept: kept.slice(0, MAX_PER_CATEGORY), suppressed };
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

interface ToolEvent {
	name: string;
	args: Record<string, any>;
	/** Set once the matching result is seen. */
	isError?: boolean;
	output?: string;
}

/** Pair tool calls with their results along one branch, in call order. */
function toolEvents(entries: EntryLike[]): ToolEvent[] {
	const byCallId = new Map<string, ToolEvent>();
	const ordered: ToolEvent[] = [];

	for (const entry of entries) {
		const message = entry.type === "message" ? entry.message : undefined;
		if (!message) continue;
		if (message.role === "assistant") {
			for (const block of (message.content ?? []) as unknown[]) {
				if (!isToolCall(block)) continue;
				const event: ToolEvent = { name: block.name, args: block.arguments ?? {} };
				byCallId.set(block.id, event);
				ordered.push(event);
			}
		} else if (message.role === "toolResult") {
			const event = byCallId.get(message.toolCallId);
			if (!event) continue;
			event.isError = message.isError;
			event.output = textOf(message.content);
		}
	}
	return ordered;
}

/** User turns worth mining, in order, with the digest's own output excluded. */
function userDirectives(entries: EntryLike[]): string[] {
	const out: string[] = [];
	for (const entry of entries) {
		const message = entry.type === "message" ? entry.message : undefined;
		if (!message || message.role !== "user") continue;
		const text = textOf(message.content);
		if (!text || text.startsWith(LEARN_DIGEST_MARKER)) continue;
		if (!isRuleShapedDirective(text)) continue;
		out.push(text.trim());
	}
	return out;
}

function clusterDirectives(
	perSession: Array<{ session: ParsedSession; directives: string[] }>,
	agentsContent: string | undefined,
	skills: Array<{ name: string; description: string }>,
	minRepeats: number,
): DirectiveCluster[] {
	interface Acc {
		text: string;
		normalized: string;
		count: number;
		sessions: Set<string>;
		lastSeen: string;
	}
	const acc = new Map<string, Acc>();

	for (const { session, directives } of perSession) {
		for (const text of directives) {
			const normalized = normalizeDirective(text);
			if (!normalized) continue;
			const existing = acc.get(normalized);
			if (existing) {
				existing.count++;
				existing.sessions.add(session.id);
				if (session.timestamp > existing.lastSeen) existing.lastSeen = session.timestamp;
				if (text.length > existing.text.length) existing.text = text;
			} else {
				acc.set(normalized, {
					text,
					normalized,
					count: 1,
					sessions: new Set([session.id]),
					lastSeen: session.timestamp,
				});
			}
		}
	}

	const agentsLines = (agentsContent ?? "")
		.split("\n")
		.map((l) => l.trim())
		.filter((l) => l.length > 0 && !l.startsWith("#"));

	const clusters: DirectiveCluster[] = [];
	for (const entry of acc.values()) {
		if (entry.count < minRepeats) continue;

		const words = contentWords(entry.text);
		let bestLine: string | undefined;
		let bestOverlap = 0;
		for (const line of agentsLines) {
			const overlap = wordOverlap(words, line);
			if (overlap > bestOverlap) {
				bestOverlap = overlap;
				bestLine = line;
			}
		}

		let bestSkill: string | undefined;
		let bestSkillOverlap = 0;
		for (const skill of skills) {
			const haystack = `${skill.name} ${skill.description.slice(0, SKILL_DESCRIPTION_CHARS)}`;
			const overlap = wordOverlap(words, haystack);
			if (overlap > bestSkillOverlap) {
				bestSkillOverlap = overlap;
				bestSkill = skill.name;
			}
		}

		// Everything reaching here cleared the repeat threshold. Suppression handles
		// the case that used to make these labels lie — a proposal accepted from a
		// previous run coming back as "not working" when nothing had happened
		// since. By the time an item survives that filter, a match genuinely means
		// you repeated yourself after the rule or skill already existed.
		//
		// A rule wins over a skill when both match: it is the more specific answer,
		// and its "rewrite this line" advice is more actionable than "sharpen a
		// description".
		const coveredByRule = bestOverlap >= COVERED_OVERLAP;
		const coveredBySkill = !coveredByRule && bestSkillOverlap >= SKILL_COVERED_OVERLAP;
		clusters.push({
			key: `directive:${entry.normalized}`,
			text: entry.text,
			normalized: entry.normalized,
			count: entry.count,
			sessions: entry.sessions.size,
			lastSeen: entry.lastSeen,
			status: coveredByRule ? "restated" : coveredBySkill ? "has-skill" : "new",
			existingRule: coveredByRule ? bestLine : undefined,
			existingSkill: coveredBySkill ? bestSkill : undefined,
			previouslyDeclined: false,
		});
	}

	return clusters.sort((a, b) => b.sessions - a.sessions || b.count - a.count || a.text.localeCompare(b.text));
}

/** Files a mutating tool touched, for the resolution summary. */
function editedFile(event: ToolEvent): string | undefined {
	if (!["edit", "write", "multi_edit", "apply_patch"].includes(event.name)) return undefined;
	const path = event.args?.path ?? event.args?.file_path ?? event.args?.filePath;
	return typeof path === "string" ? path : undefined;
}

function extractFixes(perSession: Array<{ session: ParsedSession; events: ToolEvent[] }>): FixCandidate[] {
	interface Acc {
		candidate: FixCandidate;
		sessions: Set<string>;
	}
	const acc = new Map<string, Acc>();

	for (const { session, events } of perSession) {
		for (let i = 0; i < events.length; i++) {
			const failure = events[i]!;
			if (failure.name !== "bash" || !failure.isError) continue;
			const command = typeof failure.args?.command === "string" ? failure.args.command : "";
			if (!command || isBenignFailure(command)) continue;

			const normalized = normalizeCommand(command);
			const interveningCommands: string[] = [];
			const editedFiles: string[] = [];
			let resolved = false;

			for (let j = i + 1; j < Math.min(events.length, i + 1 + FIX_LOOKAHEAD); j++) {
				const next = events[j]!;
				const file = editedFile(next);
				if (file) editedFiles.push(file);

				if (next.name !== "bash") continue;
				const nextCommand = typeof next.args?.command === "string" ? next.args.command : "";
				if (!nextCommand) continue;

				// The same command later succeeding is the only evidence that the
				// problem was actually fixed. A *different* command passing says
				// nothing, and neither does the model moving on.
				if (normalizeCommand(nextCommand) === normalized && !next.isError) {
					resolved = true;
					break;
				}
				interveningCommands.push(nextCommand.trim());
			}

			if (!resolved) continue;

			const signature = normalizeErrorSignature(failure.output ?? "");
			if (!signature) continue;

			const key = `${normalized} ${signature}`;
			const existing = acc.get(key);
			if (existing) {
				existing.candidate.count++;
				existing.sessions.add(session.id);
				if (session.timestamp > existing.candidate.lastSeen) existing.candidate.lastSeen = session.timestamp;
			} else {
				acc.set(key, {
					sessions: new Set([session.id]),
					candidate: {
						key: `fix:${key}`,
						command: normalized,
						signature,
						errorExcerpt: (failure.output ?? "").replace(/\s+/g, " ").trim().slice(0, 240),
						interveningCommands: [...new Set(interveningCommands)].slice(0, 5),
						editedFiles: [...new Set(editedFiles)].slice(0, 5),
						count: 1,
						sessions: 1,
						lastSeen: session.timestamp,
					},
				});
			}
		}
	}

	const out: FixCandidate[] = [];
	for (const { candidate, sessions } of acc.values()) {
		candidate.sessions = sessions.size;
		out.push(candidate);
	}
	return out.sort((a, b) => b.count - a.count || b.sessions - a.sessions || a.signature.localeCompare(b.signature));
}

/** A tool call reduced to a comparable step: the tool, plus the verb for bash. */
function stepSignature(event: ToolEvent): string {
	if (event.name === "bash") {
		const command = typeof event.args?.command === "string" ? event.args.command : "";
		const head = normalizeCommand(command).split(" ").slice(0, 2).join(" ");
		return head ? `bash:${head}` : "bash";
	}
	return event.name;
}

function extractWorkflows(perSession: Array<{ session: ParsedSession; events: ToolEvent[] }>): WorkflowCandidate[] {
	interface Acc {
		steps: string[];
		count: number;
		sessions: Set<string>;
		lastSeen: string;
	}
	const acc = new Map<string, Acc>();

	for (const { session, events } of perSession) {
		const steps = events.slice(0, MAX_TOOL_CALLS_PER_SESSION).map(stepSignature);
		for (let len = WORKFLOW_MIN_LEN; len <= WORKFLOW_MAX_LEN; len++) {
			for (let i = 0; i + len <= steps.length; i++) {
				const window = steps.slice(i, i + len);
				// A run of one repeated tool is a loop, not a workflow.
				if (new Set(window).size < 2) continue;
				const key = window.join(" > ");
				const existing = acc.get(key);
				if (existing) {
					existing.count++;
					existing.sessions.add(session.id);
					if (session.timestamp > existing.lastSeen) existing.lastSeen = session.timestamp;
				} else {
					acc.set(key, { steps: window, count: 1, sessions: new Set([session.id]), lastSeen: session.timestamp });
				}
			}
		}
	}

	const candidates = [...acc.values()]
		.filter((entry) => entry.count >= MIN_WORKFLOW_COUNT)
		.map((entry) => ({
			key: `workflow:${entry.steps.join(" > ")}`,
			steps: entry.steps,
			count: entry.count,
			sessions: entry.sessions.size,
			lastSeen: entry.lastSeen,
		}));

	// Prefer longer sequences at equal frequency: a five-step workflow is a more
	// useful skill than the three-step prefix it contains.
	return candidates.sort(
		(a, b) => b.count - a.count || b.steps.length - a.steps.length || a.steps.join().localeCompare(b.steps.join()),
	);
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

/** Mine the recent sessions for this cwd and return the ranked digest. */
export function extractLearnDigest(options: ExtractOptions): LearnDigest {
	const { sessions, skipped } = listSessions(options);

	const agentsFilePath = findAgentsFile(options.cwd);
	let agentsContent: string | undefined;
	if (agentsFilePath) {
		try {
			agentsContent = readFileSync(agentsFilePath, "utf-8");
		} catch {
			agentsContent = undefined;
		}
	}
	const corpus = coverageCorpus(options.agentDir, agentsFilePath);

	const withDirectives = sessions.map((session) => ({ session, directives: userDirectives(session.entries) }));
	const withEvents = sessions.map((session) => ({ session, events: toolEvents(session.entries) }));

	const timestamps = sessions.map((s) => s.timestamp).sort();
	const state = options.ignoreState ? undefined : options.state;

	// Directives carry a real coverage signal — is this written down as a rule or
	// a skill right now? — which is what separates an adopted proposal from a
	// declined one. Fixes and workflows do not: a fix may have become a rule, a
	// skill, or a habit, and which one is not recoverable here, so they get
	// suppression only and are never labelled declined.
	const skills = options.skills ?? loadSkillIndex(options.cwd, options.agentDir);
	const directives = applySuppression(
		clusterDirectives(withDirectives, corpus, skills, options.minRepeats ?? DEFAULT_MIN_DIRECTIVE_COUNT),
		state,
		(item) => item.status !== "new",
		(item) => {
			item.previouslyDeclined = true;
		},
	);
	const fixes = applySuppression(extractFixes(withEvents), state, () => false);
	const workflows = applySuppression(extractWorkflows(withEvents), state, () => false);

	const surfaced = [
		...directives.kept.map((d) => ({ key: d.key, lastSeen: d.lastSeen, covered: d.status !== "new" })),
		...fixes.kept.map((f) => ({ key: f.key, lastSeen: f.lastSeen, covered: false })),
		...workflows.kept.map((w) => ({ key: w.key, lastSeen: w.lastSeen, covered: false })),
	];

	return {
		scannedSessions: sessions.length,
		skippedSessions: skipped,
		oldestSession: timestamps[0],
		newestSession: timestamps[timestamps.length - 1],
		agentsFilePath,
		agentsFileTokens:
			agentsContent === undefined ? undefined : Math.round(Buffer.byteLength(agentsContent, "utf-8") / 4),
		directives: directives.kept,
		fixes: fixes.kept,
		workflows: workflows.kept,
		suppressed: directives.suppressed + fixes.suppressed + workflows.suppressed,
		surfaced,
	};
}
