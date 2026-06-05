/**
 * Custom message types and transformers for the coding agent.
 *
 * Extends the base AgentMessage type with coding-agent specific message types,
 * and provides a transformer to convert them to LLM-compatible messages.
 */

import type { AgentMessage, BackgroundToolResult } from "@kolisachint/hoocode-agent-core";
import type { ImageContent, Message, TextContent } from "@kolisachint/hoocode-ai";

export const COMPACTION_SUMMARY_PREFIX = `The conversation history before this point was compacted into the following summary:

<summary>
`;

export const COMPACTION_SUMMARY_SUFFIX = `
</summary>`;

export const BRANCH_SUMMARY_PREFIX = `The following is a summary of a branch that this conversation came back from:

<summary>
`;

export const BRANCH_SUMMARY_SUFFIX = `</summary>`;

/**
 * Message type for bash executions via the ! command.
 */
export interface BashExecutionMessage {
	role: "bashExecution";
	command: string;
	output: string;
	exitCode: number | undefined;
	cancelled: boolean;
	truncated: boolean;
	fullOutputPath?: string;
	timestamp: number;
	/** If true, this message is excluded from LLM context (!! prefix) */
	excludeFromContext?: boolean;
}

/**
 * Message type for extension-injected messages via sendMessage().
 * These are custom messages that extensions can inject into the conversation.
 */
export interface CustomMessage<T = unknown> {
	role: "custom";
	customType: string;
	content: string | (TextContent | ImageContent)[];
	display: boolean;
	details?: T;
	timestamp: number;
}

export interface BranchSummaryMessage {
	role: "branchSummary";
	summary: string;
	fromId: string;
	timestamp: number;
}

export interface CompactionSummaryMessage {
	role: "compactionSummary";
	summary: string;
	tokensBefore: number;
	/** Estimated context tokens after compaction; absent on entries written before this field existed. */
	tokensAfter?: number;
	timestamp: number;
}

// Extend CustomAgentMessages via declaration merging
declare module "@kolisachint/hoocode-agent-core" {
	interface CustomAgentMessages {
		bashExecution: BashExecutionMessage;
		custom: CustomMessage;
		branchSummary: BranchSummaryMessage;
		compactionSummary: CompactionSummaryMessage;
	}
}

/**
 * Convert a BashExecutionMessage to user message text for LLM context.
 */
export function bashExecutionToText(msg: BashExecutionMessage): string {
	let text = `Ran \`${msg.command}\`\n`;
	if (msg.output) {
		text += `\`\`\`\n${msg.output}\n\`\`\``;
	} else {
		text += "(no output)";
	}
	if (msg.cancelled) {
		text += "\n\n(command cancelled)";
	} else if (msg.exitCode !== null && msg.exitCode !== undefined && msg.exitCode !== 0) {
		text += `\n\nCommand exited with code ${msg.exitCode}`;
	}
	if (msg.truncated && msg.fullOutputPath) {
		text += `\n\n[Output truncated. Full output: ${msg.fullOutputPath}]`;
	}
	return text;
}

export function createBranchSummaryMessage(summary: string, fromId: string, timestamp: string): BranchSummaryMessage {
	return {
		role: "branchSummary",
		summary,
		fromId,
		timestamp: new Date(timestamp).getTime(),
	};
}

export function createCompactionSummaryMessage(
	summary: string,
	tokensBefore: number,
	timestamp: string,
	tokensAfter?: number,
): CompactionSummaryMessage {
	return {
		role: "compactionSummary",
		summary: summary,
		tokensBefore,
		tokensAfter,
		timestamp: new Date(timestamp).getTime(),
	};
}

/** customType used for the follow-up message a finished background tool injects. */
export const BACKGROUND_TASK_CUSTOM_TYPE = "backgroundTask";

/** Minimal shape of a tool call, shared by the placeholder and follow-up builders. */
interface BackgroundToolCall {
	name: string;
	arguments?: Record<string, unknown>;
}

/** A consistent, human-readable description of a background tool call. */
export interface BackgroundToolInfo {
	/** True for MCP server tools (registered as `mcp_<server>_<tool>`). */
	isMcpTool: boolean;
	/** The subagent type for `Task` calls; the tool name otherwise. */
	subagentType: string;
	/** Short label used verbatim in both the start and finish messages (kept in sync). */
	label: string;
	/** One-line summary of what the call is doing, derived from its arguments. */
	summary?: string;
}

/** First non-empty line of a string, trimmed and capped for one-line display. */
function firstLine(text: string, max = 120): string {
	const line = (text.split("\n").find((l) => l.trim().length > 0) ?? "").trim();
	return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
}

/** Summarize a tool's arguments as up to three `key: value` pairs for a one-liner. */
export function summarizeArgs(args: Record<string, unknown>): string | undefined {
	const parts: string[] = [];
	for (const [key, value] of Object.entries(args)) {
		if (value === undefined || value === null || value === "") continue;
		const rendered = typeof value === "string" ? value : JSON.stringify(value);
		parts.push(`${key}: ${firstLine(rendered, 48)}`);
		if (parts.length === 3) break;
	}
	return parts.length > 0 ? parts.join(", ") : undefined;
}

