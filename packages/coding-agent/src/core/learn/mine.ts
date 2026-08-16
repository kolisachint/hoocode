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
 * What the model does *not* do here is name or count. It used to emit a label
 * per occurrence — its own canonical name for what was meant — and the reduce
 * step grouped on exact label equality. That cannot work from inside one
 * session: the model is asked to hit a shared vocabulary it has never seen, and
 * on a real corpus it agreed with itself 3 times out of 188. Naming now happens
 * once, globally, in `cluster.ts`, where every candidate is visible at the same
 * time. Counting stays in `reduce.ts`, where it always belonged.
 *
 * Leaving labels out also makes the cache model-independent. A cached candidate
 * used to carry a label frozen at mining time, so changing the `fast` tier
 * forked the vocabulary permanently: old sessions and new ones named the same
 * thing differently, and neither side reached the repeat threshold.
 */

import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import type { Model, TextContent, ToolCall } from "@kolisachint/hoocode-ai";
import { completeSimple } from "@kolisachint/hoocode-ai";

/** What kind of thing the model noticed. */
export type CandidateKind = "directive" | "fix" | "request";

/**
 * One occurrence, as reported by the model reading a single session.
 *
 * Deliberately unnamed. What was *meant* is only decidable against everything
 * else that was said, and this stage sees one session, so it reports what it
 * saw and leaves grouping to `cluster.ts`. This is also the shape that goes in
 * the cache, which is why nothing model-specific may live on it.
 */
export interface MinedCandidate {
	kind: CandidateKind;
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
}

