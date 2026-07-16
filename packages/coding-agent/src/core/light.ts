/**
 * Light mode — a minimal, low-token preset for small/local models.
 *
 * The preset restricts the session to the four core tools (read, write, edit,
 * bash) with shortened descriptions and undocumented parameter schemas,
 * replaces the default system prompt with a terse one, and disables
 * subagents, TodoWrite, skills, context files, and the mode-prompt appendix.
 * The goal is the smallest possible fixed per-turn surface (system prompt +
 * serialized tool schemas) so weak tool-callers waste no context on harness
 * boilerplate. bash subsumes grep/find/ls: search happens via the shell.
 */

import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { Type } from "typebox";
import type { ToolInfo } from "./extensions/types.js";
import { createBashTool, createEditTool, createReadTool, createWriteTool } from "./tools/index.js";

/** The only tools a light session exposes. */
export const LIGHT_TOOL_NAMES = ["read", "write", "edit", "bash"] as const;

/**
 * Terse replacement for the default system prompt. buildSystemPrompt appends
 * the current date and working directory; nothing else rides along because
 * light mode also disables skills, context files, and the mode appendix.
 */
export const LIGHT_SYSTEM_PROMPT = `You are a coding agent. Use the tools to read, edit, and write files and run shell commands.
Search with bash (grep/find/ls). Prefer edit for changes; write for new files.
Be concise. No preamble.`;

/**
 * Environment flag signaling light mode to code that cannot see CLI flags or
 * settings — notably the hoo-core modes extension, which reads it to skip the
 * `<!-- hoo-core: mode= -->` system-prompt appendix. Same pattern as
 * WARM_SUBAGENTS_ENV / SUBAGENT_MAX_DEPTH_ENV.
 */
export const LIGHT_MODE_ENV = "HOOCODE_LIGHT";

/** Whether light mode was signaled through the environment. */
export function isLightModeEnv(): boolean {
	return process.env[LIGHT_MODE_ENV] === "1";
}

// Light parameter schemas: same shapes the real tools accept, minus the
// per-property descriptions (that is where most schema tokens live).
const lightReadSchema = Type.Object({
	path: Type.String(),
	offset: Type.Optional(Type.Number()),
	limit: Type.Optional(Type.Number()),
});

const lightWriteSchema = Type.Object({
	path: Type.String(),
	content: Type.String(),
});

// Flat single-replacement shape instead of the full edits[] batch schema. The
// execute shim below converts it to the batch form the real edit tool expects.
const lightEditSchema = Type.Object({
	path: Type.String(),
	oldText: Type.String(),
	newText: Type.String(),
});

const lightBashSchema = Type.Object({
	command: Type.String(),
	timeout: Type.Optional(Type.Number()),
});

/**
 * Build the four light tools for `baseToolsOverride`: the real tool
 * implementations wearing short descriptions and stripped parameter schemas.
 */
export function createLightTools(cwd: string): Record<string, AgentTool> {
	const read = createReadTool(cwd);
	const write = createWriteTool(cwd);
	const edit = createEditTool(cwd);
	const bash = createBashTool(cwd);

	const lightRead: AgentTool<typeof lightReadSchema> = {
		...read,
		description: "Read a file. args: path, offset?, limit?",
		parameters: lightReadSchema,
	};

	const lightWrite: AgentTool<typeof lightWriteSchema> = {
		...write,
		description: "Write file (overwrites). args: path, content",
		parameters: lightWriteSchema,
	};

	// The real edit tool validates against its edits[] schema after
	// prepareArguments runs, so the flat light shape must skip the original
	// prepareArguments (validation sees the raw flat args against the flat
	// schema) and convert to the batch form at execute time instead.
	const lightEdit: AgentTool<typeof lightEditSchema> = {
		...edit,
		description: "Replace exact text. args: path, oldText, newText",
		parameters: lightEditSchema,
		prepareArguments: undefined,
		execute: (toolCallId, params, signal, onUpdate) =>
			edit.execute(
				toolCallId,
				{ path: params.path, edits: [{ oldText: params.oldText, newText: params.newText }] },
				signal,
				onUpdate,
			),
	};

	const lightBash: AgentTool<typeof lightBashSchema> = {
		...bash,
		description: "Run a shell command. args: command, timeout?",
		parameters: lightBashSchema,
	};

	return {
		read: lightRead as AgentTool,
		write: lightWrite as AgentTool,
		edit: lightEdit as AgentTool,
		bash: lightBash as AgentTool,
	};
}

/** Token breakdown of a session's fixed per-turn surface. */
export interface PromptSurface {
	/** Estimated tokens in the assembled system prompt. */
	systemPromptTokens: number;
	/** Estimated tokens across the serialized active tool schemas. */
	toolSchemaTokens: number;
	/** systemPromptTokens + toolSchemaTokens. */
	totalTokens: number;
	/** Per-tool breakdown of the serialized schema estimate. */
	tools: Array<{ name: string; tokens: number }>;
}

/** Same conservative chars/4 heuristic the agent harness uses for context estimates. */
function estimateStringTokens(text: string): number {
	return Math.ceil(text.length / 4);
}

/**
 * Measure the fixed per-turn surface a session sends on every request: the
 * assembled system prompt plus the serialized schemas (name, description,
 * parameters) of the active tools. Providers add their own envelope on top,
 * so treat the result as a floor estimate.
 */
export function measurePromptSurface(session: {
	systemPrompt: string;
	getActiveToolNames(): string[];
	getAllTools(): ToolInfo[];
}): PromptSurface {
	const activeNames = new Set(session.getActiveToolNames());
	const tools = session
		.getAllTools()
		.filter((tool) => activeNames.has(tool.name))
		.map((tool) => ({
			name: tool.name,
			tokens: estimateStringTokens(
				JSON.stringify({ name: tool.name, description: tool.description, parameters: tool.parameters }),
			),
		}));
	const systemPromptTokens = estimateStringTokens(session.systemPrompt);
	const toolSchemaTokens = tools.reduce((sum, tool) => sum + tool.tokens, 0);
	return {
		systemPromptTokens,
		toolSchemaTokens,
		totalTokens: systemPromptTokens + toolSchemaTokens,
		tools,
	};
}
