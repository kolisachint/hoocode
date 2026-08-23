/**
 * Scaffold commands — /new-skill, /new-agent, and /new-command.
 *
 * `/new-canvas` was their fourth sibling and is not here any more. It stopped
 * being a file-writing command when it grew Copilot's `/create-canvas` shape:
 * it opens what it writes and hands the agent a brief to build it, so it needs
 * the canvas session and the agent loop, neither of which belongs in a scaffold.
 * It lives in `extensions/core/canvas.ts` over `core/canvas/scaffold.ts`.
 *
 * Without `--platform`, each creates a ready-to-edit resource file
 * under `.hoocode/` (hoocode's private surface), picked up on the next /reload.
 *
 * With `--platform` (or the `platform` setting), the scaffold
 * instead lands in each target platform's *workspace* conventions via the
 * format registry's per-adapter {@link WorkspaceLayout} — e.g.
 * `--platform copilot` writes `.github/skills/<name>/SKILL.md`,
 * `.github/agents/<name>.agent.md`, and `.github/prompts/<name>.prompt.md`,
 * while `claude` writes `.claude/skills|agents|commands/`. hoocode reads all
 * of these back, so the scaffold is live after /reload either way.
 */

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { getFormatByPlatform } from "../../core/extensions/plugins/formats/index.js";
import { getWorkspacePlatforms } from "../../core/extensions/plugins/formats/platform-targets.js";
import type { EmittedFile, MarketplacePlatform, WorkspaceLayout } from "../../core/extensions/plugins/formats/types.js";
import type { ExtensionAPI, ExtensionCommandContext } from "../../core/extensions/types.js";

/** Validates a resource name: lowercase a-z, 0-9, hyphens, no leading/trailing/double hyphens. */
function validateResourceName(name: string): string | null {
	if (!name) return "name is required";
	if (!/^[a-z0-9-]+$/.test(name)) return "name must be lowercase a-z, 0-9, and hyphens only";
	if (name.startsWith("-") || name.endsWith("-")) return "name must not start or end with a hyphen";
	if (name.includes("--")) return "name must not contain consecutive hyphens";
	return null;
}

/**
 * Write one scaffolded artifact into every `--platform` target's
 * workspace layout. Existing files are never clobbered — they are reported and
 * skipped. Returns true when the platform-targeted path handled the command.
 */
function scaffoldForPlatforms(
	ctx: ExtensionCommandContext,
	command: string,
	platforms: MarketplacePlatform[],
	emit: (workspace: WorkspaceLayout) => EmittedFile,
): void {
	const created: string[] = [];
	const skipped: string[] = [];
	for (const platform of platforms) {
		const adapter = getFormatByPlatform(platform);
		if (!adapter) continue;
		const file = emit(adapter.workspace);
		const abs = join(ctx.cwd, file.path);
		if (existsSync(abs)) {
			skipped.push(file.path);
			continue;
		}
		mkdirSync(dirname(abs), { recursive: true });
		writeFileSync(abs, file.content, "utf8");
		created.push(file.path);
	}

	const lines: string[] = [];
	if (created.length > 0) {
		lines.push(`Created (${platforms.join(", ")}):`, ...created.map((f) => `  ${f}`));
		lines.push("Edit the file(s), then run /reload to activate.");
	}
	if (skipped.length > 0) {
		lines.push(`Skipped (already exist):`, ...skipped.map((f) => `  ${f}`));
	}
	if (lines.length === 0) {
		lines.push(`/${command}: no writable platform targets resolved`);
	}
	ctx.ui.notify(lines.join("\n"), created.length > 0 ? "info" : "warning");
}