/** A candidate once the global naming pass has decided what to call it. */
export interface LabelledCandidate extends MinedCandidate {
	/** Canonical slug for what was meant. The clustering key. */
	label: string;
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

/** Error output kept per call. Errors carry the signal; success output is dropped entirely. */
const TOOL_ERROR_CHARS = 1_500;

/**
 * Shortest literal run of a slash-command body that identifies a replay.
 *
 * Long enough that a user cannot type it by accident, short enough to survive a
 * template whose placeholders are densely packed.
 */
const REPLAY_FINGERPRINT_CHARS = 40;

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

/** Collapse whitespace so a quote survives the wrapping a markdown source imposes on it. */
function normalizeForMatch(text: string): string {
	return text.replace(/\s+/g, " ").trim().toLowerCase();
}

/**
 * Literal runs from slash-command bodies, used to recognise a replayed expansion.
 *
 * A `user`-type slash command is persisted as an ordinary user message holding
 * the whole template body, with nothing to mark it as machinery. Read back off
 * disk it is indistinguishable from something the user typed — and it is the
 * most repeated text in a real corpus, because running `/pr` thirty times
 * writes the same two thousand characters thirty times. Mining it produces
 * directives the user never stated, at counts that look exactly like organic
 * repetition.
 *
 * Detection is retroactive on purpose. A provenance flag written at turn time
 * would be exact, but it would only help sessions recorded after it shipped,
 * leaving the existing corpus contaminated for months. Matching against the
 * command bodies still on disk fixes the history that already exists. The gap
 * is a template that has since been deleted; that case wants the flag, and is
 * the reason to add one later.
 */
export function replayFingerprints(templates: Array<{ content: string }>): string[] {
	const out: string[] = [];
	for (const template of templates) {
		// Split on the placeholders that argument substitution rewrites, leaving the
		// literal text that survives every expansion.
		const segments = template.content.split(/\$(?:\d+|ARGUMENTS|\*)/);
		let longest = "";
		for (const segment of segments) {
			const normalized = normalizeForMatch(segment);
			if (normalized.length > longest.length) longest = normalized;
		}
		if (longest.length >= REPLAY_FINGERPRINT_CHARS) out.push(longest);
	}
	return out;
}

/** True when a user turn is the body of a slash command rather than something typed. */
export function isReplayedTurn(text: string, fingerprints: string[]): boolean {
	if (fingerprints.length === 0) return false;
	const normalized = normalizeForMatch(text);
	if (normalized.length < REPLAY_FINGERPRINT_CHARS) return false;
	return fingerprints.some((fingerprint) => normalized.includes(fingerprint));
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
 * User turns the user actually typed go in whole and unfiltered — any
 * truncation there would quietly reintroduce the recall problem the old regex
 * gate had. What does not go in is text the user's tooling replayed: its own
 * past digests, and slash-command bodies.
 *
 * Assistant prose is dropped: it is the bulk of a transcript and almost none of
 * it is evidence about what the *user* wants. Tool calls are kept, because a
 * failure-then-pass is a fix. Successful tool output is dropped: it is a file
 * or a command's stdout, not a statement by anyone, and feeding it to a miner
 * looking for directives yields lines lifted out of plan files and configs
 * attributed to the user.
 */
export function renderTranscript(session: MinableSession, fingerprints: string[] = []): string {
	const lines: string[] = [];

	for (const entry of session.entries) {
		const message = entry.type === "message" ? entry.message : undefined;
		if (!message) continue;

		if (message.role === "user") {
			const text = textOf(message.content);
			// Skip the command's own past output, or proposals compound their counts.
			if (!text || text.startsWith(LEARN_DIGEST_MARKER)) continue;
			if (isReplayedTurn(text, fingerprints)) continue;
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

		if (message.role === "toolResult" && message.isError) {
			const output = textOf(message.content);
			if (!output) continue;
			lines.push(`ERROR: ${output.length > TOOL_ERROR_CHARS ? `${output.slice(0, TOOL_ERROR_CHARS)}…` : output}`);
		}
	}

	return lines.join("\n");
}

/** Everything the user actually said in a session, normalized, for checking quotes against. */
export function spokenText(session: MinableSession, fingerprints: string[] = []): string {
	const parts: string[] = [];
	for (const entry of session.entries) {
		const message = entry.type === "message" ? entry.message : undefined;
		if (!message || message.role !== "user") continue;
		const text = textOf(message.content);
		if (!text || text.startsWith(LEARN_DIGEST_MARKER)) continue;
		if (isReplayedTurn(text, fingerprints)) continue;
		parts.push(text);
	}
	return normalizeForMatch(parts.join("\n"));
}

/**
 * Drop candidates whose quote cannot be found in what the user said.
 *
 * The miner is told to quote verbatim and the digest renders every quote inside
 * quotation marks, but on a real corpus a third of them appear nowhere in the
 * session: paraphrases, merged sentences, and lines lifted out of tool output.
 * A quote that cannot be located is evidence that cannot be shown, and a
 * proposal the reader cannot check is worse than one that was never made.
 *
 * Whitespace is normalized before comparing, because a directive written in a
 * markdown file arrives wrapped across lines and the model unwraps it.
 */
export function verifyCandidates(candidates: MinedCandidate[], spoken: string): MinedCandidate[] {
	// Normalized again rather than trusting the caller: the check is a substring
	// test, and one un-normalized argument would silently reject everything.
	const haystack = normalizeForMatch(spoken);
	if (!haystack) return [];
	return candidates.filter((candidate) => {
		// A fix is evidenced by commands and errors, not by something the user said.
		if (candidate.kind === "fix") return true;
		return haystack.includes(normalizeForMatch(candidate.text));
	});
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
A directive is how things should be done in general. What to do right now is a **request** — see below — not a directive. When a message contains both, report the directive part here.

**fix** — a command that failed and later succeeded, where something in between was the cause. Report the failing command, a short error excerpt, and what changed in between.

**request** — the user asking for a piece of work by name: "open a release PR", "run the full check and fix what it finds", "give me a demo of X". Report the request as they phrased it. A request repeated across sessions is a slash command waiting to be written, which is why it is worth reporting even though it is not a rule.

A message can contain both a request and a directive — "open a release PR, and remember to stage only your own files" is one of each. Report both, separately.

Quote "text" VERBATIM from the transcript. Do not paraphrase, merge two sentences, or tidy the wording: a quote that cannot be found in the session is discarded, because the reader is shown it in quotation marks and has to be able to check it.

Do not name or group anything. A later stage sees every session at once and decides what counts as the same point; from inside one session you cannot know.

Output STRICT JSON, no markdown fence, no prose:
{"candidates":[{"kind":"directive","text":"<verbatim quote>","rationale":"<one clause on why it is durable>"}]}

For fix items add: "command", "errorExcerpt", "interveningCommands" (array), "editedFiles" (array).

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
		if (kind !== "directive" && kind !== "fix" && kind !== "request") continue;

		const text = typeof candidate.text === "string" ? candidate.text.trim() : "";
		// Nothing to quote back means nothing to show the reader.
		if (!text) continue;

		const strings = (value: unknown): string[] | undefined =>
			Array.isArray(value) ? value.filter((v): v is string => typeof v === "string").slice(0, 12) : undefined;

		out.push({
			kind,
			text,
			rationale: typeof candidate.rationale === "string" ? candidate.rationale.trim() : undefined,
			command: typeof candidate.command === "string" ? candidate.command : undefined,
			errorExcerpt: typeof candidate.errorExcerpt === "string" ? candidate.errorExcerpt.slice(0, 400) : undefined,
			interveningCommands: strings(candidate.interveningCommands),
			editedFiles: strings(candidate.editedFiles),
		});
	}
	return out;
}

export interface MinerDeps {
	model: Model<any>;
	apiKey?: string;
	headers?: Record<string, string>;
	/**
	 * Literal runs from the slash-command bodies in force, from
	 * `replayFingerprints`. User turns matching one are machinery replaying
	 * itself, not the user speaking.
	 */
	replayFingerprints?: string[];
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
	const fingerprints = deps.replayFingerprints ?? [];
	return async (session, signal) => {
		const chunks = chunkTranscript(renderTranscript(session, fingerprints), chunkChars);
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

		// Verify against the whole session rather than the chunk that produced the
		// candidate: a quote can legitimately straddle a chunk boundary, and a
		// dropped-for-being-unfindable verdict has to mean unfindable anywhere.
		return verifyCandidates(candidates, spokenText(session, fingerprints));
	};
}
