/**
 * The map half of `/learn`: a model reads one session transcript and says what
 * it saw.
 *
 * This replaces the regex gate that used to decide which user turns were worth
 * looking at. That gate was a whitelist of imperative words, so a directive
 * phrased any other way — "we're on bun now", "that's not how our error
 * handling works" — was not ranked low, it was invisible. Recall was traded for
 * a token budget, silently and unrecoverably.
 *
 * The trade here is explicit instead. Every user turn goes to the model
 * verbatim; the budget is enforced by chunking and by a session cap the reader
 * can see, not by a filter they cannot.
 *
 * What the model does *not* do is count. It reports occurrences one session at
 * a time, and each one carries a `label` — its own normalization of what was
 * meant. Counting identical labels across sessions is arithmetic, and it stays
 * in code (see `reduce.ts`), for two reasons: models do not count reliably over
 * long contexts, and a session mined in isolation cannot see recurrence anyway.
 * Semantic grouping is the model's job; the number is not.
 */

import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import type { Model, TextContent, ToolCall } from "@kolisachint/hoocode-ai";
import { completeSimple } from "@kolisachint/hoocode-ai";

/** What kind of thing the model noticed. */
export type CandidateKind = "directive" | "fix" | "workflow";

/**
 * One occurrence, as reported by the model reading a single session.
 *
 * `label` is the load-bearing field. It is the model's canonical name for what
 * was meant — "use-bun-not-npm" for all of "we're on bun now", "stop using
 * npm", and "pnpm isn't what we use" — and it is what the reduce step groups
 * on. Getting a stable label out of the model is what buys semantic clustering
 * that `normalizeDirective`'s lowercase-and-strip-punctuation could never do.
 */
export interface MinedCandidate {
	kind: CandidateKind;
	/** Canonical slug for what was meant. The clustering key. */
	label: string;
	/** Verbatim text from the transcript, so the digest can quote rather than paraphrase. */
	text: string;
	/** Why this is durable, in the model's words. Shown when a proposal is borderline. */
	rationale?: string;
	/** The failing command, for `fix` candidates. */
	command?: string;
	/** Short error excerpt, for `fix` candidates. */
	errorExcerpt?: string;
	/** What was done in between, for `fix` candidates. */
	interveningCommands?: string[];
	/** Files changed as part of the fix. */
	editedFiles?: string[];
	/** Tool sequence, for `workflow` candidates. */
	steps?: string[];
}

/** A session reduced to what the miner needs: identity, time, and rendered text. */
export interface MinableSession {
	id: string;
	timestamp: string;
	entries: Array<{ type: string; message?: AgentMessage }>;
}

/**
 * Mines one session. Injectable so the reduce path can be tested without a
 * model, and so a cached result can stand in for a live call.
 */
export type Miner = (session: MinableSession, signal?: AbortSignal) => Promise<MinedCandidate[]>;

/**
 * Chunking exists to fit a session into a context window, so it is sized from
 * the window rather than from a fixed guess.
 *
 * The guess was costing calls. Rendering already strips assistant prose and
 * truncates tool output, which compresses the two real transcripts in this repo
 * from 0.93 MB and 2.26 MB down to 183 KB and 266 KB — about 47k and 68k
 * tokens. A fixed 120k-character chunk cut those into two and three pieces for
 * no reason: on any model with a 200k window each is comfortably one call.
 *
 * One call per session is also better than a cheaper-looking alternative. A
 * chunk boundary is a blind spot — a failure and the fix that resolved it can
 * land on opposite sides of one — so the fewer boundaries inside a session, the
 * more the model can actually see.
 */
const CHUNK_CONTEXT_FRACTION = 0.6;

/** Rough bytes per token. Deliberately conservative; a wrong guess here costs a wasted call. */
const CHARS_PER_TOKEN = 4;

/** Used when a model does not report a usable window. */
const FALLBACK_CHUNK_CHARS = 120_000;

/** Never chunk below this, or a small window would shred a transcript into noise. */
const MIN_CHUNK_CHARS = 40_000;

/**
 * How much rendered transcript to send per call, given the reading model.
 *
 * Only a fraction of the window is used: the instructions, the response, and
 * tokenizer variance all have to fit alongside, and overshooting costs a
 * context-overflow error rather than a slightly worse answer.
 */
export function chunkCharsForModel(model: Pick<Model<any>, "contextWindow">): number {
	const window = model.contextWindow;
	if (!Number.isFinite(window) || window <= 0) return FALLBACK_CHUNK_CHARS;
	const budgetTokens = window * CHUNK_CONTEXT_FRACTION - MAX_RESPONSE_TOKENS;
	return Math.max(MIN_CHUNK_CHARS, Math.floor(budgetTokens * CHARS_PER_TOKEN));
}

