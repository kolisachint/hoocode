/**
 * The naming pass: decide which occurrences are the same point.
 *
 * This is the stage the pipeline was missing. Mining is a map over sessions and
 * counting is a reduce over labels, but nothing sat in between to agree on what
 * the labels *are*. The miner was asked to produce them from inside a single
 * session — to hit a shared vocabulary it had never seen — and on a real corpus
 * it agreed with itself 3 times in 188 candidates. `use-bun-not-npm` and
 * `prefer-bun-over-npm` are the same rule and never met.
 *
 * So naming happens once, with everything visible at the same time. That is a
 * different question than the miner was being asked: not "what is a good name
 * for this sentence" but "which of these sentences are the same point", which
 * is only answerable in the presence of the others.
 *
 * Two properties matter more than elegance here:
 *
 * - **Stability across runs.** State keys are `directive:<label>`, so a label
 *   that drifts between runs silently breaks suppression — every proposal you
 *   already decided on comes back forever. The labels already on record are
 *   therefore sent as a preferred vocabulary, and reusing one is the first
 *   instruction the model gets.
 * - **Degrading in order.** A window too large for one call is processed in
 *   sequence, with the names assigned so far carried into the next call. That is
 *   worse than seeing everything at once, but it is worse in a predictable
 *   direction: later candidates join earlier clusters rather than starting
 *   rival ones.
 */

import { completeSimple, type Model } from "@kolisachint/hoocode-ai";
import type { MinedCandidate } from "./mine.js";

/**
 * Candidates named per call.
 *
 * Sized so a typical window is one call: 200 quotes at ~120 characters is well
 * inside a small model's window with room for the reply. Past that, clustering
 * quality would degrade anyway — a list nobody can hold in mind is one nobody
 * names consistently.
 */
const MAX_CANDIDATES_PER_CALL = 200;

/** Known labels offered as vocabulary. Enough to cover a real state file, short of flooding the prompt. */
const MAX_KNOWN_LABELS = 150;

/**
 * Trim the vocabulary to what fits, keeping both ends.
 *
 * The list is ordered: labels already on record first, then names invented
 * earlier in this run. Those are two different anchors — the first keeps the
 * bookmark matching across runs, the second keeps a split window from starting
 * rival names for one point — and taking a plain prefix silently drops the
 * second exactly when batching makes it necessary.
 */
export function trimVocabulary(labels: string[], max = MAX_KNOWN_LABELS): string[] {
	if (labels.length <= max) return labels;
	const head = Math.ceil(max / 2);
	return [...labels.slice(0, head), ...labels.slice(-(max - head))];
}

/** Quote characters sent per candidate. A directive is identifiable long before this. */
const QUOTE_CHARS = 240;

const MAX_RESPONSE_TOKENS = 4_000;

/** One candidate to be named, with the identity the caller needs to put the label back. */
export interface ClusterInput {
	/** Caller's handle for this candidate; returned untouched. */
	id: number;
	kind: MinedCandidate["kind"];
	text: string;
}

/**
 * Assign a label to each input. Missing entries are left for the caller to
 * handle; a clusterer may legitimately decline to name something.
 */
export type Clusterer = (
	inputs: ClusterInput[],
	knownLabels: string[],
	signal?: AbortSignal,
) => Promise<Map<number, string>>;

const CLUSTER_SYSTEM_PROMPT = `You group occurrences from coding sessions by what they MEAN, and give each group a name.

You are given numbered ITEMS. Each is something a user said, or something that happened, across many sessions. Different sessions phrase the same point differently — your job is to recognise that and name the point once.

Rules, in order of importance:

1. If a label in KNOWN LABELS already names the point, reuse it EXACTLY. These are names already on record; reusing one is how a proposal the reader already decided on stays decided. Do not invent a synonym for a label that exists.
2. Items meaning the same thing MUST get the same label, even when the wording shares no words.
   - "we're on bun now" / "stop using npm install" / "pnpm isn't what we use here" → use-bun-not-npm
   - "never force push" / "don't rewrite shared history" → never-force-push
3. Items meaning different things MUST NOT share a label, even when the wording is similar. "doc tools off by default" and "network tools off by default" are the same shape and different rules.
4. A label is a short kebab-case slug naming the point, 2-5 words. Name the point, not the session it came from.
5. Never group across kinds. A directive ("how work should be done") and a request ("do this piece of work") are never the same item, even when they are about the same subject.

Output STRICT JSON, no markdown fence, no prose. One entry per item, using the item's number:
{"labels":[{"id":1,"label":"use-bun-not-npm"},{"id":2,"label":"use-bun-not-npm"}]}

Every item gets exactly one label. An item that means something no other item means still gets its own label — a group of one is a normal answer.`;

