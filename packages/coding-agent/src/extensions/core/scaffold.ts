/**
 * Scaffold commands — /new-skill, /new-agent, /new-command, and /create-canvas.
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
import { CANVAS_ENTRY_FILE } from "../../core/canvas/discovery.js";
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

const SKILL_BODY_TEMPLATE = (name: string) =>
	[
		`# ${name}`,
		"",
		"TODO: write the skill instructions here.",
		"",
		"When relative paths appear below, they are resolved from this file's directory.",
		"",
	].join("\n");

const AGENT_BODY_TEMPLATE = (name: string) =>
	[
		`You are a ${name} subagent.`,
		"You run in an isolated context and cannot see the parent conversation.",
		"",
		"TODO: write the system prompt here.",
		"",
		"Your final message must contain ONLY your answer — it is the only output",
		"the caller receives. Do not include intermediate reasoning or tool logs.",
		"",
	].join("\n");

const COMMAND_BODY_TEMPLATE = (name: string) =>
	[
		`Run the /${name} command with arguments: **$ARGUMENTS**.`,
		"",
		"TODO: write the instructions here. Placeholders you can use:",
		"- $1, $2, ... for positional arguments",
		"- $@ or $ARGUMENTS for all arguments",
		"",
	].join("\n");

/**
 * A working single-canvas extension.
 *
 * Deliberately complete rather than a stub: a canvas has no passive half — its
 * name, its actions and its UI all come from running its code — so a scaffold
 * that does not run teaches nothing and cannot be checked with `/canvas open`.
 * This one opens, serves a page, answers an action, and closes cleanly, which is
 * the whole contract; everything past that is the author's.
 *
 * Shaped like the catalog extensions hoocode already hosts: the only import is
 * `@github/copilot-sdk/extension` (host-resolved — never installed, see
 * `docs/canvas-extensions-design.md` §4.1) plus `node:` builtins, and the UI is
 * served from an ephemeral loopback port behind a per-instance token so nothing
 * else on the machine can read it.
 */
const CANVAS_ENTRY_TEMPLATE = (name: string): string => `\
// ${name} — a canvas extension. Run it with: /canvas open ${name}
//
// The "@github/copilot-sdk/extension" import is resolved by the host at fork
// time. Do not install it, and do not add a node_modules here.
import { createCanvas, CanvasError, joinSession } from "@github/copilot-sdk/extension";
import { randomBytes } from "node:crypto";
import { createServer } from "node:http";

/** Per-instance state. A canvas can be opened more than once at a time. */
const instances = new Map();

const session = await joinSession({
	canvases: [
		createCanvas({
			id: "${name}",
			displayName: "${name}",
			description: "TODO: one sentence — the agent reads this to decide whether to open it.",
			actions: [
				{
					name: "add_note",
					description: "TODO: describe what the agent can do to this canvas.",
					inputSchema: {
						type: "object",
						properties: { text: { type: "string" } },
						required: ["text"],
					},
					handler: (ctx) => {
						const entry = instances.get(ctx.instanceId);
						// CanvasError carries a code the host shows verbatim; a bare
						// throw arrives as an opaque internal error instead.
						if (!entry) throw new CanvasError("no_instance", \`Instance "\${ctx.instanceId}" is not open.\`);
						entry.notes.push(ctx.input.text);
						return { notes: entry.notes.length };
					},
				},
			],
			open: async (ctx) => {
				const token = randomBytes(32).toString("base64url");
				const notes = [];
				const server = createServer((req, res) => {
					const url = new URL(req.url ?? "/", "http://127.0.0.1");
					if (url.searchParams.get("token") !== token) {
						res.writeHead(403, { "Content-Type": "text/plain" });
						res.end("forbidden");
						return;
					}
					res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
					res.end(
						\`<!doctype html><meta charset="utf-8"><title>${name}</title>\` +
							\`<p>\${notes.length} note(s). TODO: build the UI.\`,
					);
				});
				// Port 0 on 127.0.0.1: an ephemeral port, reachable only from this machine.
				await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
				const { port } = server.address();
				instances.set(ctx.instanceId, { server, notes });
				return {
					url: \`http://127.0.0.1:\${port}/?token=\${token}\`,
					title: "${name}",
				};
			},
			onClose: async (ctx) => {
				const entry = instances.get(ctx.instanceId);
				// Called for an abandoned open too, so the instance may be unknown.
				if (!entry) return;
				instances.delete(ctx.instanceId);
				await new Promise((resolve) => entry.server.close(resolve));
			},
		}),
	],
});

// stdout is the protocol channel. Use session.log, never console.log.
await session.log("${name} ready");
`;

