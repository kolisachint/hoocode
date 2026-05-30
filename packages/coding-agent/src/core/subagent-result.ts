/**
 * Writes a subagent's `result.json` audit file.
 *
 * When a subagent runs as a spawned child process (`--task-id <id>`), the parent
 * SubagentPool verifies `.hoocode/agents/<task_id>/result.json` against a fixed
 * schema (see OutputVerifier). This module derives that file deterministically
 * from the finished session so subagents never have to write it themselves.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import type { AssistantMessage } from "@kolisachint/hoocode-ai";
import { CONFIG_DIR_NAME } from "../config.js";

export interface SubagentUsage {
	input: number;
	output: number;
	cacheRead: number;
	cacheWrite: number;
	cost: number;
}

export interface SubagentResultFile {
	summary: string;
	files_changed: string[];
	confidence: number;
	status: "complete" | "partial" | "failed";
	/** Token and cost usage for the subagent session (extra field; ignored by the verifier). */
	usage?: SubagentUsage;
}

/** Tool names that mutate files. Their `path`/`file_path` argument is a changed file. */
const MUTATING_TOOLS = new Set(["edit", "write"]);

/** Collect distinct file paths touched by edit/write tool calls across the session. */
function collectChangedFiles(messages: readonly AgentMessage[]): string[] {
	const files = new Set<string>();
	for (const message of messages) {
		if (message.role !== "assistant") continue;
		for (const content of (message as AssistantMessage).content) {
			if (content.type !== "toolCall") continue;
			if (!MUTATING_TOOLS.has(content.name)) continue;
			const args = content.arguments ?? {};
			const path =
				typeof args.path === "string" ? args.path : typeof args.file_path === "string" ? args.file_path : null;
			if (path) files.add(path);
		}
	}
	return [...files];
}

/** Last assistant text, trimmed, as the summary. */
function deriveSummary(messages: readonly AgentMessage[]): string {
	for (let i = messages.length - 1; i >= 0; i--) {
		const message = messages[i];
		if (message.role !== "assistant") continue;
		const assistant = message as AssistantMessage;
		let text = "";
		for (const content of assistant.content) {
			if (content.type === "text") text += content.text;
		}
		const trimmed = text.trim();
		if (trimmed) return trimmed;
	}
	return "";
}

/** Derive the terminal status from the final assistant message. */
function deriveStatus(messages: readonly AgentMessage[]): "complete" | "failed" {
	const last = messages[messages.length - 1];
	if (last?.role === "assistant") {
		const assistant = last as AssistantMessage;
		if (assistant.stopReason === "error" || assistant.stopReason === "aborted") return "failed";
	}
	return "complete";
}

/**
 * Build the `result.json` payload for a finished subagent session.
 *
 * The verifier requires a non-empty summary and confidence >= 0.5, so a
 * successful run with no assistant text still yields a usable summary.
 */
export function buildSubagentResult(messages: readonly AgentMessage[], usage?: SubagentUsage): SubagentResultFile {
	const status = deriveStatus(messages);
	const summary =
		deriveSummary(messages) || (status === "complete" ? "Task completed with no textual summary." : "Task failed.");
	return {
		summary,
		files_changed: collectChangedFiles(messages),
		confidence: status === "complete" ? 0.9 : 0.5,
		status,
		usage,
	};
}

/** Write `result.json` for a task. Best-effort: never throws. */
export function writeSubagentResult(cwd: string, taskId: string, result: SubagentResultFile): void {
	const path = join(cwd, CONFIG_DIR_NAME, "agents", taskId, "result.json");
	try {
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, JSON.stringify(result, null, 2));
	} catch {
		// Audit file is best-effort; the parent treats a missing file as a failure.
	}
}