/** Render the numbered list the prompt describes. */
export function renderClusterRequest(inputs: ClusterInput[], knownLabels: string[]): string {
	const lines: string[] = [];

	if (knownLabels.length > 0) {
		lines.push("KNOWN LABELS (reuse exactly when one fits):");
		for (const label of trimVocabulary(knownLabels)) lines.push(`- ${label}`);
		lines.push("");
	}

	lines.push("ITEMS:");
	for (const input of inputs) {
		const quote = input.text.length > QUOTE_CHARS ? `${input.text.slice(0, QUOTE_CHARS)}…` : input.text;
		lines.push(`${input.id}. [${input.kind}] ${quote.replace(/\s+/g, " ")}`);
	}

	return lines.join("\n");
}

/**
 * Read the label assignments out of a model response.
 *
 * Same forgiving parse as the miner: models fence JSON they were told not to
 * fence, and one unparseable response should cost the run its grouping, not its
 * life. An id the caller never asked about is dropped rather than trusted.
 */
export function parseClusterLabels(response: string, known: Set<number>): Map<number, string> {
	const out = new Map<number, string>();
	const start = response.indexOf("{");
	const end = response.lastIndexOf("}");
	if (start < 0 || end <= start) return out;

	let parsed: unknown;
	try {
		parsed = JSON.parse(response.slice(start, end + 1));
	} catch {
		return out;
	}

	const raw = (parsed as { labels?: unknown })?.labels;
	if (!Array.isArray(raw)) return out;

	for (const item of raw) {
		if (!item || typeof item !== "object") continue;
		const entry = item as Record<string, unknown>;
		const id = typeof entry.id === "number" ? entry.id : Number.NaN;
		const label = typeof entry.label === "string" ? entry.label.trim().toLowerCase() : "";
		if (!Number.isInteger(id) || !known.has(id) || !label) continue;
		// Normalized to the slug shape the state file keys on, so a model that
		// answers "Use Bun Not Npm" does not fork the vocabulary on punctuation.
		out.set(
			id,
			label
				.replace(/[^a-z0-9]+/g, "-")
				.replace(/^-|-$/g, "")
				.slice(0, 60),
		);
	}
	return out;
}

export interface ClustererDeps {
	model: Model<any>;
	apiKey?: string;
	headers?: Record<string, string>;
}

export function createLlmClusterer(deps: ClustererDeps): Clusterer {
	return async (inputs, knownLabels, signal) => {
		const assigned = new Map<number, string>();
		// Labels invented in an earlier batch join the vocabulary for the next, so
		// a split window still converges on one name per point.
		const vocabulary = [...knownLabels];

		for (let offset = 0; offset < inputs.length; offset += MAX_CANDIDATES_PER_CALL) {
			if (signal?.aborted) break;
			const batch = inputs.slice(offset, offset + MAX_CANDIDATES_PER_CALL);

			const response = await completeSimple(
				deps.model,
				{
					systemPrompt: CLUSTER_SYSTEM_PROMPT,
					messages: [
						{
							role: "user",
							content: [{ type: "text", text: renderClusterRequest(batch, vocabulary) }],
							timestamp: Date.now(),
						},
					],
				},
				{ maxTokens: MAX_RESPONSE_TOKENS, signal, apiKey: deps.apiKey, headers: deps.headers },
			);

			if (response.stopReason === "error") {
				throw new Error(response.errorMessage || "clustering call failed");
			}

			const text = response.content
				.filter((block): block is { type: "text"; text: string } => block.type === "text")
				.map((block) => block.text)
				.join("");

			for (const [id, label] of parseClusterLabels(text, new Set(batch.map((item) => item.id)))) {
				assigned.set(id, label);
				if (!vocabulary.includes(label)) vocabulary.push(label);
			}
		}

		return assigned;
	};
}

/**
 * Fallback naming for a candidate the clusterer did not label.
 *
 * A run whose clustering call failed should still propose something, so an
 * unlabelled candidate falls back to a slug of its own text. That groups
 * identical wording and nothing else — the behaviour the pipeline had before
 * clustering existed, which is the right floor to fail to.
 */
export function fallbackLabel(text: string): string {
	return (
		text
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, "-")
			.replace(/^-|-$/g, "")
			.slice(0, 60) || "unlabelled"
	);
}