/**
 * The scaffold bodies and descriptions, each defined once.
 *
 * Every command here has two paths — the `--platform` emitters and the plain
 * `.hoocode/` writer — and each path used to carry its own copy of the text.
 * They had drifted in both directions: the `.hoocode/` agent body said "running
 * inside hoocode" where the platform one did not, and the `.hoocode/` command
 * body documented the bash-style slice placeholders that the platform one
 * silently omitted, so what `/new-command` taught you depended on whether
 * `--platform` was set. Both paths read these now.
 *
 * These stay as functions rather than moving to `templates/`: unlike the mode
 * and `/grill` prompts they interpolate throughout and feed a structured
 * emitter, so a flat markdown file would be the worse home.
 */
const SKILL_BODY_TEMPLATE = (name: string) =>
	[
		`# ${name}`,
		"",
		"TODO: write the skill instructions here.",
		"",
		"When relative paths appear below, they are resolved from this file's directory.",
		"",
	].join("\n");

const SKILL_DESCRIPTION_TEMPLATE =
	"TODO: describe when to use this skill — the agent reads this to decide whether to load it.";

const AGENT_BODY_TEMPLATE = (name: string) =>
	[
		`You are a ${name} subagent running inside hoocode.`,
		"You run in an isolated context and cannot see the parent conversation.",
		"",
		"TODO: write the system prompt here.",
		"",
		"Your final message must contain ONLY your answer — it is the only output",
		"the caller receives. Do not include intermediate reasoning or tool logs.",
		"",
	].join("\n");

const AGENT_DESCRIPTION_TEMPLATE = "TODO: describe the task(s) to delegate to this agent.";

const COMMAND_BODY_TEMPLATE = (name: string) =>
	[
		`Run the /${name} command with arguments: **$ARGUMENTS**.`,
		"",
		"TODO: write the instructions here. Placeholders you can use:",
		"- $1, $2, ... for positional arguments",
		"- $@ or $ARGUMENTS for all arguments",
		`- $${"{"}@:N} / $${"{"}@:N:L} for bash-style slices`,
		"",
	].join("\n");

const COMMAND_DESCRIPTION_TEMPLATE = (name: string) => `TODO: describe what /${name} does and when to use it.`;

