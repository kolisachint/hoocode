/**
 * The model call G4 was designed around and never had.
 *
 * `trigger-eval.ts` takes its judge as a parameter so scoring stays testable
 * without a model, and nothing in the tree ever passed one — so every G4 run
 * has reported `not-run` since it was written. This is that judge.
 *
 * It is deliberately in its own module rather than inside `trigger-eval.ts`:
 * the eval is pure and testable, this reaches the network, and the same
 * separation lets the agent-selection eval reuse the judge without pulling in
 * the plugin gate machinery.
 *
 * ## Why one call for all prompts
 *
 * The judge sees every candidate and every prompt at once. Per-prompt calls
 * would be cleaner to reason about, but the question being scored is
 * comparative — "which of these fires" — and batching keeps the candidate list
 * identical across prompts, which is the thing that must not vary. It also
 * makes the cost proportional to the corpus rather than to the case count.
 */

import { completeSimple, type Model } from "@kolisachint/hoocode-ai";
import type { TriggerCandidate, TriggerJudge, TriggerJudgeVerdict } from "./trigger-eval.js";

/** Description characters per candidate. The opening states the trigger; past that it is filler. */
const DESCRIPTION_CHARS = 600;
const MAX_RESPONSE_TOKENS = 4_000;

const JUDGE_SYSTEM_PROMPT = `You simulate how a coding agent picks a capability.

You are given numbered CAPABILITIES (name and description) and numbered PROMPTS. For each prompt, answer with the ONE capability an agent would reach for, or null when none of them fits and the agent should just do the work itself.

Rules:
- Judge ONLY from the descriptions. Do not use knowledge about what these names usually mean elsewhere.
- Pick the single best fit. When two fit, pick the one whose description names the prompt's situation more specifically.
- Answer null when no description covers the prompt. Do not stretch a description to make it fit; a wrong pick and a null are both wrong, but pretending coverage hides the real failure.
- Answer every prompt exactly once, in the order given.

Output STRICT JSON, no markdown fence, no prose:
{"verdicts":[{"prompt":0,"capability":"explore"},{"prompt":1,"capability":null}]}`;

function buildPrompt(candidates: readonly TriggerCandidate[], prompts: readonly string[]): string {
	const lines: string[] = ["CAPABILITIES:"];
	for (const [i, candidate] of candidates.entries()) {
		lines.push(`${i}. ${candidate.name} — ${candidate.description.slice(0, DESCRIPTION_CHARS)}`);
	}
	lines.push("", "PROMPTS:");
	for (const [i, prompt] of prompts.entries()) {
		lines.push(`${i}. ${prompt}`);
	}
	return lines.join("\n");
}

/**
 * Read the verdict list back, positionally.
 *
 * Returns one entry per prompt no matter what came back: a missing or
 * unparseable row becomes null (read as "nothing fired") rather than shifting
 * every later verdict onto the wrong prompt. `runTriggerEval` rejects a
 * length mismatch outright, so the alternative to filling the gaps is
 * discarding the whole run — and a hallucinated capability name is a real
 * signal about the candidate list, not a reason to throw the batch away.
 */
export function parseTriggerVerdicts(
	response: string,
	candidates: readonly TriggerCandidate[],
	promptCount: number,
): TriggerJudgeVerdict[] {
	const verdicts: TriggerJudgeVerdict[] = new Array(promptCount).fill(null);
	const start = response.indexOf("{");
	const end = response.lastIndexOf("}");
	if (start < 0 || end <= start) return verdicts;

	let parsed: unknown;
	try {
		parsed = JSON.parse(response.slice(start, end + 1));
	} catch {
		return verdicts;
	}

	const rows = (parsed as { verdicts?: unknown })?.verdicts;
	if (!Array.isArray(rows)) return verdicts;

	const known = new Set(candidates.map((c) => c.name));
	for (const row of rows) {
		if (!row || typeof row !== "object") continue;
		const entry = row as Record<string, unknown>;
		const index = typeof entry.prompt === "number" ? entry.prompt : Number.NaN;
		if (!Number.isInteger(index) || index < 0 || index >= promptCount) continue;
		const capability = entry.capability;
		if (capability === null || capability === undefined) {
			verdicts[index] = null;
			continue;
		}
		// A name that is not on the candidate list is a hallucination. Recording it
		// as null keeps it wrong without letting it masquerade as a real pick.
		verdicts[index] = typeof capability === "string" && known.has(capability) ? capability : null;
	}
	return verdicts;
}

export interface TriggerJudgeDeps {
	model: Model<any>;
	apiKey?: string;
	headers?: Record<string, string>;
	signal?: AbortSignal;
}

/** A {@link TriggerJudge} backed by a real model. */
export function createLlmTriggerJudge(deps: TriggerJudgeDeps): TriggerJudge {
	return async ({ candidates, prompts }) => {
		if (candidates.length === 0 || prompts.length === 0) return prompts.map(() => null);

		const response = await completeSimple(
			deps.model,
			{
				systemPrompt: JUDGE_SYSTEM_PROMPT,
				messages: [
					{
						role: "user",
						content: [{ type: "text", text: buildPrompt(candidates, prompts) }],
						timestamp: Date.now(),
					},
				],
			},
			{ maxTokens: MAX_RESPONSE_TOKENS, signal: deps.signal, apiKey: deps.apiKey, headers: deps.headers },
		);

		if (response.stopReason === "error") {
			// Thrown, not swallowed: runTriggerEval turns this into `not-run` with
			// the reason attached, which is the honest outcome. Returning nulls
			// would score every case as "nothing fired" and read like a real result.
			throw new Error(response.errorMessage || "trigger judge call failed");
		}

		const text = response.content
			.filter((c): c is { type: "text"; text: string } => c.type === "text")
			.map((c) => c.text)
			.join("\n");
		return parseTriggerVerdicts(text, candidates, prompts.length);
	};
}