/**
 * Where a canvas extension lives, per platform.
 *
 * This does not go through {@link WorkspaceLayout} like the other scaffolds, and
 * the reason is that a canvas has no Claude convention to emit into: the surface
 * exists in Copilot and in hoocode's own `.agents/` tree and nowhere else. A
 * layout method returning nothing for one adapter would be a worse lie than
 * naming the two real homes here — these are exactly the roots
 * `core/canvas/discovery.ts` searches, which is what makes a scaffold live on
 * the next /canvas.
 */
const CANVAS_HOMES: Partial<Record<MarketplacePlatform, string[]>> = {
	agents: [".agents", "extensions"],
	github: [".github", "extensions"],
};

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
						description:
							"TODO: describe when to use this skill — the agent reads this to decide whether to load it.",
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
					"description: |",
					"  TODO: describe when to use this skill — one clear sentence per bullet.",
					"  The model reads this to decide whether to load the skill.",
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
						description: "TODO: describe the task(s) to delegate to this agent.",
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
					"  - TODO: describe the task(s) to delegate here",
					"",
					"  DO NOT use for:",
					"  - TODO: describe what this agent should NOT handle",
					"tools: read, bash",
					"model: sonnet",
					"---",
					`You are a ${name} subagent running inside hoocode.`,
					"You run in an isolated context and cannot see the parent conversation.",
					"",
					"TODO: write the system prompt here.",
					"",
					"Your final message must contain ONLY your answer — it is the only output",
					"the caller receives. Do not include intermediate reasoning or tool logs.",
					"",
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
						description: `TODO: describe what /${name} does and when to use it.`,
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
					`  TODO: describe what /${name} does and when to use it.`,
					`  Usage: /${name} <args>`,
					"argument-hint: <args>",
					"---",
					`Run the /${name} command with arguments: **$ARGUMENTS**.`,
					"",
					"TODO: write the instructions here. Placeholders you can use:",
					"- $1, $2, ... for positional arguments",
					"- $@ or $ARGUMENTS for all arguments",
					`- $${"{"}@:N} / $${"{"}@:N:L} for bash-style slices`,
					"",
				].join("\n"),
				"utf8",
			);

			ctx.ui.notify(
				`Command created: ${join(".hoocode", "commands", `${name}.md`)}\nEdit the file, then run /reload to activate it.`,
				"info",
			);
		},
	});

	// ── /create-canvas <name> ─────────────────────────────────────────────────
	// Creates a canvas extension — .agents/extensions/<name>/extension.mjs by
	// default, or .github/extensions/<name>/ with --platform github (Copilot's
	// project scope). Design: docs/canvas-extensions-design.md §9, Phase 3.
	//
	// Unlike the other scaffolds this one needs no /reload: canvases are
	// discovered when /canvas runs, not loaded at session start, so the new
	// extension is openable immediately.

	pi.registerCommand("create-canvas", {
		description: "Scaffold a new canvas extension. Usage: /create-canvas <name>",
		getArgumentCompletions: () => [],
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const name = args.trim();
			const error = validateResourceName(name);
			if (error) {
				ctx.ui.notify(`/create-canvas: ${error}. Usage: /create-canvas <name>`, "warning");
				return;
			}

			// `--platform claude` is dropped rather than redirected: Claude has no
			// canvas convention, and silently writing into someone else's marker
			// directory would put an extension where that vendor will never look.
			const requested = getWorkspacePlatforms() ?? ["agents"];
			const targets = requested.filter((platform) => CANVAS_HOMES[platform] !== undefined);
			if (targets.length === 0) {
				ctx.ui.notify(
					`/create-canvas: no canvas home for platform "${requested.join(", ")}". ` +
						"Canvas extensions exist under .agents/extensions (agents) and .github/extensions (github) only.",
					"warning",
				);
				return;
			}

			const created: string[] = [];
			const skipped: string[] = [];
			for (const platform of targets) {
				const home = CANVAS_HOMES[platform];
				if (!home) continue;
				const relative = join(...home, name, CANVAS_ENTRY_FILE);
				const absolute = join(ctx.cwd, relative);
				if (existsSync(absolute)) {
					skipped.push(relative);
					continue;
				}
				mkdirSync(dirname(absolute), { recursive: true });
				writeFileSync(absolute, CANVAS_ENTRY_TEMPLATE(name), "utf8");
				created.push(relative);
			}

			const lines: string[] = [];
			if (created.length > 0) {
				lines.push("Canvas created:", ...created.map((file) => `  ${file}`));
				lines.push("", `Open it now with /canvas open ${name} — no /reload needed.`);
			}
			if (skipped.length > 0) {
				lines.push("Skipped (already exist):", ...skipped.map((file) => `  ${file}`));
			}
			ctx.ui.notify(lines.join("\n"), created.length > 0 ? "info" : "warning");
		},
	});
}
