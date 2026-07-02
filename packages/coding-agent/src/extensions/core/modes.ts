/**
 * Mode system — resolves the active mode (ask/plan/build/debug), loads the
 * mode's system prompt, filters active tools, and exposes the /mode, /plan,
 * and /approve commands.
 *
 * Mode prompt search order (first hit wins):
 *   - `./.hoocode/modes/{mode}/system.md`
 *   - `~/.hoocode/modes/{mode}/system.md`
 *   - each configured/contributed external dir in declared order
 *   - built-in MODE_DEFAULTS for the four known modes
 */

import { existsSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { getHooCodeDir } from "../../config.js";
import type {
	BeforeAgentStartEvent,
	BeforeAgentStartEventResult,
	ExtensionAPI,
	ExtensionCommandContext,
	ExtensionContext,
	SessionStartEvent,
} from "../../core/extensions/types.js";
import { DEFAULT_MODE, DEFAULT_MODE_PROMPTS as MODE_DEFAULTS } from "../../core/mode-prompts.js";
import { mergeSearchPaths, readConfig, readMergedConfig, writeConfig } from "./config.js";

const HOOCODE_DIR = getHooCodeDir();

/**
 * Per-session plan file path. Keying on sessionId lets concurrent or resumed
 * plan sessions keep distinct plans instead of clobbering each other.
 */
function getPlanPath(cwd: string, sessionId: string): string {
	return join(cwd, ".hoocode", "plans", `${sessionId}.md`);
}

/** Legacy single-file plan location, retained as a read-only fallback for /approve. */
function getLegacyPlanPath(cwd: string): string {
	return join(cwd, ".hoocode", "plan.md");
}

function tryReadFile(path: string): string | undefined {
	if (!existsSync(path)) return undefined;
	try {
		const text = readFileSync(path, "utf8").trim();
		return text || undefined;
	} catch {
		return undefined;
	}
}

/**
 * Walks search dirs in precedence order and returns the first existing
 * `modes/{name}/system.md` content. Order: project → user → externalDirs.
 */
function resolveModeFile(name: string, cwd: string, externalDirs: string[]): string | undefined {
	const candidates: string[] = [
		join(cwd, ".hoocode", "modes", name, "system.md"),
		join(HOOCODE_DIR, "modes", name, "system.md"),
		...externalDirs.map((dir) => join(dir, name, "system.md")),
	];
	for (const candidate of candidates) {
		const content = tryReadFile(candidate);
		if (content !== undefined) return content;
	}
	return undefined;
}

/**
 * Returns the system prompt for the active mode.
 *
 * Search order (first hit wins):
 *   - `./.hoocode/modes/{mode}/system.md`
 *   - `~/.hoocode/modes/{mode}/system.md`
 *   - each of `externalDirs` in declared order (config + CLI + extension contributions)
 *   - built-in MODE_DEFAULTS for the four known modes
 */
export function buildSystemPrompt(mode: string, cwd: string, options?: { modePaths?: string[] }): string | undefined {
	const modePaths = options?.modePaths ?? [];
	return resolveModeFile(mode, cwd, modePaths) ?? MODE_DEFAULTS[mode];
}

// ============================================================================
// Plan file: section parsing and step-by-step execution message
// ============================================================================

export interface PlanSections {
	goal?: string;
	filesToModify?: string;
	newFiles?: string;
	tests?: string;
	verification?: string;
	/** Original full text, used as fallback if no sections parsed */
	raw: string;
}

/**
 * Parses `.hoocode/plan.md` into named sections.
 *
 * Recognises both ATX headings (`## Goal`) and bold labels (`**Goal**`).
 * Section names matched (case-insensitive): Goal, Files to modify, New files,
 * Tests, Verification.
 */
export function parsePlanSections(planContent: string): PlanSections {
	const result: PlanSections = { raw: planContent };

	// Match `## Heading text` or `**Heading text**` followed by content until
	// the next heading of the same style.
	const sectionPattern =
		/^(?:#{1,3}\s+(.+?)|(?:\*\*(.+?)\*\*))\s*\n([\s\S]*?)(?=(?:^#{1,3}\s+|\*\*[^*\n]+\*\*\s*\n)|$)/gm;

	for (const match of planContent.matchAll(sectionPattern)) {
		const heading = (match[1] ?? match[2] ?? "").toLowerCase().trim();
		const content = match[3].trim();
		if (!content) continue;

		if (/^goal/.test(heading)) {
			result.goal = content;
		} else if (/files?\s+to\s+modif|^modif/.test(heading)) {
			result.filesToModify = content;
		} else if (/new\s+files?/.test(heading)) {
			result.newFiles = content;
		} else if (/^tests?/.test(heading)) {
			result.tests = content;
		} else if (/^verif/.test(heading)) {
			result.verification = content;
		}
	}

	return result;
}

/**
 * Builds the user message sent to the agent when `/approve` is run.
 *
 * If the plan has recognisable sections, each is presented as a numbered step
 * so the agent works through them sequentially. Otherwise the raw plan is used.
 *
 * Execution order:
 *   1. Modify existing files
 *   2. Create new files
 *   3. Update / add tests
 *   4. Run verification commands
 */
export function buildApproveMessage(sections: PlanSections): string {
	const steps: string[] = [];

	if (sections.goal) {
		steps.push(`**Goal:** ${sections.goal}`);
	}
	if (sections.filesToModify) {
		steps.push(`**Step 1 — Modify existing files:**\n${sections.filesToModify}`);
	}
	if (sections.newFiles) {
		steps.push(`**Step 2 — Create new files:**\n${sections.newFiles}`);
	}
	if (sections.tests) {
		steps.push(`**Step 3 — Update tests:**\n${sections.tests}`);
	}
	if (sections.verification) {
		steps.push(`**Step 4 — Verify:**\n${sections.verification}`);
	}

	if (steps.length === 0) {
		return `Execute the following plan:\n\n${sections.raw}`;
	}

	return `Execute this plan step by step. Complete each step fully before moving to the next.\n\n${steps.join("\n\n")}`;
}

// ============================================================================
// setupMode
// ============================================================================

export function setupMode(pi: ExtensionAPI): void {
	let cachedMode = DEFAULT_MODE;
	let cachedSystemPrompt: string | undefined;
	let cachedPlanPath: string | undefined;

	// ── session_start ─────────────────────────────────────────────────────────
	// Config resolution order:
	//   1. Read global config  (~/.hoocode/hoo-config.json)
	//   2. Read project config (./.hoocode/hoo-config.json) if present
	//   3. Merge — project scalars win; arrays are unioned
	//   4. Re-resolve active_mode from the merged result

	pi.on("session_start", (_event: SessionStartEvent, ctx: ExtensionContext) => {
		// Steps 1–3: merge global + project configs
		const config = readMergedConfig(ctx.cwd);

		// Step 4: resolve mode from the merged config
		cachedMode = config.active_mode ?? DEFAULT_MODE;
		// External search dirs come from two channels:
		//  - HooConfig.mode_paths (config-declared)
		//  - pi.addModeSearchPath (CLI flags + extension contributions)
		const modePaths = mergeSearchPaths(config.mode_paths, pi.getModeSearchPaths());
		const rawSystemPrompt = buildSystemPrompt(cachedMode, ctx.cwd, { modePaths });

		// Per-session plan path so concurrent sessions don't overwrite each other.
		// The `{{PLAN_PATH}}` token in plan-mode templates is substituted here.
		cachedPlanPath = getPlanPath(ctx.cwd, ctx.sessionManager.getSessionId());
		const relPlanPath = relative(ctx.cwd, cachedPlanPath) || cachedPlanPath;
		cachedSystemPrompt = rawSystemPrompt?.replace(/\{\{PLAN_PATH\}\}/g, relPlanPath);

		// Update footer with active mode
		if (ctx.hasUI) {
			ctx.ui.setMode(cachedMode);
		}

		// Apply tool filter from mode enabled_tools
		const modeCfg = config.modes?.[cachedMode];
		if (modeCfg?.enabled_tools && modeCfg.enabled_tools.length > 0) {
			pi.setActiveTools(modeCfg.enabled_tools);
		}
	});

	// ── before_agent_start ────────────────────────────────────────────────────

	pi.on("before_agent_start", (event: BeforeAgentStartEvent): BeforeAgentStartEventResult | undefined => {
		if (!cachedSystemPrompt) return;
		return {
			systemPrompt: `${event.systemPrompt}\n\n<!-- hoo-core: mode=${cachedMode} -->\n${cachedSystemPrompt}`,
		};
	});

	// ── /mode command ─────────────────────────────────────────────────────────

	const KNOWN_MODES = ["ask", "plan", "build", "debug"];

	pi.registerCommand("mode", {
		description: "Switch active mode. Usage: /mode <ask|plan|build|debug>",
		getArgumentCompletions: (prefix: string) =>
			KNOWN_MODES.filter((m) => m.startsWith(prefix)).map((m) => ({ value: m, label: m })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const name = args.trim();
			if (!name) {
				ctx.ui.notify(`Active mode: ${cachedMode}`, "info");
				return;
			}
			const config = readConfig();
			config.active_mode = name === DEFAULT_MODE ? undefined : name;
			writeConfig(config);
			ctx.ui.notify(`Mode set to "${name}" — reloading…`, "info");
			await ctx.reload();
		},
	});

	// ── /plan command (shorthand for /mode plan) ──────────────────────────────

	pi.registerCommand("plan", {
		description: "Switch to plan mode. Shorthand for /mode plan.",
		getArgumentCompletions: () => [],
		handler: async (_args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const config = readConfig();
			config.active_mode = "plan";
			writeConfig(config);
			ctx.ui.notify(`Mode set to "plan" — reloading…`, "info");
			await ctx.reload();
		},
	});

	// ── /approve command ──────────────────────────────────────────────────────
	// Reads .hoocode/plan.md, parses it into named sections (Goal, Files to
	// modify, New files, Tests, Verification), switches to build mode, then
	// injects a step-by-step execution message into the new session.

	pi.registerCommand("approve", {
		description: "Approve the current plan and switch to build mode to execute it.",
		getArgumentCompletions: () => [],
		handler: async (_args: string, ctx: ExtensionCommandContext): Promise<void> => {
			if (cachedMode !== "plan") {
				ctx.ui.notify(`/approve is only available in plan mode (current mode: "${cachedMode}")`, "warning");
				return;
			}

			// Prefer the per-session plan file, fall back to the legacy single file.
			const sessionPlanPath = cachedPlanPath ?? getPlanPath(ctx.cwd, ctx.sessionManager.getSessionId());
			const candidatePaths = [sessionPlanPath, getLegacyPlanPath(ctx.cwd)];
			let approveMessage: string | undefined;

			for (const planPath of candidatePaths) {
				if (!existsSync(planPath)) continue;
				try {
					const raw = readFileSync(planPath, "utf8").trim();
					if (raw) {
						const sections = parsePlanSections(raw);
						approveMessage = buildApproveMessage(sections);
						break;
					}
				} catch {
					ctx.ui.notify(`Could not read ${relative(ctx.cwd, planPath) || planPath}`, "error");
					return;
				}
			}

			// Switch global config to build mode
			const config = readConfig();
			config.active_mode = "build";
			writeConfig(config);

			if (approveMessage) {
				// Open a new build-mode session and deliver the parsed plan as the
				// first user message so the agent starts executing immediately
				await ctx.newSession({
					withSession: async (replacedCtx) => {
						await replacedCtx.sendUserMessage(approveMessage!, { deliverAs: "followUp" });
					},
				});
			} else {
				const relPlan = relative(ctx.cwd, sessionPlanPath) || sessionPlanPath;
				ctx.ui.notify(`Switched to build mode. No ${relPlan} found — describe what to build.`, "info");
				await ctx.reload();
			}
		},
	});
}
