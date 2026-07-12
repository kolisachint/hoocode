/**
 * Capability authoring tools (spec §3). The two escalating-risk build paths are
 * kept as *separate tools* by design — not collapsed into one:
 *
 *   ProposePlugin            scaffold passive/low-risk capabilities — skills,
 *                            commands, and read-only subagents. Autonomous +
 *                            transparent (near-zero risk, reversible).
 *   ProposeExecutablePlugin  the risk-bearing path — hooks, MCP servers, and
 *                            mutating/high-privilege subagents. Draft → display
 *                            the code and the tool grant → human confirms →
 *                            activate. Bar >= install.
 *
 * Both author into `.agents/plugins/<id>/` in the requested vendor layouts
 * (Claude Code + GitHub Copilot by default) via the format registry, so results
 * are proper, publishable plugins that round-trip through parsePluginDir.
 *
 * Privilege-amplification guardrail: an authored subagent may never carry a
 * plugin-system (capability-acquisition) tool in its allowlist — enforced in
 * both tools — so a low-trust authored agent cannot bootstrap privilege.
 */

import { type Static, Type } from "typebox";
import {
	classifyAllowlist,
	DEFAULT_AUTHORING_PLATFORMS,
	pluginExists,
	writePluginDraft,
} from "../extensions/plugins/authoring.js";
import type { MarketplacePlatform, PluginDraft } from "../extensions/plugins/formats/types.js";
import type { ExtensionContext } from "../extensions/types.js";
import { defineTool, type ToolDefinition } from "../extensions/types.js";
import { PROPOSE_EXECUTABLE_PLUGIN_TOOL_NAME, PROPOSE_PLUGIN_TOOL_NAME } from "./plugin-tool-names.js";

export { PROPOSE_EXECUTABLE_PLUGIN_TOOL_NAME, PROPOSE_PLUGIN_TOOL_NAME } from "./plugin-tool-names.js";

const platformSchema = Type.Union([Type.Literal("claude"), Type.Literal("github"), Type.Literal("agents")], {
	description: "Target format: claude (Claude Code), github (GitHub Copilot), or agents (native).",
});

const skillSchema = Type.Object(
	{
		name: Type.String({ description: "Skill name." }),
		description: Type.Optional(Type.String({ description: "One-line trigger description (kept lazy in context)." })),
		body: Type.String({ description: "SKILL.md instruction body (markdown)." }),
	},
	{ additionalProperties: false },
);

const commandSchema = Type.Object(
	{
		name: Type.String({ description: "Command name (invoked as /name)." }),
		description: Type.Optional(Type.String({ description: "One-line description." })),
		body: Type.String({ description: "Prompt template body (markdown)." }),
	},
	{ additionalProperties: false },
);

const subagentSchema = Type.Object(
	{
		name: Type.String({ description: "Subagent name." }),
		description: Type.Optional(Type.String({ description: "When to dispatch this subagent." })),
		tools: Type.Optional(
			Type.String({ description: "Comma-separated allowed-tools, e.g. 'read, grep, glob'. Omit for none." }),
		),
		model: Type.Optional(Type.String({ description: "Model override, or 'inherit'." })),
		body: Type.String({ description: "System-prompt / instruction body (markdown)." }),
	},
	{ additionalProperties: false },
);

const hookSchema = Type.Object(
	{
		event: Type.String({ description: "Event: PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, Stop, ..." }),
		matcher: Type.Optional(Type.String({ description: "Regex matched against the tool name. Empty/'*' = all." })),
		command: Type.String({ description: "Shell command to run on the event." }),
		timeout: Type.Optional(Type.Number({ description: "Timeout in seconds." })),
	},
	{ additionalProperties: false },
);

const mcpServerSchema = Type.Object(
	{
		name: Type.String({ description: "MCP server name." }),
		command: Type.String({ description: "Executable to launch the server." }),
		args: Type.Optional(Type.Array(Type.String(), { description: "Command arguments." })),
		env: Type.Optional(Type.Record(Type.String(), Type.String(), { description: "Environment variables." })),
	},
	{ additionalProperties: false },
);

function resolvePlatforms(input: string[] | undefined): MarketplacePlatform[] {
	return (input && input.length > 0 ? (input as MarketplacePlatform[]) : DEFAULT_AUTHORING_PLATFORMS).slice();
}

function summarizeWrite(id: string, platforms: MarketplacePlatform[], files: string[], dest: string): string {
	return (
		`Authored plugin "${id}" (${platforms.join(", ")}) with ${files.length} file(s) at ${dest}:\n` +
		files.map((f) => `  ${f}`).join("\n") +
		`\nActive after the next reload; remove it with UninstallPlugin.`
	);
}

// ── ProposePlugin (scaffold path) ─────────────────────────────────────────────

