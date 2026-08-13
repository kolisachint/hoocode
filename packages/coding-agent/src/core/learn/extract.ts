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
import { getDefaultSessionDir } from "../session-manager.js";
import {
	contentWords,
	isBenignFailure,
	isRuleShapedDirective,
	normalizeCommand,
	normalizeDirective,
	normalizeErrorSignature,
	wordOverlap,
} from "./normalize.js";

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
 * Two outcomes, not three. A directive already covered by AGENTS.md and said
 * only once is simply dropped — the rule exists and is working. What survives
 * is either not written down (`new`) or written down *and still being repeated*
 * (`restated`), and only the second one tells you something you could not have
 * learned from reading the file.
 */
export type DirectiveStatus = "new" | "restated";

export interface DirectiveCluster {
	/** Representative raw text, the longest seen in the cluster. */
	text: string;
	normalized: string;
	/** Total times said. */
	count: number;
	/** Distinct sessions it was said in — the stronger of the two counts. */
	sessions: number;
	lastSeen: string;
	status: DirectiveStatus;
	/** The existing rule line matched, when status is `restated`. */
	existingRule?: string;
}

export interface FixCandidate {
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
	lastSeen: string;
}

export interface WorkflowCandidate {
	/** Tool-call signatures in order. */
	steps: string[];
	count: number;
	sessions: number;
	lastSeen: string;
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

		// Everything reaching here cleared the repeat threshold, so a match against
		// an existing rule means the rule is there and not working — it wants
		// rewriting, not duplicating.
		const covered = bestOverlap >= COVERED_OVERLAP;
		clusters.push({
			text: entry.text,
			normalized: entry.normalized,
			count: entry.count,
			sessions: entry.sessions.size,
			lastSeen: entry.lastSeen,
			status: covered ? "restated" : "new",
			existingRule: covered ? bestLine : undefined,
		});
	}

	return clusters
		.sort((a, b) => b.sessions - a.sessions || b.count - a.count || a.text.localeCompare(b.text))
		.slice(0, MAX_PER_CATEGORY);
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
	return out
		.sort((a, b) => b.count - a.count || b.sessions - a.sessions || a.signature.localeCompare(b.signature))
		.slice(0, MAX_PER_CATEGORY);
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
			steps: entry.steps,
			count: entry.count,
			sessions: entry.sessions.size,
			lastSeen: entry.lastSeen,
		}));

	// Prefer longer sequences at equal frequency: a five-step workflow is a more
	// useful skill than the three-step prefix it contains.
	return candidates
		.sort(
			(a, b) => b.count - a.count || b.steps.length - a.steps.length || a.steps.join().localeCompare(b.steps.join()),
		)
		.slice(0, MAX_PER_CATEGORY);
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

	const withDirectives = sessions.map((session) => ({ session, directives: userDirectives(session.entries) }));
	const withEvents = sessions.map((session) => ({ session, events: toolEvents(session.entries) }));

	const timestamps = sessions.map((s) => s.timestamp).sort();

	return {
		scannedSessions: sessions.length,
		skippedSessions: skipped,
		oldestSession: timestamps[0],
		newestSession: timestamps[timestamps.length - 1],
		agentsFilePath,
		agentsFileTokens:
			agentsContent === undefined ? undefined : Math.round(Buffer.byteLength(agentsContent, "utf-8") / 4),
		directives: clusterDirectives(withDirectives, agentsContent, options.minRepeats ?? DEFAULT_MIN_DIRECTIVE_COUNT),
		fixes: extractFixes(withEvents),
		workflows: extractWorkflows(withEvents),
	};
}
