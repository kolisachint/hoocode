/**
 * System prompt construction and project context loading
 */

import { type AgentDefinition, TASK_TOOL_NAME } from "./agent-frontmatter.js";
import { formatAgentsForPrompt } from "./agent-registry.js";
import { formatSelfDocsForPrompt } from "./self-docs.js";
import { formatSkillsForPrompt, type Skill } from "./skills.js";

export interface BuildSystemPromptOptions {
	/** Custom system prompt (replaces default). */
	customPrompt?: string;
	/** Tools to include in prompt. Default: [read, bash, edit, write, search] */
	selectedTools?: string[];
	/** Optional one-line tool snippets keyed by tool name. */
	toolSnippets?: Record<string, string>;
	/** Additional guideline bullets appended to the default system prompt guidelines. */
	promptGuidelines?: string[];
	/** Text to append to system prompt. */
	appendSystemPrompt?: string;
	/** Working directory. */
	cwd: string;
	/** Pre-loaded context files. */
	contextFiles?: Array<{ path: string; content: string }>;
	/** Pre-loaded skills. */
	skills?: Skill[];
	/**
	 * Available agents for delegation, emitted as `<available_agents>` XML.
	 * Only populated when the Task tool is active so the model knows which
	 * agents exist without re-reading the agent registry each turn.
	 */
	agents?: AgentDefinition[];
	/**
	 * Point the model at hoocode's own shipped docs so it can answer questions
	 * about hoocode itself.
	 *
	 * Defaults to true for the built-in prompt and false when `customPrompt`
	 * replaces it. Every other appended section (context files, skills, agents)
	 * only appears because the caller passed the content in; this one
	 * materializes on its own, so a caller who has taken over the system prompt
	 * gets it only by asking. That also keeps it out of light mode, whose whole
	 * point is a minimal fixed per-turn surface. Needs the read tool either way.
	 */
	includeSelfDocs?: boolean;
}

