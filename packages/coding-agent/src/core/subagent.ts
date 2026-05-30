/**
 * Subagent definitions: modes, per-mode tool allowlists, and system prompts.
 *
 * Subagents run as isolated `hoocode` child processes managed by SubagentPool.
 * This module is the single source of truth for what each mode is allowed to do
 * (tool allowlist) and the clean system prompt it runs with.
 */

import { EMBEDDED_SUBAGENT_PROMPTS } from "../init-templates.generated.js";

export type SubagentMode = "explore" | "edit" | "test" | "fix" | "review" | "doc";

export const SUBAGENT_MODES: readonly SubagentMode[] = ["explore", "edit", "test", "fix", "review", "doc"];

/** Tool allowlist per mode. Read-only modes deliberately omit edit/write. */
export const MODE_TOOLS: Record<SubagentMode, string[]> = {
	explore: ["read", "grep", "find", "ls", "bash"],
	edit: ["read", "edit", "write", "grep", "find", "ls", "bash"],
	test: ["read", "bash", "grep", "find", "ls"],
	fix: ["read", "edit", "write", "bash", "grep", "find", "ls"],
	review: ["read", "grep", "find", "ls", "bash"],
	doc: ["read", "write", "edit", "grep", "find", "ls", "bash"],
};

/** Return the clean, minimal system prompt for a subagent mode. */
export function getSubagentSystemPrompt(mode: SubagentMode): string {
	const prompt = EMBEDDED_SUBAGENT_PROMPTS[mode];
	if (!prompt) {
		throw new Error(`No system prompt template for subagent mode "${mode}"`);
	}
	return prompt;
}
