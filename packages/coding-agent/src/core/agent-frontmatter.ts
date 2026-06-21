/**
 * Agent frontmatter: schema, validation, and Claude Code compatibility shim.
 *
 * Agent definitions are Markdown files with YAML frontmatter, mirroring the
 * Claude Code subagent format so `.claude/agents/*.md` files import natively:
 *
 *   ---
 *   name: explore
 *   description: When and why to use this agent (drives auto-delegation).
 *   tools: Read, Grep, Glob, Bash   # optional allowlist; omit = inherit all
 *   model: sonnet                   # optional; sonnet|opus|haiku|inherit|pattern
 *   ---
 *   <system prompt body>
 *
 * The body becomes the subagent system prompt. Validation is non-fatal: we emit
 * diagnostics (warnings) and still load the agent when possible, matching the
 * behavior of skills.ts.
 */

import { basename } from "path";
import { parseFrontmatter } from "../utils/frontmatter.js";
import type { ResourceDiagnostic } from "./diagnostics.js";

/** Max name length, aligned with skills. */
const MAX_NAME_LENGTH = 64;

/** Max description length, aligned with skills. */
const MAX_DESCRIPTION_LENGTH = 1024;

/** Where an agent definition came from. Used for precedence and diagnostics. */
export type AgentSource = "builtin" | "user" | "project" | "claude-user" | "claude-project";

/** Sentinel meaning "use the parent session's model" (Claude Code `model: inherit`). */
export const MODEL_INHERIT = "inherit";

/**
 * The seven built-in hoocode tools. An agent's `tools` allowlist is normalized
 * against this set; unknown tools are dropped with a diagnostic.
 */
export const HOOCODE_TOOL_NAMES: readonly string[] = ["bash", "edit", "find", "grep", "ls", "read", "write"];

/**
 * Canonical registered names of the opt-in tools. These are case-sensitive
 * identifiers the system prompt and tool gating match exactly, so downstream
 * callers should reference these constants instead of hardcoding the strings
 * (a mis-cased `"task"` silently disables the agents/skills prompt sections).
 */
export const EXECUTE_TASK_TOOL_NAME = "ExecuteTask";
/** @deprecated Use EXECUTE_TASK_TOOL_NAME. Kept for backward compatibility. */
export const TASK_TOOL_NAME = EXECUTE_TASK_TOOL_NAME;
export const TODO_WRITE_TOOL_NAME = "TodoWrite";

/**
 * D7 — Claude Code compatibility shim.
 *
 * Maps Claude Code tool names (case-insensitive) to their hoocode equivalents.
 * Claude tools without a hoocode counterpart (MultiEdit, Task, WebFetch,
 * WebSearch, TodoWrite, NotebookEdit, MCP tools, ...) are intentionally absent
 * and get dropped during normalization.
 */
export const CLAUDE_TOOL_ALIASES: Readonly<Record<string, string>> = {
	read: "read",
	write: "write",
	edit: "edit",
	bash: "bash",
	grep: "grep",
	glob: "find",
	find: "find",
	ls: "ls",
};

/** Raw frontmatter shape before validation/normalization. */
export interface AgentFrontmatter {
	name?: string;
	description?: string;
	/** Claude Code uses a comma-separated string; a YAML list is also accepted. */
	tools?: string | string[];
	/** Tools to subtract from the agent's set (allow + deny), Claude Code compatible. */
	disallowedTools?: string | string[];
	/** sonnet | opus | haiku | inherit | a model id/pattern. */
	model?: string;
	/** hoocode extension (not part of the Claude Code format): turn cap. */
	maxTurns?: number;
	/** Claude Code extension: run this agent detached (non-blocking) so the parent polls for its result. */
	background?: boolean;
	/**
	 * hoocode extension: opt a agent into delegating via the ExecuteTask tool (subject to the
	 * tree-wide nesting cap). `true` = any subagent type; a comma-separated string or
	 * YAML list = only those types. Opt-in per agent so the deliberate "ExecuteTask is not a
	 * normal tool" boundary stays intact for everyone else.
	 */
	delegate?: boolean | string | string[];
	/**
	 * When true, this agent inherits the parent's full conversation (a fork) instead
	 * of starting from a fresh context, reusing the parent's prompt cache.
	 */
	fork?: boolean;
	[key: string]: unknown;
}