/**
 * Describe a background tool call consistently, so the "started" placeholder, the
 * "finished" follow-up message, and any UI all read the same way. Subagent `Task`
 * calls are keyed off their `subagent_type`; MCP server tools off their `mcp_`
 * prefix.
 */
export function describeBackgroundTool(toolCall: BackgroundToolCall): BackgroundToolInfo {
	const args = toolCall.arguments ?? {};
	const isMcpTool = toolCall.name.startsWith("mcp_");

	if (isMcpTool) {
		// Registered name is `mcp_<server>_<tool>`; drop the prefix for display.
		const pretty = toolCall.name.replace(/^mcp_/, "");
		return { isMcpTool, subagentType: toolCall.name, label: `MCP tool \`${pretty}\``, summary: summarizeArgs(args) };
	}

	const subagentType = typeof args.subagent_type === "string" ? args.subagent_type : toolCall.name;
	const summary =
		typeof args.description === "string" && args.description.trim()
			? args.description.trim()
			: typeof args.prompt === "string"
				? firstLine(args.prompt)
				: undefined;
	return { isMcpTool, subagentType, label: `subagent \`${subagentType}\``, summary };
}

/**
 * Verbose, human-readable placeholder shown the moment a background tool is
 * dispatched (before its result is known). Wired into the agent loop via
 * `createBackgroundPlaceholder`, it replaces the generic "Started X in the
 * background" line so the demo legibly explains what each background agent / MCP
 * tool is doing. Kept in sync with the finish message via {@link describeBackgroundTool}.
 */
export function createBackgroundPlaceholderText(toolCall: BackgroundToolCall): string {
	const info = describeBackgroundTool(toolCall);
	const lead = info.isMcpTool
		? `Started ${info.label} in the background — it runs on an external MCP server and may take a while.`
		: `Delegated to ${info.label} in the background — it runs in its own isolated context and cannot see this conversation.`;
	const what = info.summary ? `\nTask: ${info.summary}` : "";
	return `${lead}${what}\nIts result will arrive here as a follow-up message once it finishes — keep working in the meantime; you do not need to poll for it.`;
}

/**
 * Build the follow-up message injected when a background tool finishes.
 *
 * Handles both subagent `Task` calls and MCP tools, described consistently with
 * the start placeholder via {@link describeBackgroundTool}. Rendered as a distinct
 * custom message rather than a plain user message, and converted to a user message
 * for the LLM.
 */
export function createBackgroundTaskMessage(result: BackgroundToolResult): CustomMessage {
	const info = describeBackgroundTool(result.toolCall);
	const verb = result.isError ? "failed" : "finished";
	const summary = info.summary ? ` (${info.summary})` : "";
	const header = `Background ${info.label}${summary} ${verb}:`;
	return {
		role: "custom",
		customType: BACKGROUND_TASK_CUSTOM_TYPE,
		content: [{ type: "text", text: header }, ...result.result.content],
		display: true,
		details: { subagentType: info.subagentType, isMcpTool: info.isMcpTool, isError: result.isError },
		timestamp: Date.now(),
	};
}

/** Convert CustomMessageEntry to AgentMessage format */
export function createCustomMessage(
	customType: string,
	content: string | (TextContent | ImageContent)[],
	display: boolean,
	details: unknown | undefined,
	timestamp: string,
): CustomMessage {
	return {
		role: "custom",
		customType,
		content,
		display,
		details,
		timestamp: new Date(timestamp).getTime(),
	};
}

/**
 * Transform AgentMessages (including custom types) to LLM-compatible Messages.
 *
 * This is used by:
 * - Agent's transormToLlm option (for prompt calls and queued messages)
 * - Compaction's generateSummary (for summarization)
 * - Custom extensions and tools
 */
export function convertToLlm(messages: AgentMessage[]): Message[] {
	return messages
		.map((m): Message | undefined => {
			switch (m.role) {
				case "bashExecution":
					// Skip messages excluded from context (!! prefix)
					if (m.excludeFromContext) {
						return undefined;
					}
					return {
						role: "user",
						content: [{ type: "text", text: bashExecutionToText(m) }],
						timestamp: m.timestamp,
					};
				case "custom": {
					const content = typeof m.content === "string" ? [{ type: "text" as const, text: m.content }] : m.content;
					return {
						role: "user",
						content,
						timestamp: m.timestamp,
					};
				}
				case "branchSummary":
					return {
						role: "user",
						content: [{ type: "text" as const, text: BRANCH_SUMMARY_PREFIX + m.summary + BRANCH_SUMMARY_SUFFIX }],
						timestamp: m.timestamp,
					};
				case "compactionSummary":
					return {
						role: "user",
						content: [
							{ type: "text" as const, text: COMPACTION_SUMMARY_PREFIX + m.summary + COMPACTION_SUMMARY_SUFFIX },
						],
						timestamp: m.timestamp,
					};
				case "user":
				case "assistant":
				case "toolResult":
					return m;
				default:
					// biome-ignore lint/correctness/noSwitchDeclarations: fine
					const _exhaustiveCheck: never = m;
					return undefined;
			}
		})
		.filter((m) => m !== undefined);
}