const proposeParams = Type.Object(
	{
		id: Type.String({ description: "Plugin id (directory + manifest name)." }),
		description: Type.Optional(Type.String({ description: "Plugin description." })),
		version: Type.Optional(Type.String({ description: "Plugin version, e.g. '0.1.0'." })),
		platforms: Type.Optional(
			Type.Array(platformSchema, { description: "Formats to scaffold into. Default: claude + github." }),
		),
		skills: Type.Optional(Type.Array(skillSchema)),
		commands: Type.Optional(Type.Array(commandSchema)),
		subagents: Type.Optional(
			Type.Array(subagentSchema, { description: "Read-only subagents only (mutating grants are rejected here)." }),
		),
	},
	{ additionalProperties: false },
);

export interface ProposePluginDetails {
	id: string;
	authored: boolean;
}

export function createProposePluginToolDefinition(): ToolDefinition {
	return defineTool<typeof proposeParams, ProposePluginDetails>({
		name: PROPOSE_PLUGIN_TOOL_NAME,
		label: PROPOSE_PLUGIN_TOOL_NAME,
		description:
			"Author a new plugin from passive, low-risk capabilities — skills, slash commands, and READ-ONLY subagents — when no marketplace plugin fits a gap. Scaffolds a proper plugin (Claude Code + GitHub Copilot layouts by default). Autonomous and reversible. For hooks, MCP servers, or mutating/high-privilege subagents, use ProposeExecutablePlugin instead.",
		promptSnippet:
			"Author a skill/command/read-only-subagent plugin to fill a capability gap (scaffold; reversible).",
		promptGuidelines: [
			"Use ProposePlugin only for passive capabilities: skills, commands, and subagents whose tools are read-only (read, grep, glob, webfetch).",
			"A subagent that needs Bash/Write/Edit/MCP or tools:* is mutating — author it with ProposeExecutablePlugin (human confirmation), not here.",
			"Never grant a subagent any plugin-system tool (InstallPlugin, ProposePlugin, ...); that is always rejected.",
		],
		parameters: proposeParams,
		async execute(_id, params: Static<typeof proposeParams>, _signal, _onUpdate, ctx: ExtensionContext) {
			const platforms = resolvePlatforms(params.platforms);

			// Guardrail + risk gate on each subagent before writing anything.
			for (const sa of params.subagents ?? []) {
				const cls = classifyAllowlist(sa.tools);
				if (cls.pluginTools.length > 0) {
					return reject(
						`Subagent "${sa.name}" requests plugin-system tools (${cls.pluginTools.join(", ")}). ` +
							"Authored subagents may never carry capability-acquisition tools.",
						params.id,
					);
				}
				if (cls.risk === "mutating") {
					return reject(
						`Subagent "${sa.name}" has a mutating allowlist (${cls.reason}). ` +
							"Use ProposeExecutablePlugin so the human can review and confirm the tool grant.",
						params.id,
					);
				}
			}

			if (pluginExists(ctx.cwd, params.id)) {
				return reject(
					`A plugin named "${params.id}" already exists. Uninstall it first or pick another id.`,
					params.id,
				);
			}

			const draft: PluginDraft = {
				id: params.id,
				version: params.version,
				description: params.description,
				supportPlatform: platforms,
				skills: params.skills,
				commands: params.commands,
				agents: params.subagents,
			};
			const result = writePluginDraft(ctx.cwd, draft, platforms);
			const text = summarizeWrite(params.id, platforms, result.files, result.dest);
			ctx.ui.notify(`Authored plugin "${params.id}" (${platforms.join(", ")}).`, "info");
			return { content: [{ type: "text" as const, text }], details: { id: params.id, authored: true } };
		},
	});
}

// ── ProposeExecutablePlugin (risk-bearing path) ───────────────────────────────

const proposeExecParams = Type.Object(
	{
		id: Type.String({ description: "Plugin id (directory + manifest name)." }),
		description: Type.Optional(Type.String({ description: "Plugin description." })),
		version: Type.Optional(Type.String({ description: "Plugin version, e.g. '0.1.0'." })),
		platforms: Type.Optional(
			Type.Array(platformSchema, { description: "Formats to scaffold into. Default: claude + github." }),
		),
		hooks: Type.Optional(Type.Array(hookSchema)),
		mcpServers: Type.Optional(Type.Array(mcpServerSchema)),
		subagents: Type.Optional(
			Type.Array(subagentSchema, { description: "Subagents with mutating/exec/network or tools:* allowlists." }),
		),
	},
	{ additionalProperties: false },
);

export interface ProposeExecutablePluginDetails {
	id: string;
	authored: boolean;
	confirmed: boolean;
}