/** A validated, normalized agent definition. */
export interface AgentDefinition {
	name: string;
	description: string;
	/**
	 * Resolved hoocode tool allowlist. `undefined` means "inherit all parent
	 * tools" (Claude Code behavior when `tools` is omitted).
	 */
	tools?: string[];
	/** Resolved denylist subtracted from the agent's tool set. */
	disallowedTools?: string[];
	/**
	 * Model alias/pattern, the `inherit` sentinel, or `undefined` for the
	 * subagent default.
	 */
	model?: string;
	/** System prompt body (frontmatter stripped). */
	prompt: string;
	/** Origin of this definition. */
	source: AgentSource;
	/** Absolute path of the source file, when loaded from disk. */
	filePath?: string;
	/** hoocode extension: optional per-agent turn cap. */
	maxTurns?: number;
	/** When true, dispatch is non-blocking: the parent receives a handle and polls for the result. */
	background?: boolean;
	/** When true, this agent may delegate via the ExecuteTask tool, subject to the nesting cap. */
	delegate?: boolean;
	/** Restricts delegation to these subagent types (undefined = any when `delegate` is true). */
	delegateTo?: string[];
	/** When true, the agent inherits the parent's full conversation (a fork). */
	fork?: boolean;
}

const KNOWN_MODEL_ALIASES = new Set(["sonnet", "opus", "haiku", "inherit", "fast", "standard", "capable"]);

/** Validate an agent name. Returns warning messages (empty when valid). */
function validateName(name: string): string[] {
	const errors: string[] = [];
	if (!name) {
		errors.push("name is required");
		return errors;
	}
	if (name.length > MAX_NAME_LENGTH) {
		errors.push(`name exceeds ${MAX_NAME_LENGTH} characters (${name.length})`);
	}
	if (!/^[a-z0-9-]+$/.test(name)) {
		errors.push("name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)");
	}
	if (name.startsWith("-") || name.endsWith("-")) {
		errors.push("name must not start or end with a hyphen");
	}
	if (name.includes("--")) {
		errors.push("name must not contain consecutive hyphens");
	}
	return errors;
}

/** Validate a description. Returns warning messages (empty when valid). */
function validateDescription(description: string | undefined): string[] {
	const errors: string[] = [];
	if (!description || description.trim() === "") {
		errors.push("description is required");
	} else if (description.length > MAX_DESCRIPTION_LENGTH) {
		errors.push(`description exceeds ${MAX_DESCRIPTION_LENGTH} characters (${description.length})`);
	}
	return errors;
}

function validateModel(value: string): string[] {
	const trimmed = value.trim();
	if (!trimmed) return [];
	if (KNOWN_MODEL_ALIASES.has(trimmed)) return [];
	if (/^claude-/.test(trimmed)) return [];
	return [
		`model "${trimmed}" is not a recognized alias (sonnet | opus | haiku | inherit | fast | standard | capable) or full model ID; the agent may not load correctly`,
	];
}

/** Split a `tools` frontmatter value (string or list) into raw token names. */
function splitToolsValue(value: string | string[]): string[] {
	const tokens = Array.isArray(value) ? value : value.split(",");
	return tokens.map((t) => t.trim()).filter((t) => t.length > 0);
}

/**
 * Normalize a raw `tools` allowlist into hoocode tool names via the Claude Code
 * alias map. Returns the deduped, resolved list plus diagnostics for any tokens
 * that could not be mapped.
 *
 * Emits a warning when `value` is a YAML list rather than a comma-separated
 * string — the Claude Code standard format is `tools: read, bash` (string).
 */
export function normalizeTools(
	value: string | string[],
	filePath?: string,
): { tools: string[]; diagnostics: ResourceDiagnostic[] } {
	const diagnostics: ResourceDiagnostic[] = [];
	if (Array.isArray(value)) {
		diagnostics.push({
			type: "warning",
			message:
				'tools: use a comma-separated string ("tools: read, bash") instead of a YAML list for Claude Code compatibility',
			path: filePath,
		});
	}
	const resolved: string[] = [];
	for (const raw of splitToolsValue(value)) {
		const mapped = CLAUDE_TOOL_ALIASES[raw.toLowerCase()];
		if (!mapped) {
			diagnostics.push({
				type: "warning",
				message: `tool "${raw}" has no hoocode equivalent and was dropped from the allowlist`,
				path: filePath,
			});
			continue;
		}
		if (!resolved.includes(mapped)) {
			resolved.push(mapped);
		}
	}
	return { tools: resolved, diagnostics };
}

/**
 * Normalize a `model` frontmatter value. `inherit` is preserved as a sentinel;
 * any other non-empty string is passed through to the model resolver as-is
 * (so Claude aliases like `sonnet`/`opus`/`haiku` resolve via pattern match).
 */
export function normalizeModel(value: string | undefined): string | undefined {
	if (typeof value !== "string") return undefined;
	const trimmed = value.trim();
	if (!trimmed) return undefined;
	return trimmed;
}

/**
 * Parse and validate a single agent definition from raw Markdown content.
 *
 * `fallbackName` is used when frontmatter omits `name` (e.g. the filename, or
 * the embedded-template key). Returns `agent: null` only when the definition
 * is unusable (missing description). Other problems surface as diagnostics.
 */