export function setupScaffold(pi: ExtensionAPI): void {
	// ── /new-skill <name> ─────────────────────────────────────────────────────
	// Creates a SKILL.md with valid Agent Skills frontmatter — under .hoocode/ by
	// default, or under each --platform target's skills directory.

	pi.registerCommand("new-skill", {
		description: "Scaffold a new skill. Usage: /new-skill <name>",
		getArgumentCompletions: () => [],
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const name = args.trim();
			const error = validateResourceName(name);
			if (error) {
				ctx.ui.notify(`/new-skill: ${error}. Usage: /new-skill <name>`, "warning");
				return;
			}

			const platforms = getWorkspacePlatforms();
			if (platforms) {
				scaffoldForPlatforms(ctx, "new-skill", platforms, (ws) =>
					ws.emitSkill({
						name,
						description: SKILL_DESCRIPTION_TEMPLATE,
						body: SKILL_BODY_TEMPLATE(name),
					}),
				);
				return;
			}

			const skillDir = join(ctx.cwd, ".hoocode", "skills", name);
			const skillFile = join(skillDir, "SKILL.md");

			if (existsSync(skillFile)) {
				ctx.ui.notify(`/new-skill: ${skillFile} already exists`, "warning");
				return;
			}

			mkdirSync(skillDir, { recursive: true });
			writeFileSync(
				skillFile,
				[
					"---",
					`name: ${name}`,
					`description: ${SKILL_DESCRIPTION_TEMPLATE}`,
					"allowed-tools: read, bash",
					"---",
					"",
					SKILL_BODY_TEMPLATE(name),
				].join("\n"),
				"utf8",
			);

			ctx.ui.notify(
				`Skill created: ${join(".hoocode", "skills", name, "SKILL.md")}\nEdit the file, then run /reload to activate it.`,
				"info",
			);
		},
	});

	// ── /new-agent <name> ─────────────────────────────────────────────────────
	// Creates a subagent definition — .hoocode/agents/<name>.md by default, or
	// each platform's convention (.claude/agents/<name>.md,
	// .github/agents/<name>.agent.md with a YAML-list tools grant, ...).

	pi.registerCommand("new-agent", {
		description: "Scaffold a new subagent. Usage: /new-agent <name>",
		getArgumentCompletions: () => [],
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const name = args.trim();
			const error = validateResourceName(name);
			if (error) {
				ctx.ui.notify(`/new-agent: ${error}. Usage: /new-agent <name>`, "warning");
				return;
			}

			const platforms = getWorkspacePlatforms();
			if (platforms) {
				scaffoldForPlatforms(ctx, "new-agent", platforms, (ws) =>
					ws.emitAgent({
						name,
						description: AGENT_DESCRIPTION_TEMPLATE,
						tools: "read, bash",
						body: AGENT_BODY_TEMPLATE(name),
					}),
				);
				return;
			}

			const agentsDir = join(ctx.cwd, ".hoocode", "agents");
			const agentFile = join(agentsDir, `${name}.md`);

			if (existsSync(agentFile)) {
				ctx.ui.notify(`/new-agent: ${agentFile} already exists`, "warning");
				return;
			}

			mkdirSync(agentsDir, { recursive: true });
			writeFileSync(
				agentFile,
				[
					"---",
					`name: ${name}`,
					"description: |",
					"  Use this subagent ONLY when:",
					`  - ${AGENT_DESCRIPTION_TEMPLATE}`,
					"",
					"  DO NOT use for:",
					"  - TODO: describe what this agent should NOT handle",
					"tools: read, bash",
					"model: sonnet",
					"---",
					AGENT_BODY_TEMPLATE(name),
				].join("\n"),
				"utf8",
			);

			ctx.ui.notify(
				`Agent created: ${join(".hoocode", "agents", `${name}.md`)}\nEdit the file, then run /reload to activate it.`,
				"info",
			);
		},
	});

	// ── /new-command <name> ───────────────────────────────────────────────────
	// Creates a slash-command prompt template — .hoocode/commands/<name>.md by
	// default, or each platform's convention (.claude/commands/<name>.md,
	// .github/prompts/<name>.prompt.md, ...).

	pi.registerCommand("new-command", {
		description: "Scaffold a new slash command. Usage: /new-command <name>",
		getArgumentCompletions: () => [],
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const name = args.trim();
			const error = validateResourceName(name);
			if (error) {
				ctx.ui.notify(`/new-command: ${error}. Usage: /new-command <name>`, "warning");
				return;
			}

			const platforms = getWorkspacePlatforms();
			if (platforms) {
				scaffoldForPlatforms(ctx, "new-command", platforms, (ws) =>
					ws.emitCommand({
						name,
						description: COMMAND_DESCRIPTION_TEMPLATE(name),
						body: COMMAND_BODY_TEMPLATE(name),
					}),
				);
				return;
			}

			const commandsDir = join(ctx.cwd, ".hoocode", "commands");
			const commandFile = join(commandsDir, `${name}.md`);

			if (existsSync(commandFile)) {
				ctx.ui.notify(`/new-command: ${commandFile} already exists`, "warning");
				return;
			}

			mkdirSync(commandsDir, { recursive: true });
			writeFileSync(
				commandFile,
				[
					"---",
					`name: ${name}`,
					"description: |",
					`  ${COMMAND_DESCRIPTION_TEMPLATE(name)}`,
					`  Usage: /${name} <args>`,
					"argument-hint: <args>",
					"---",
					COMMAND_BODY_TEMPLATE(name),
				].join("\n"),
				"utf8",
			);

			ctx.ui.notify(
				`Command created: ${join(".hoocode", "commands", `${name}.md`)}\nEdit the file, then run /reload to activate it.`,
				"info",
			);
		},
	});
}