/** Build the system prompt with tools, guidelines, and context */
export function buildSystemPrompt(options: BuildSystemPromptOptions): string {
	const {
		customPrompt,
		selectedTools,
		toolSnippets,
		promptGuidelines,
		appendSystemPrompt,
		cwd,
		contextFiles: providedContextFiles,
		skills: providedSkills,
		agents: providedAgents,
		includeSelfDocs,
	} = options;
	const resolvedCwd = cwd;
	const promptCwd = resolvedCwd.replace(/\\/g, "/");

	const now = new Date();
	const year = now.getFullYear();
	const month = String(now.getMonth() + 1).padStart(2, "0");
	const day = String(now.getDate()).padStart(2, "0");
	const date = `${year}-${month}-${day}`;

	const appendSection = appendSystemPrompt ? `\n\n${appendSystemPrompt}` : "";
	const wantSelfDocs = includeSelfDocs ?? !customPrompt;

	const contextFiles = providedContextFiles ?? [];
	const skills = providedSkills ?? [];
	const agents = providedAgents ?? [];

	if (customPrompt) {
		let prompt = customPrompt;

		if (appendSection) {
			prompt += appendSection;
		}

		// Append project context files
		if (contextFiles.length > 0) {
			prompt += "\n\n# Project Context\n\n";
			prompt += "Project-specific instructions and guidelines:\n\n";
			for (const { path: filePath, content } of contextFiles) {
				prompt += `## ${filePath}\n\n${content}\n\n`;
			}
		}

		// Append skills section (only if read tool is available)
		const hasRead = !selectedTools || selectedTools.includes("read");
		if (hasRead && skills.length > 0) {
			prompt += formatSkillsForPrompt(skills);
		}

		// Append agents section (only when Task tool is active)
		const hasTask = !selectedTools || selectedTools.includes(TASK_TOOL_NAME);
		if (hasTask && agents.length > 0) {
			prompt += formatAgentsForPrompt(agents);
		}

		// Append hoocode's own docs (only if read tool is available)
		if (wantSelfDocs && hasRead) {
			prompt += formatSelfDocsForPrompt();
		}

		// Add date and working directory last
		prompt += `\nCurrent date: ${date}`;
		prompt += `\nCurrent working directory: ${promptCwd}`;

		return prompt;
	}

	// Build tools list based on selected tools.
	// A tool appears in Available tools only when the caller provides a one-line snippet.
	const tools = selectedTools || ["read", "bash", "edit", "write", "search"];
	const visibleTools = tools.filter((name) => !!toolSnippets?.[name]);
	const toolsList =
		visibleTools.length > 0 ? visibleTools.map((name) => `- ${name}: ${toolSnippets![name]}`).join("\n") : "(none)";

	// Build guidelines based on which tools are actually available
	const guidelinesList: string[] = [];
	const guidelinesSet = new Set<string>();
	const addGuideline = (guideline: string): void => {
		if (guidelinesSet.has(guideline)) {
			return;
		}
		guidelinesSet.add(guideline);
		guidelinesList.push(guideline);
	};

	const hasBash = tools.includes("bash");
	const hasSearch = tools.includes("search");
	const hasRead = tools.includes("read");

	// File exploration guidelines. `search` is the only dedicated discovery tool
	// left, so name it when it is registered and fall back to the shell
	// otherwise — never advertise a tool that isn't in the bundle.
	if (hasSearch) {
		addGuideline(
			"For code discovery use search (find where code lives by concept or identifier) instead of bash; it is faster and respects .gitignore",
		);
		if (hasBash) {
			addGuideline(
				"Between search and bash: search finds where code lives by concept, behavior, or half-known name (ranked results); shell out to rg/find/ls when you need exact matching lines, counts, or a raw directory listing",
			);
		}
	} else if (hasBash) {
		addGuideline("Use bash for file exploration (ls, rg/grep, find)");
	}

	for (const guideline of promptGuidelines ?? []) {
		const normalized = guideline.trim();
		if (normalized.length > 0) {
			addGuideline(normalized);
		}
	}

	// Always include these
	addGuideline(
		"Put independent tool calls in one message — they execute in parallel; only split them across turns when a call needs an earlier call's result",
	);
	addGuideline("Be concise in your responses");
	addGuideline("No preamble or postamble; do not restate the task or summarize what you just did");
	addGuideline('Do not add closers like "Let me know" or "Hope this helps"');
	addGuideline(
		"Do not narrate routine tool calls or results — the permission gate already shows them; speak when you have the answer or need a decision",
	);
	addGuideline(
		"Match the surrounding code's conventions for comments, docstrings, and types — do not add or strip them by default",
	);
	addGuideline("Show file paths clearly when working with files");

	const guidelines = guidelinesList.map((g) => `- ${g}`).join("\n");

	let prompt = `You are an expert coding assistant operating inside hoocode, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.

Available tools:
${toolsList}

In addition to the tools above, you may have access to other custom tools depending on the project.

Guidelines:
${guidelines}`;

	if (appendSection) {
		prompt += appendSection;
	}

	// Append project context files
	if (contextFiles.length > 0) {
		prompt += "\n\n# Project Context\n\n";
		prompt += "Project-specific instructions and guidelines:\n\n";
		for (const { path: filePath, content } of contextFiles) {
			prompt += `## ${filePath}\n\n${content}\n\n`;
		}
	}

	// Append skills section (only if read tool is available)
	if (hasRead && skills.length > 0) {
		prompt += formatSkillsForPrompt(skills);
	}

	// Append agents section (only when Task tool is active)
	const hasTask = tools.includes(TASK_TOOL_NAME);
	if (hasTask && agents.length > 0) {
		prompt += formatAgentsForPrompt(agents);
	}

	// Append hoocode's own docs (only if read tool is available)
	if (wantSelfDocs && hasRead) {
		prompt += formatSelfDocsForPrompt();
	}

	// Add date and working directory last
	prompt += `\nCurrent date: ${date}`;
	prompt += `\nCurrent working directory: ${promptCwd}`;

	return prompt;
}