export function parseAgentDefinition(
	rawContent: string,
	options: { source: AgentSource; filePath?: string; fallbackName?: string },
): { agent: AgentDefinition | null; diagnostics: ResourceDiagnostic[] } {
	const { source, filePath } = options;
	const diagnostics: ResourceDiagnostic[] = [];

	let frontmatter: AgentFrontmatter;
	let body: string;
	try {
		const parsed = parseFrontmatter<AgentFrontmatter>(rawContent);
		frontmatter = parsed.frontmatter;
		body = parsed.body;
	} catch (error) {
		const message = error instanceof Error ? error.message : "failed to parse agent frontmatter";
		diagnostics.push({ type: "warning", message, path: filePath });
		return { agent: null, diagnostics };
	}

	const fallbackName = options.fallbackName ?? (filePath ? basename(filePath, ".md") : "");
	const name = (frontmatter.name ?? fallbackName).trim();

	for (const error of validateName(name)) {
		diagnostics.push({ type: "warning", message: error, path: filePath });
	}

	const description = typeof frontmatter.description === "string" ? frontmatter.description.trim() : "";
	for (const error of validateDescription(description)) {
		diagnostics.push({ type: "warning", message: error, path: filePath });
	}

	// Description is mandatory: it drives delegation. Without it the agent is unusable.
	if (!description) {
		return { agent: null, diagnostics };
	}
	// A usable name is required as the registry key.
	if (!name || !/^[a-z0-9-]+$/.test(name)) {
		return { agent: null, diagnostics };
	}

	let tools: string[] | undefined;
	if (frontmatter.tools !== undefined) {
		const normalized = normalizeTools(frontmatter.tools, filePath);
		diagnostics.push(...normalized.diagnostics);
		tools = normalized.tools;
	}

	let disallowedTools: string[] | undefined;
	if (frontmatter.disallowedTools !== undefined) {
		const normalized = normalizeTools(frontmatter.disallowedTools, filePath);
		diagnostics.push(...normalized.diagnostics);
		disallowedTools = normalized.tools.length > 0 ? normalized.tools : undefined;
	}

	const model = normalizeModel(frontmatter.model);
	if (model !== undefined) {
		for (const error of validateModel(model)) {
			diagnostics.push({ type: "warning", message: error, path: filePath });
		}
	}

	const maxTurns =
		typeof frontmatter.maxTurns === "number" && Number.isInteger(frontmatter.maxTurns) && frontmatter.maxTurns > 0
			? frontmatter.maxTurns
			: undefined;

	let background: boolean | undefined;
	if (frontmatter.background !== undefined) {
		if (typeof frontmatter.background !== "boolean") {
			diagnostics.push({
				type: "warning",
				message: `background must be a boolean (true or false), got "${frontmatter.background}" — field ignored`,
				path: filePath,
			});
		} else {
			background = frontmatter.background === true ? true : undefined;
		}
	}

	// `delegate` accepts a boolean (delegate to any agent) or a comma-separated
	// string / YAML list of agent type names (delegate only to those).
	let fork: boolean | undefined;
	if (frontmatter.fork !== undefined) {
		if (typeof frontmatter.fork !== "boolean") {
			diagnostics.push({
				type: "warning",
				message: `fork must be a boolean (true or false), got "${frontmatter.fork}" — field ignored`,
				path: filePath,
			});
		} else {
			fork = frontmatter.fork === true ? true : undefined;
		}
	}

	let delegate: boolean | undefined;
	let delegateTo: string[] | undefined;
	if (frontmatter.delegate !== undefined) {
		const value = frontmatter.delegate;
		if (value === true) {
			delegate = true;
		} else if (value === false) {
			delegate = undefined;
		} else if (typeof value === "string" || Array.isArray(value)) {
			const names = (Array.isArray(value) ? value.join(",") : value)
				.split(",")
				.map((s) => s.trim().toLowerCase())
				.filter((s) => /^[a-z0-9-]+$/.test(s));
			if (names.length > 0) {
				delegate = true;
				delegateTo = [...new Set(names)];
			} else {
				diagnostics.push({
					type: "warning",
					message: `delegate list "${value}" contained no valid agent names — field ignored`,
					path: filePath,
				});
			}
		} else {
			diagnostics.push({
				type: "warning",
				message: `delegate must be a boolean or a list of agent names, got "${value}" — field ignored`,
				path: filePath,
			});
		}
	}

	return {
		agent: {
			name,
			description,
			tools,
			disallowedTools,
			model,
			prompt: body.trim(),
			source,
			filePath,
			maxTurns,
			background,
			delegate,
			delegateTo,
			fork,
		},
		diagnostics,
	};
}
