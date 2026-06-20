/**
 * Validated per-tool extractive compression prompts.
 *
 * These prompts were validated against Qwen3-4B (see
 * docs/local-executor-routing.md). Only `bash` output compresses safely: its
 * verbose output is mostly low-value noise (progress/passing lines) around a
 * few load-bearing facts (errors, counts, exit codes). `read` was removed after
 * measurement showed ~0% reduction on real source code (every line is a
 * keep-line). Fact-list outputs (grep/find/ls) are intentionally excluded:
 * every line is a distinct fact, so compression drops matches.
 *
 * The prompt is extractive (keep identifiers, drop only redundant filler),
 * not abstractive (rewrite in prose), which is what made retention reliable.
 */

export const TOOL_RESULT_SYSTEM_PROMPT =
	"You compress tool output for another AI to consume. Be extractive: keep exact identifiers, " +
	"never paraphrase facts, and never invent anything. Output only the compressed result.";

const BASH_PROMPT =
	"Compress this command output. Keep ONLY: the command, every error/warning with its file:line:col " +
	"and code, and any final counts/timings/exit code. Drop progress bars, info lines, and passing/OK " +
	"lines. Keep all numbers and paths exactly. No prose.";

const TOOL_RESULT_PROMPTS: Record<string, string> = {
	bash: BASH_PROMPT,
};

/** Get the extractive compression prompt for a tool, or undefined if not compressible. */
export function getToolResultPrompt(toolName: string): string | undefined {
	return TOOL_RESULT_PROMPTS[toolName];
}

/** Build the full prompt text for compressing a tool result. */
export function buildToolResultPrompt(toolName: string, output: string): string | undefined {
	const instruction = getToolResultPrompt(toolName);
	if (!instruction) return undefined;
	return `${instruction}\n\n${output}`;
}

/**
 * Remove reasoning-model `<think>...</think>` blocks (including empty ones that
 * Qwen3 emits even under `/no_think`) and trim the result. Reasoning models
 * leave these tags in the text stream; they must not leak into context.
 */
export function stripThinkTags(text: string): string {
	return text
		.replace(/<think>[\s\S]*?<\/think>/g, "")
		.replace(/<\/?think>/g, "")
		.trim();
}