/** Tool output kept per call. Errors carry the signal; success output is mostly noise. */
const TOOL_OUTPUT_CHARS = 600;
const TOOL_ERROR_CHARS = 1_500;

/** Response ceiling per chunk. A chunk yielding more than this is noise, not signal. */
const MAX_RESPONSE_TOKENS = 4_000;

/** Candidates accepted from a single chunk, as a guard against a runaway response. */
const MAX_CANDIDATES_PER_CHUNK = 40;

/**
 * Prefix on the message `/learn` injects. Its own digest is persisted like any
 * other user turn, so without this the next run would mine its own output and
 * every proposal would compound its own count.
 */
export const LEARN_DIGEST_MARKER = "[learn-digest]";

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

/** Compact one tool call's arguments — enough to recognise it, not enough to flood the window. */
function renderArgs(args: Record<string, unknown> | undefined): string {
	if (!args) return "";
	const parts: string[] = [];
	for (const [key, value] of Object.entries(args)) {
		if (typeof value === "string") {
			parts.push(`${key}=${value.length > 200 ? `${value.slice(0, 200)}…` : value}`);
		} else if (typeof value === "number" || typeof value === "boolean") {
			parts.push(`${key}=${value}`);
		}
		// Objects and arrays are structural detail the miner does not need.
	}
	return parts.join(" ");
}

/**
 * Render a session as plain text for the model.
 *
 * User turns go in whole and unfiltered — that is the entire point of this
 * rewrite, and any truncation here would quietly reintroduce the recall problem
 * the regex gate had. Assistant prose is dropped: it is the bulk of a
 * transcript and almost none of it is evidence about what the *user* wants.
 * Tool calls are kept because a repeated sequence is a workflow and a
 * failure-then-pass is a fix, and both are things worth proposing.
 */
export function renderTranscript(session: MinableSession): string {
	const lines: string[] = [];

	for (const entry of session.entries) {
		const message = entry.type === "message" ? entry.message : undefined;
		if (!message) continue;

		if (message.role === "user") {
			const text = textOf(message.content);
			// Skip the command's own past output, or proposals compound their counts.
			if (!text || text.startsWith(LEARN_DIGEST_MARKER)) continue;
			lines.push(`USER: ${text}`);
			continue;
		}

		if (message.role === "assistant") {
			for (const block of (message.content ?? []) as unknown[]) {
				if (!isToolCall(block)) continue;
				lines.push(`TOOL: ${block.name}(${renderArgs(block.arguments as Record<string, unknown>)})`);
			}
			continue;
		}

		if (message.role === "toolResult") {
			const output = textOf(message.content);
			if (!output) continue;
			const limit = message.isError ? TOOL_ERROR_CHARS : TOOL_OUTPUT_CHARS;
			const label = message.isError ? "ERROR" : "RESULT";
			lines.push(`${label}: ${output.length > limit ? `${output.slice(0, limit)}…` : output}`);
		}
	}

	return lines.join("\n");
}

/**
 * Split rendered text on line boundaries, so a chunk never cuts a user turn in
 * half. A single turn longer than the budget gets its own oversized chunk
 * rather than being split — losing the second half of a long directive is
 * exactly the failure this rewrite exists to remove.
 */
export function chunkTranscript(text: string, chunkChars = FALLBACK_CHUNK_CHARS): string[] {
	if (text.length <= chunkChars) return text.length > 0 ? [text] : [];

	const chunks: string[] = [];
	let current: string[] = [];
	let size = 0;
	for (const line of text.split("\n")) {
		if (size > 0 && size + line.length + 1 > chunkChars) {
			chunks.push(current.join("\n"));
			current = [];
			size = 0;
		}
		current.push(line);
		size += line.length + 1;
	}
	if (current.length > 0) chunks.push(current.join("\n"));
	return chunks;
}

