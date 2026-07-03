/**
 * Pure helpers for AgentSession reporting and export.
 *
 * These functions derive statistics, context-usage, forkable user messages, and
 * JSONL exports from session state without touching the live agent or extension
 * runner. AgentSession delegates to them so the class stays focused on lifecycle
 * and event wiring.
 */

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import { calculateContextTokens, estimateContextTokens } from "@kolisachint/hoocode-agent-core";
import type { AssistantMessage, Model } from "@kolisachint/hoocode-ai";
import type { ContextUsage } from "./extensions/index.js";
import type { SessionManager } from "./session-manager.js";
import { CURRENT_SESSION_VERSION, getLatestCompactionEntry, type SessionHeader } from "./session-manager.js";

/** Session statistics for /session command */
export interface SessionStats {
	sessionFile: string | undefined;
	sessionId: string;
	userMessages: number;
	assistantMessages: number;
	toolCalls: number;
	toolResults: number;
	totalMessages: number;
	tokens: {
		input: number;
		output: number;
		cacheRead: number;
		cacheWrite: number;
		total: number;
	};
	cost: number;
	contextUsage?: ContextUsage;
}

/** Extract concatenated text from a user message content value. */
export function extractUserMessageText(content: string | Array<{ type: string; text?: string }>): string {
	if (typeof content === "string") return content;
	if (Array.isArray(content)) {
		return content
			.filter((c): c is { type: "text"; text: string } => c.type === "text")
			.map((c) => c.text)
			.join("");
	}
	return "";
}

/** Compute aggregate statistics (message counts, token usage, cost) for a session. */
export function computeSessionStats(params: {
	messages: AgentMessage[];
	sessionFile: string | undefined;
	sessionId: string;
	contextUsage: ContextUsage | undefined;
}): SessionStats {
	const { messages, sessionFile, sessionId, contextUsage } = params;
	const userMessages = messages.filter((m) => m.role === "user").length;
	const assistantMessages = messages.filter((m) => m.role === "assistant").length;
	const toolResults = messages.filter((m) => m.role === "toolResult").length;

	let toolCalls = 0;
	let totalInput = 0;
	let totalOutput = 0;
	let totalCacheRead = 0;
	let totalCacheWrite = 0;
	let totalCost = 0;

	for (const message of messages) {
		if (message.role === "assistant") {
			const assistantMsg = message as AssistantMessage;
			toolCalls += assistantMsg.content.filter((c) => c.type === "toolCall").length;
			totalInput += assistantMsg.usage.input;
			totalOutput += assistantMsg.usage.output;
			totalCacheRead += assistantMsg.usage.cacheRead;
			totalCacheWrite += assistantMsg.usage.cacheWrite;
			totalCost += assistantMsg.usage.cost.total;
		}
	}

	return {
		sessionFile,
		sessionId,
		userMessages,
		assistantMessages,
		toolCalls,
		toolResults,
		totalMessages: messages.length,
		tokens: {
			input: totalInput,
			output: totalOutput,
			cacheRead: totalCacheRead,
			cacheWrite: totalCacheWrite,
			total: totalInput + totalOutput + totalCacheRead + totalCacheWrite,
		},
		cost: totalCost,
		contextUsage,
	};
}

/**
 * Estimate current context-window usage.
 *
 * After compaction, the last assistant usage reflects pre-compaction context
 * size, so usage is only trusted from an assistant that responded after the
 * latest compaction boundary. When no such assistant exists yet, tokens are
 * reported as null (unknown until the next LLM response).
 */
export function computeContextUsage(params: {
	model: Model<any> | undefined;
	sessionManager: SessionManager;
	messages: AgentMessage[];
}): ContextUsage | undefined {
	const { model, sessionManager, messages } = params;
	if (!model) return undefined;

	const contextWindow = model.contextWindow ?? 0;
	if (contextWindow <= 0) return undefined;

	const branchEntries = sessionManager.getBranch();
	const latestCompaction = getLatestCompactionEntry(branchEntries);

	if (latestCompaction) {
		// Check if there's a valid assistant usage after the compaction boundary
		const compactionIndex = branchEntries.lastIndexOf(latestCompaction);
		let hasPostCompactionUsage = false;
		for (let i = branchEntries.length - 1; i > compactionIndex; i--) {
			const entry = branchEntries[i];
			if (entry.type === "message" && entry.message.role === "assistant") {
				const assistant = entry.message;
				if (assistant.stopReason !== "aborted" && assistant.stopReason !== "error") {
					const contextTokens = calculateContextTokens(assistant.usage);
					if (contextTokens > 0) {
						hasPostCompactionUsage = true;
					}
					break;
				}
			}
		}

		if (!hasPostCompactionUsage) {
			return { tokens: null, contextWindow, percent: null };
		}
	}

	const estimate = estimateContextTokens(messages);
	const percent = (estimate.tokens / contextWindow) * 100;

	return {
		tokens: estimate.tokens,
		contextWindow,
		percent,
	};
}

/** Collect all user messages on the session (for the fork selector). */
export function collectUserMessagesForForking(
	sessionManager: SessionManager,
): Array<{ entryId: string; text: string }> {
	const entries = sessionManager.getEntries();
	const result: Array<{ entryId: string; text: string }> = [];

	for (const entry of entries) {
		if (entry.type !== "message") continue;
		if (entry.message.role !== "user") continue;

		const text = extractUserMessageText(entry.message.content);
		if (text) {
			result.push({ entryId: entry.id, text });
		}
	}

	return result;
}

/**
 * Get the text content of the last non-empty assistant message (for /copy).
 * Returns undefined if no assistant message with text exists.
 */
export function getLastAssistantText(messages: AgentMessage[]): string | undefined {
	const lastAssistant = messages
		.slice()
		.reverse()
		.find((m) => {
			if (m.role !== "assistant") return false;
			const msg = m as AssistantMessage;
			// Skip aborted messages with no content
			if (msg.stopReason === "aborted" && msg.content.length === 0) return false;
			return true;
		});

	if (!lastAssistant) return undefined;

	let text = "";
	for (const content of (lastAssistant as AssistantMessage).content) {
		if (content.type === "text") {
			text += content.text;
		}
	}

	return text.trim() || undefined;
}

/**
 * Export the current session branch to a JSONL file.
 * Writes the session header followed by all entries on the current branch path,
 * re-chaining parentIds into a linear sequence.
 * @returns The resolved output file path.
 */
export function exportSessionBranchToJsonl(sessionManager: SessionManager, outputPath?: string): string {
	const filePath = resolve(outputPath ?? `session-${new Date().toISOString().replace(/[:.]/g, "-")}.jsonl`);
	const dir = dirname(filePath);
	if (!existsSync(dir)) {
		mkdirSync(dir, { recursive: true });
	}

	const header: SessionHeader = {
		type: "session",
		version: CURRENT_SESSION_VERSION,
		id: sessionManager.getSessionId(),
		timestamp: new Date().toISOString(),
		cwd: sessionManager.getCwd(),
	};

	const branchEntries = sessionManager.getBranch();
	const lines = [JSON.stringify(header)];

	// Re-chain parentIds to form a linear sequence
	let prevId: string | null = null;
	for (const entry of branchEntries) {
		const linear = { ...entry, parentId: prevId };
		lines.push(JSON.stringify(linear));
		prevId = entry.id;
	}

	writeFileSync(filePath, `${lines.join("\n")}\n`);
	return filePath;
}
