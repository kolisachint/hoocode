/**
 * Subagent core: spawn a fresh, isolated agent loop for one delegated task.
 *
 * Each invocation creates a brand-new in-process AgentSession with:
 *   - a clean, minimal system prompt (the mode template, no parent history),
 *   - an in-memory session (nothing persisted to disk),
 *   - a tool allowlist scoped to the mode.
 *
 * The subagent runs to completion and returns ONLY its final answer string;
 * intermediate reasoning, tool calls, and tool output are discarded.
 */

import type { AssistantMessage, Model } from "@kolisachint/hoocode-ai";
import { EMBEDDED_SUBAGENT_PROMPTS } from "../init-templates.generated.js";
import { createExtensionRuntime, type LoadExtensionsResult } from "./extensions/index.js";
import type { ModelRegistry } from "./model-registry.js";
import type { ResourceExtensionPaths, ResourceLoader } from "./resource-loader.js";
import { createAgentSession } from "./sdk.js";
import { SessionManager } from "./session-manager.js";

export type SubagentMode = "explore" | "edit" | "test" | "fix" | "review";

export const SUBAGENT_MODES: readonly SubagentMode[] = ["explore", "edit", "test", "fix", "review"];

/** Tool allowlist per mode. Read-only modes deliberately omit edit/write. */
const MODE_TOOLS: Record<SubagentMode, string[]> = {
	explore: ["read", "grep", "find", "ls", "bash"],
	edit: ["read", "edit", "write", "grep", "find", "ls", "bash"],
	test: ["read", "bash", "grep", "find", "ls"],
	fix: ["read", "edit", "write", "bash", "grep", "find", "ls"],
	review: ["read", "grep", "find", "ls", "bash"],
};

export interface RunSubagentOptions {
	/** The task to delegate. */
	task: string;
	/** Optional context distilled from the parent conversation. */
	context?: string;
	/** Which subagent mode to run. */
	mode: SubagentMode;
	/** Working directory for the subagent. */
	cwd: string;
	/** Model to use. When omitted the subagent picks the configured default. */
	model?: Model<any>;
	/** Model registry to reuse (shares the parent's auth). */
	modelRegistry?: ModelRegistry;
	/** Abort signal from the parent; aborts the subagent loop. */
	signal?: AbortSignal;
}

export interface SubagentResult {
	mode: SubagentMode;
	/** The subagent's final answer (its last assistant text), or "" on failure. */
	answer: string;
	ok: boolean;
	/** Populated when ok is false. */
	error?: string;
}

/** Return the clean, minimal system prompt for a subagent mode. */
export function getSubagentSystemPrompt(mode: SubagentMode): string {
	const prompt = EMBEDDED_SUBAGENT_PROMPTS[mode];
	if (!prompt) {
		throw new Error(`No system prompt template for subagent mode "${mode}"`);
	}
	return prompt;
}

/**
 * Resource loader that gives the subagent a clean context: just the mode's
 * system prompt, with no project context files, skills, prompts, or extensions.
 */
class MinimalResourceLoader implements ResourceLoader {
	private readonly extensionsResult: LoadExtensionsResult;

	constructor(private readonly systemPrompt: string) {
		this.extensionsResult = { extensions: [], errors: [], runtime: createExtensionRuntime() };
	}

	getExtensions(): LoadExtensionsResult {
		return this.extensionsResult;
	}

	getSkills() {
		return { skills: [], diagnostics: [] };
	}

	getPrompts() {
		return { prompts: [], diagnostics: [] };
	}

	getThemes() {
		return { themes: [], diagnostics: [] };
	}

	getAgentsFiles() {
		return { agentsFiles: [], warnings: [] };
	}

	getSystemPrompt(): string | undefined {
		return this.systemPrompt;
	}

	getAppendSystemPrompt(): string[] {
		return [];
	}

	addAppendSystemPrompt(_text: string): void {
		// Subagents do not accept dynamic prompt appendages.
	}

	extendResources(_paths: ResourceExtensionPaths): void {
		// Subagents do not accept additional resource paths.
	}

	async reload(): Promise<void> {
		// Nothing to reload: the context is fixed at construction.
	}
}

function composePrompt(task: string, context: string | undefined): string {
	const trimmedContext = context?.trim();
	const trimmedTask = task.trim();
	if (trimmedContext) {
		return `Context from the calling agent:\n\n${trimmedContext}\n\nTask: ${trimmedTask}`;
	}
	return `Task: ${trimmedTask}`;
}

/**
 * Run one subagent task to completion and return only its final answer.
 */
export async function runSubagent(options: RunSubagentOptions): Promise<SubagentResult> {
	const { task, context, mode, cwd, model, modelRegistry, signal } = options;

	if (signal?.aborted) {
		return { mode, answer: "", ok: false, error: "Subagent aborted before starting." };
	}

	const systemPrompt = getSubagentSystemPrompt(mode);
	const { session } = await createAgentSession({
		cwd,
		model,
		modelRegistry,
		tools: MODE_TOOLS[mode],
		resourceLoader: new MinimalResourceLoader(systemPrompt),
		sessionManager: SessionManager.inMemory(cwd),
	});

	const onAbort = () => {
		void session.abort();
	};
	signal?.addEventListener("abort", onAbort, { once: true });

	try {
		if (!session.model) {
			return { mode, answer: "", ok: false, error: "No model available for the subagent." };
		}

		await session.bindExtensions({ onError: () => {} });
		await session.prompt(composePrompt(task, context), {
			expandPromptTemplates: false,
			source: "extension",
		});

		const messages = session.messages;
		const last = messages[messages.length - 1];
		if (last?.role === "assistant") {
			const assistant = last as AssistantMessage;
			if (assistant.stopReason === "error" || assistant.stopReason === "aborted") {
				return {
					mode,
					answer: "",
					ok: false,
					error: assistant.errorMessage || `Subagent ${assistant.stopReason}.`,
				};
			}
		}

		const answer = session.getLastAssistantText() ?? "";
		return { mode, answer, ok: true };
	} catch (error) {
		return {
			mode,
			answer: "",
			ok: false,
			error: error instanceof Error ? error.message : String(error),
		};
	} finally {
		signal?.removeEventListener("abort", onAbort);
		session.dispose();
	}
}