const MINER_SYSTEM_PROMPT = `You read one coding-session transcript and report durable signals in it.

You are the recall stage of a two-stage pipeline. A later stage counts how often each signal recurs ACROSS sessions and decides what is worth writing down. Your job is to notice and name, not to judge importance and not to count — you are seeing one session and cannot know what repeats.

Report three kinds of thing.

**directive** — the user stating a preference, correction, constraint, or fact about how they want work done. Include these regardless of phrasing. All of these are directives:
- imperative: "always run the tests before pushing"
- corrective: "no, that's not how our error handling works"
- declarative: "we're on bun now", "the API returns snake_case"
- preference stated once, in passing: "I'd rather see this as a table"
Do NOT report task requests ("add a button to the header", "fix the login bug"). A task is what to do now; a directive is how things should be done in general. When a message contains both, report only the directive part.

**fix** — a command that failed and later succeeded, where something in between was the cause. Report the failing command, a short error excerpt, and what changed in between.

**workflow** — a sequence of three or more tool calls that recurs within this session, or that clearly represents a routine procedure (scaffold a file, then register it, then test it).

For every item, produce a "label": a short kebab-case slug naming what was MEANT, not what was said. The label is how occurrences are grouped across sessions, so two different phrasings of the same underlying point MUST get the same label.
- "we're on bun now" → use-bun-not-npm
- "stop using npm install" → use-bun-not-npm
- "pnpm isn't what we use here" → use-bun-not-npm
Keep labels general enough to collide when they mean the same thing, specific enough not to collide when they do not. Prefer 2-5 words.

Output STRICT JSON, no markdown fence, no prose:
{"candidates":[{"kind":"directive","label":"use-bun-not-npm","text":"<verbatim quote>","rationale":"<one clause on why it is durable>"}]}

For fix items add: "command", "errorExcerpt", "interveningCommands" (array), "editedFiles" (array).
For workflow items add: "steps" (array of tool names in order).

Report nothing rather than padding. An empty list is a correct answer for a session that taught nothing: {"candidates":[]}`;

/**
 * Pull the JSON object out of a model response.
 *
 * Models fence JSON even when told not to, and occasionally prepend a sentence.
 * Scanning for the outermost braces is more forgiving than trusting the format
 * and cheaper than a repair pass — and a chunk whose response cannot be parsed
 * is skipped, never fatal, because one bad chunk should not lose a whole run.
 */
export function parseCandidates(response: string): MinedCandidate[] {
	const start = response.indexOf("{");
	const end = response.lastIndexOf("}");
	if (start < 0 || end <= start) return [];

	let parsed: unknown;
	try {
		parsed = JSON.parse(response.slice(start, end + 1));
	} catch {
		return [];
	}

	const raw = (parsed as { candidates?: unknown })?.candidates;
	if (!Array.isArray(raw)) return [];

	const out: MinedCandidate[] = [];
	for (const item of raw.slice(0, MAX_CANDIDATES_PER_CHUNK)) {
		if (!item || typeof item !== "object") continue;
		const candidate = item as Record<string, unknown>;
		const kind = candidate.kind;
		if (kind !== "directive" && kind !== "fix" && kind !== "workflow") continue;

		const label = typeof candidate.label === "string" ? candidate.label.trim().toLowerCase() : "";
		const text = typeof candidate.text === "string" ? candidate.text.trim() : "";
		// A candidate with no label cannot be grouped, and one with no text cannot
		// be quoted back — either way there is nothing to show the reader.
		if (!label || !text) continue;

		const strings = (value: unknown): string[] | undefined =>
			Array.isArray(value) ? value.filter((v): v is string => typeof v === "string").slice(0, 12) : undefined;

		out.push({
			kind,
			label,
			text,
			rationale: typeof candidate.rationale === "string" ? candidate.rationale.trim() : undefined,
			command: typeof candidate.command === "string" ? candidate.command : undefined,
			errorExcerpt: typeof candidate.errorExcerpt === "string" ? candidate.errorExcerpt.slice(0, 400) : undefined,
			interveningCommands: strings(candidate.interveningCommands),
			editedFiles: strings(candidate.editedFiles),
			steps: strings(candidate.steps),
		});
	}
	return out;
}

export interface MinerDeps {
	model: Model<any>;
	apiKey?: string;
	headers?: Record<string, string>;
}

/**
 * Build the real miner: one model call per chunk of one session.
 *
 * Chunks are mined sequentially rather than in parallel. A cold-cache run is
 * already the expensive path, and firing every chunk of every session at once
 * is how you trip a provider rate limit on exactly the run that has the most to
 * do.
 */
export function createLlmMiner(deps: MinerDeps): Miner {
	const chunkChars = chunkCharsForModel(deps.model);
	return async (session, signal) => {
		const chunks = chunkTranscript(renderTranscript(session), chunkChars);
		const candidates: MinedCandidate[] = [];

		for (const chunk of chunks) {
			if (signal?.aborted) break;

			const response = await completeSimple(
				deps.model,
				{
					systemPrompt: MINER_SYSTEM_PROMPT,
					messages: [{ role: "user", content: [{ type: "text", text: chunk }], timestamp: Date.now() }],
				},
				{ maxTokens: MAX_RESPONSE_TOKENS, signal, apiKey: deps.apiKey, headers: deps.headers },
			);

			if (response.stopReason === "error") {
				throw new Error(response.errorMessage || "miner call failed");
			}

			const text = response.content
				.filter((c): c is { type: "text"; text: string } => c.type === "text")
				.map((c) => c.text)
				.join("\n");
			candidates.push(...parseCandidates(text));
		}

		return candidates;
	};
}