/** Build the human-facing review text: the executable code and every tool grant. */
function buildReview(params: Static<typeof proposeExecParams>): string {
	const lines: string[] = [`Plugin "${params.id}" wants to install executable capabilities:`];
	for (const h of params.hooks ?? []) {
		lines.push(`  hook [${h.event}${h.matcher ? ` matcher=${h.matcher}` : ""}]: ${h.command}`);
	}
	for (const s of params.mcpServers ?? []) {
		lines.push(`  mcp server "${s.name}": ${s.command}${s.args?.length ? ` ${s.args.join(" ")}` : ""}`);
	}
	for (const sa of params.subagents ?? []) {
		lines.push(`  subagent "${sa.name}" tools: ${sa.tools ?? "(none)"}`);
	}
	return lines.join("\n");
}

export function createProposeExecutablePluginToolDefinition(): ToolDefinition {
	return defineTool<typeof proposeExecParams, ProposeExecutablePluginDetails>({
		name: PROPOSE_EXECUTABLE_PLUGIN_TOOL_NAME,
		label: PROPOSE_EXECUTABLE_PLUGIN_TOOL_NAME,
		description:
			"Author a plugin that includes EXECUTABLE or high-privilege capabilities — hooks (run on tool events), MCP servers, or subagents with mutating/exec/network tool grants. The code and the exact tool grant are shown to the human, who must confirm before anything is activated. Use ProposePlugin for passive skills/commands/read-only subagents.",
		promptSnippet: "Author a hook/MCP/high-privilege-subagent plugin (shows the code; requires human confirmation).",
		promptGuidelines: [
			"ProposeExecutablePlugin always shows the code and tool grant and requires explicit human confirmation before activating.",
			"Never grant a subagent any plugin-system tool (InstallPlugin, ProposePlugin, ...); that is always rejected.",
			"Publishing a proven-useful plugin to a marketplace stays a human action — do not do it autonomously.",
		],
		parameters: proposeExecParams,
		async execute(_id, params: Static<typeof proposeExecParams>, _signal, _onUpdate, ctx: ExtensionContext) {
			const platforms = resolvePlatforms(params.platforms);

			// Guardrail: authored subagents may never carry plugin-system tools.
			for (const sa of params.subagents ?? []) {
				const cls = classifyAllowlist(sa.tools);
				if (cls.pluginTools.length > 0) {
					return rejectExec(
						`Subagent "${sa.name}" requests plugin-system tools (${cls.pluginTools.join(", ")}). ` +
							"Authored subagents may never carry capability-acquisition tools.",
						params.id,
					);
				}
			}

			const hasExecutable =
				(params.hooks?.length ?? 0) > 0 ||
				(params.mcpServers?.length ?? 0) > 0 ||
				(params.subagents?.length ?? 0) > 0;
			if (!hasExecutable) {
				return rejectExec("Nothing to author. Provide hooks, mcpServers, or subagents.", params.id);
			}

			if (pluginExists(ctx.cwd, params.id)) {
				return rejectExec(
					`A plugin named "${params.id}" already exists. Uninstall it first or pick another id.`,
					params.id,
				);
			}

			// Draft → display → confirm → activate. Fail closed without a UI to confirm on.
			const review = buildReview(params);
			ctx.ui.notify(review, "warning");
			if (!ctx.hasUI) {
				return rejectExec(
					"Authoring executable capabilities requires human confirmation, which is unavailable in this mode. " +
						`Not activated.\n${review}`,
					params.id,
				);
			}
			const confirmed = await ctx.ui.confirm(
				`Author executable plugin "${params.id}"?`,
				`${review}\n\nThis installs and can run the code above. Activate it?`,
			);
			if (!confirmed) {
				return {
					content: [{ type: "text" as const, text: `Declined — plugin "${params.id}" was not authored.` }],
					details: { id: params.id, authored: false, confirmed: false },
				};
			}

			const draft: PluginDraft = {
				id: params.id,
				version: params.version,
				description: params.description,
				supportPlatform: platforms,
				hooks: params.hooks,
				mcpServers: params.mcpServers,
				agents: params.subagents,
			};
			const result = writePluginDraft(ctx.cwd, draft, platforms);
			const text = summarizeWrite(params.id, platforms, result.files, result.dest);
			ctx.ui.notify(`Authored executable plugin "${params.id}" (${platforms.join(", ")}).`, "info");
			return {
				content: [{ type: "text" as const, text }],
				details: { id: params.id, authored: true, confirmed: true },
			};
		},
	});
}

function reject(
	message: string,
	id: string,
): {
	content: { type: "text"; text: string }[];
	details: ProposePluginDetails;
} {
	return { content: [{ type: "text" as const, text: message }], details: { id, authored: false } };
}

function rejectExec(
	message: string,
	id: string,
): {
	content: { type: "text"; text: string }[];
	details: ProposeExecutablePluginDetails;
} {
	return { content: [{ type: "text" as const, text: message }], details: { id, authored: false, confirmed: false } };
}

/** Both authoring tool definitions, for registration on the top-level agent. */
export function createProposePluginToolDefinitions(): ToolDefinition[] {
	return [createProposePluginToolDefinition(), createProposeExecutablePluginToolDefinition()];
}
