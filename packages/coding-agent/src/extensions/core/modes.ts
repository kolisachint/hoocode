/**
 * Mode system — resolves the active mode (ask/plan/build/debug), loads the
 * mode's system prompt, filters active tools, and exposes the /mode, /plan,
 * /grill, and /approve commands.
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
import { isLightModeEnv } from "../../core/light.js";
import { DEFAULT_MODE, DEFAULT_MODE_PROMPTS as MODE_DEFAULTS } from "../../core/mode-prompts.js";
import { mergeSearchPaths, readConfig, readMergedConfig, writeConfig } from "./config.js";
import { LOOP_AUTO_CHANGED } from "./loop.js";

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

/** Outcome of reading the first usable plan file out of a candidate list. */
type PlanLoadResult =
	| { status: "loaded"; sections: PlanSections }
	| { status: "missing" }
	| { status: "error"; path: string };

/**
 * Reads the first candidate plan file that exists and is non-empty, parsed into
 * sections. Missing and blank files fall through to the next candidate; an
 * unreadable one stops the walk so the caller can report the specific path.
 */
function loadPlanSections(candidatePaths: string[]): PlanLoadResult {
	for (const planPath of candidatePaths) {
		if (!existsSync(planPath)) continue;
		try {
			const raw = readFileSync(planPath, "utf8").trim();
			if (raw) return { status: "loaded", sections: parsePlanSections(raw) };
		} catch {
			return { status: "error", path: planPath };
		}
	}
	return { status: "missing" };
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
// Grill: plan interrogation
// ============================================================================

/** Which half (or both) of the grill flow to run. */
export type GrillTarget = "both" | "me" | "plan";

/**
 * Parses the `/grill` argument into a target.
 *
 * Bare `/grill` runs both phases, `/grill me` only questions the user, and
 * `/grill plan` only critiques the plan. Anything else returns undefined so the
 * caller shows usage rather than guessing at intent.
 */
export function parseGrillTarget(args: string): GrillTarget | undefined {
	const arg = args.trim().toLowerCase();
	if (!arg) return "both";
	if (arg === "me" || arg === "plan") return arg;
	return undefined;
}

/**
 * Renders the parsed plan back into the message so the agent reviews a concrete
 * artifact rather than whatever it remembers writing. Falls back to the raw plan
 * text when no sections were recognised.
 */
function renderPlanForReview(sections: PlanSections): string {
	const parts: string[] = [];
	if (sections.goal) parts.push(`**Goal**\n${sections.goal}`);
	if (sections.filesToModify) parts.push(`**Files to modify**\n${sections.filesToModify}`);
	if (sections.newFiles) parts.push(`**New files**\n${sections.newFiles}`);
	if (sections.tests) parts.push(`**Tests**\n${sections.tests}`);
	if (sections.verification) parts.push(`**Verification**\n${sections.verification}`);
	return parts.length > 0 ? parts.join("\n\n") : sections.raw;
}

/** Phase 1 — interrogate the request, surfacing unknowns as ask_options questions. */
const GRILL_ME_PROMPT = `You are interrogating the *request*, not writing code.

Re-read the plan below and find where it rests on assumptions nobody confirmed: ambiguous requirements, unstated constraints, decisions you made silently because the request did not specify them, and places where a different reasonable reading of the request would produce a materially different plan.

Put the most consequential of those to the user with the ask_options tool. Ask only what would actually change the plan — settle anything you can from the codebase or from convention yourself. Prefer two to four sharp questions over an exhaustive list, and mark a recommended option wherever you have a genuine preference.

Do not edit files and do not revise the plan yet.`;

/** Phase 2 — adversarial self-critique of the plan file. */
const GRILL_PLAN_PROMPT = `You are attacking the plan below, not executing it.

Review it as a skeptical reviewer would:
- Which steps rest on assumptions you have not verified in the codebase? Read the relevant files and verify them now.
- Where could this fail silently — wrong-but-plausible behaviour rather than a loud error?
- What does the goal require that the plan does not cover? What does the plan cover that the goal does not need?
- Does the verification section prove the goal is met, or only that the code runs?

Report what you find. Where a weakness is real, revise the plan file and state what changed. Where the plan holds up, say so plainly rather than inventing criticism. Do not implement anything.`;

/**
 * Builds the follow-up message injected by `/grill`.
 *
 * The two phases are ordered rather than alternative: underspecification sits
 * upstream of plan weakness, so critiquing a plan built on a misread request
 * produces a well-reviewed plan for the wrong job. Bare `/grill` therefore asks
 * first and folds the answers into the critique.
 */
export function buildGrillMessage(sections: PlanSections, target: GrillTarget): string {
	const plan = renderPlanForReview(sections);
	if (target === "me") return `${GRILL_ME_PROMPT}\n\n---\n\n${plan}`;
	if (target === "plan") return `${GRILL_PLAN_PROMPT}\n\n---\n\n${plan}`;
	return (
		`${GRILL_ME_PROMPT}\n\n` +
		`Once the user has answered, fold their answers into the plan and immediately continue with this review:\n\n` +
		`${GRILL_PLAN_PROMPT}\n\n---\n\n${plan}`
	);
}

// ============================================================================
// setupMode
// ============================================================================

export function setupMode(pi: ExtensionAPI): void {
	let cachedMode = DEFAULT_MODE;
	let cachedSystemPrompt: string | undefined;
	let cachedPlanPath: string | undefined;

	// `/grill me` asks the user a question via ask_options, which an unattended
	// `/loop auto` run has nobody to answer. Track the loop's broadcast state so
	// the command can drop its interrogation phase instead of stalling the run.
	let autoLoopActive = false;
	pi.events.on(LOOP_AUTO_CHANGED, (data) => {
		autoLoopActive = (data as { active?: boolean } | undefined)?.active === true;
	});

	// ── session_start ─────────────────────────────────────────────────────────
	// Config resolution order:
	//   1. Read global config  (~/.hoocode/hoo-config.json)
	//   2. Read project config (./.hoocode/hoo-config.json) if present
	//   3. Merge — project scalars win; arrays are unioned
	//   4. Re-resolve active_mode from the merged result

	pi.on("session_start", (_event: SessionStartEvent, ctx: ExtensionContext) => {
		// Light mode strips every fixed prompt appendix. Leaving
		// cachedSystemPrompt unset makes before_agent_start a no-op, so no
		// `<!-- hoo-core: mode= -->` block is appended.
		if (isLightModeEnv()) {
			return;
		}

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

	// ── /grill command ────────────────────────────────────────────────────────
	// Stress-tests the current plan in two phases: `me` surfaces the request's
	// unconfirmed assumptions as ask_options questions, `plan` critiques the plan
	// file itself. Unlike /approve this only injects a follow-up message — no
	// session switch, no mode change, no config write.

	pi.registerCommand("grill", {
		description: "Stress-test the current plan. Usage: /grill [me|plan]",
		getArgumentCompletions: (prefix: string) =>
			["me", "plan"].filter((s) => s.startsWith(prefix)).map((s) => ({ value: s, label: s })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const requested = parseGrillTarget(args);
			if (!requested) {
				ctx.ui.notify("Usage: /grill [me|plan]", "warning");
				return;
			}

			// Same lookup as /approve: per-session plan first, legacy file second.
			const sessionPlanPath = cachedPlanPath ?? getPlanPath(ctx.cwd, ctx.sessionManager.getSessionId());
			const loaded = loadPlanSections([sessionPlanPath, getLegacyPlanPath(ctx.cwd)]);
			if (loaded.status === "error") {
				ctx.ui.notify(`Could not read ${relative(ctx.cwd, loaded.path) || loaded.path}`, "error");
				return;
			}
			if (loaded.status === "missing") {
				const relPlan = relative(ctx.cwd, sessionPlanPath) || sessionPlanPath;
				ctx.ui.notify(`No plan to grill — ${relPlan} not found. Run /plan first.`, "warning");
				return;
			}

			// Nobody is present to answer during an autonomous loop, so keep the
			// half of the flow that still works rather than blocking on a question.
			let target = requested;
			if (autoLoopActive && target !== "plan") {
				target = "plan";
				ctx.ui.notify("Autonomous loop active — grilling the plan only, skipping questions.", "info");
			}

			pi.sendUserMessage(buildGrillMessage(loaded.sections, target), { deliverAs: "followUp" });
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
			const loaded = loadPlanSections([sessionPlanPath, getLegacyPlanPath(ctx.cwd)]);
			if (loaded.status === "error") {
				ctx.ui.notify(`Could not read ${relative(ctx.cwd, loaded.path) || loaded.path}`, "error");
				return;
			}
			const approveMessage = loaded.status === "loaded" ? buildApproveMessage(loaded.sections) : undefined;

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
