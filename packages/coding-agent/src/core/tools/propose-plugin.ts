/**
 * Capability authoring tools (spec §3), refactored to a single risk-gated path.
 *
 *   ProposePlugin  author a NEW plugin from any capability mix — skills,
 *                  commands, subagents, hooks, MCP servers. The risk gate is
 *                  *computed from content*, not pre-declared by tool choice:
 *                  passive content (skills, commands, read-only subagents) is
 *                  authored autonomously; executable content (hooks, MCP servers,
 *                  mutating/high-privilege subagents) auto-triggers a "show the
 *                  code + tool grant → human confirms → activate" gate in the
 *                  same call. A mixed plugin (skill + hook) is authored in one
 *                  call, and a hook can never be mis-routed through a "passive"
 *                  tool because the gate keys off what the draft contains.
 *   UpdatePlugin   merge inline-authored capabilities into an EXISTING local
 *                  plugin. Nothing is fetched from a remote, so the supply-chain
 *                  "benign v1 → hostile v2" risk that keeps a marketplace
 *                  UpdatePlugin out of the model's hands does not apply here;
 *                  executable additions still pass through the same confirm gate.
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
	getPlugin,
	mergePluginDraft,
	pluginExists,
	resolveAuthoringPlatforms,
	writePluginDraft,
} from "../extensions/plugins/authoring.js";
import type { MarketplacePlatform, PluginDraft } from "../extensions/plugins/formats/types.js";
import type { ExtensionContext } from "../extensions/types.js";
import { defineTool, type ToolDefinition } from "../extensions/types.js";
import { PROPOSE_PLUGIN_TOOL_NAME, UPDATE_PLUGIN_TOOL_NAME } from "./plugin-tool-names.js";

export { PROPOSE_PLUGIN_TOOL_NAME, UPDATE_PLUGIN_TOOL_NAME } from "./plugin-tool-names.js";

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
			Type.String({
				description:
					"Comma-separated allowed-tools, e.g. 'read, grep, glob'. Read-only grants are autonomous; " +
					"mutating/exec/network grants (Bash, Write, Edit, MCP) or '*' require human confirmation. Omit for none.",
			}),
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

/** The capability params shared by ProposePlugin (create) and UpdatePlugin (merge). */
const capabilityProps = {
	description: Type.Optional(Type.String({ description: "Plugin description." })),
	version: Type.Optional(Type.String({ description: "Plugin version, e.g. '0.1.0'." })),
	platforms: Type.Optional(
		Type.Array(platformSchema, {
			description:
				"Formats to scaffold into. Default: the session's --support-platform targets, else claude + github " +
				"(UpdatePlugin defaults to the plugin's existing platforms).",
		}),
	),
	skills: Type.Optional(Type.Array(skillSchema)),
	commands: Type.Optional(Type.Array(commandSchema)),
	subagents: Type.Optional(
		Type.Array(subagentSchema, {
			description: "Subagents. Read-only allowlists are autonomous; mutating ones trigger human confirmation.",
		}),
	),
	hooks: Type.Optional(Type.Array(hookSchema, { description: "Shell hooks (executable — trigger confirmation)." })),
	mcpServers: Type.Optional(
		Type.Array(mcpServerSchema, { description: "MCP servers (executable — trigger confirmation)." }),
	),
} as const;

/** Union of every capability a draft can carry (used to build a draft and to classify risk). */
interface CapabilityInput {
	description?: string;
	version?: string;
	platforms?: MarketplacePlatform[];
	skills?: Static<typeof skillSchema>[];
	commands?: Static<typeof commandSchema>[];
	subagents?: Static<typeof subagentSchema>[];
	hooks?: Static<typeof hookSchema>[];
	mcpServers?: Static<typeof mcpServerSchema>[];
}

function resolvePlatforms(input: MarketplacePlatform[] | undefined): MarketplacePlatform[] {
	// Explicit tool param → session --support-platform targets → claude + github.
	return resolveAuthoringPlatforms(input);
}

function draftFrom(id: string, params: CapabilityInput, platforms: MarketplacePlatform[]): PluginDraft {
	return {
		id,
		version: params.version,
		description: params.description,
		supportPlatform: platforms,
		skills: params.skills,
		commands: params.commands,
		agents: params.subagents,
		hooks: params.hooks,
		mcpServers: params.mcpServers,
	};
}

/** The subagents whose allowlist makes them mutating/high-privilege (need the confirm gate). */
function mutatingSubagents(params: CapabilityInput): Static<typeof subagentSchema>[] {
	return (params.subagents ?? []).filter((sa) => classifyAllowlist(sa.tools).risk === "mutating");
}

/** True when the draft carries anything executable — hooks, MCP servers, or a mutating subagent. */
function hasExecutable(params: CapabilityInput): boolean {
	return (
		(params.hooks?.length ?? 0) > 0 || (params.mcpServers?.length ?? 0) > 0 || mutatingSubagents(params).length > 0
	);
}

/** Reject if any subagent carries a plugin-system tool (privilege-amplification guardrail). Returns the message, or null. */
function guardrailViolation(params: CapabilityInput): string | null {
	for (const sa of params.subagents ?? []) {
		const cls = classifyAllowlist(sa.tools);
		if (cls.pluginTools.length > 0) {
			return (
				`Subagent "${sa.name}" requests plugin-system tools (${cls.pluginTools.join(", ")}). ` +
				"Authored subagents may never carry capability-acquisition tools."
			);
		}
	}
	return null;
}

/** Build the human-facing review text: the executable code and every mutating tool grant. */
function buildReview(id: string, params: CapabilityInput): string {
	const lines: string[] = [`Plugin "${id}" wants to install executable capabilities:`];
	for (const h of params.hooks ?? []) {
		lines.push(`  hook [${h.event}${h.matcher ? ` matcher=${h.matcher}` : ""}]: ${h.command}`);
	}
	for (const s of params.mcpServers ?? []) {
		lines.push(`  mcp server "${s.name}": ${s.command}${s.args?.length ? ` ${s.args.join(" ")}` : ""}`);
	}
	for (const sa of mutatingSubagents(params)) {
		lines.push(`  subagent "${sa.name}" tools: ${sa.tools ?? "(none)"} (${classifyAllowlist(sa.tools).reason})`);
	}
	return lines.join("\n");
}

function summarizeWrite(
	id: string,
	platforms: MarketplacePlatform[],
	files: string[],
	dest: string,
	verb: string,
): string {
	return (
		`${verb} plugin "${id}" (${platforms.join(", ")}) with ${files.length} file(s) at ${dest}:\n` +
		files.map((f) => `  ${f}`).join("\n") +
		`\nRemove it with UninstallPlugin.`
	);
}

export interface AuthorPluginDetails {
	id: string;
	authored: boolean;
	/** Whether an executable-capability confirmation gate ran (and was accepted). */
	confirmed?: boolean;
}

function reject(
	id: string,
	message: string,
): {
	content: { type: "text"; text: string }[];
	details: AuthorPluginDetails;
} {
	return { content: [{ type: "text" as const, text: message }], details: { id, authored: false } };
}

/**
 * Run the shared "executable capabilities → show → confirm" gate. Returns:
 *  - `{ ok: true }` when there is nothing executable, or the human confirmed;
 *  - a tool result (authored:false) when there is no UI to confirm on, or the
 *    human declined.
 */
async function passExecutableGate(
	id: string,
	params: CapabilityInput,
	ctx: ExtensionContext,
): Promise<{ ok: true; gated: boolean } | { ok: false; result: ReturnType<typeof reject> }> {
	if (!hasExecutable(params)) return { ok: true, gated: false };

	const review = buildReview(id, params);
	ctx.ui.notify(review, "warning");
	if (!ctx.hasUI) {
		return {
			ok: false,
			result: reject(
				id,
				"Authoring executable capabilities requires human confirmation, which is unavailable in this mode. " +
					`Not activated.\n${review}`,
			),
		};
	}
	const confirmed = await ctx.ui.confirm(
		`Author executable plugin "${id}"?`,
		`${review}\n\nThis installs and can run the code above. Activate it?`,
	);
	if (!confirmed) {
		return {
			ok: false,
			result: {
				content: [{ type: "text" as const, text: `Declined — plugin "${id}" was not authored.` }],
				details: { id, authored: false, confirmed: false },
			},
		};
	}
	return { ok: true, gated: true };
}

// ── ProposePlugin (create) ────────────────────────────────────────────────────

const proposeParams = Type.Object(
	{ id: Type.String({ description: "Plugin id (directory + manifest name)." }), ...capabilityProps },
	{ additionalProperties: false },
);

export function createProposePluginToolDefinition(): ToolDefinition {
	return defineTool<typeof proposeParams, AuthorPluginDetails>({
		name: PROPOSE_PLUGIN_TOOL_NAME,
		label: PROPOSE_PLUGIN_TOOL_NAME,
		description:
			"Author a NEW plugin to fill a capability gap when no marketplace plugin fits. Accepts any capability mix — " +
			"skills, slash commands, subagents, hooks, MCP servers. Passive content (skills, commands, read-only " +
			"subagents) is authored autonomously; executable content (hooks, MCP servers, mutating subagents) is shown " +
			"and requires human confirmation before it activates. To change an existing plugin, use UpdatePlugin.",
		promptSnippet:
			"Author a new plugin to fill a capability gap (passive is autonomous; executable asks to confirm).",
		promptGuidelines: [
			"Sense reusability proactively: when you complete a multi-step recipe you'd plausibly repeat (or repeat the same pattern twice in one session) and SearchPlugins finds nothing that covers it, author it with ProposePlugin. Passive skills/commands activate immediately and are reversible with UninstallPlugin — announce what you created and why.",
			"One tool for the whole plugin: put skills + a hook in a single call. The risk gate is computed from content — you don't pre-classify. Read-only subagents and skills/commands go straight through; hooks, MCP servers, or a subagent needing Bash/Write/Edit/MCP or tools:* pause for human confirmation.",
			"Never grant a subagent any plugin-system tool (InstallPlugin, ProposePlugin, ...); that is always rejected.",
			"Publishing a proven-useful plugin to a marketplace stays a human action — do not do it autonomously.",
		],
		parameters: proposeParams,
		async execute(_id, params: Static<typeof proposeParams>, _signal, _onUpdate, ctx: ExtensionContext) {
			const violation = guardrailViolation(params);
			if (violation) return reject(params.id, violation);

			if (pluginExists(ctx.cwd, params.id)) {
				return reject(
					params.id,
					`A plugin named "${params.id}" already exists. Use UpdatePlugin to change it, or pick another id.`,
				);
			}

			const gate = await passExecutableGate(params.id, params, ctx);
			if (!gate.ok) return gate.result;

			const platforms = resolvePlatforms(params.platforms);
			const result = writePluginDraft(ctx.cwd, draftFrom(params.id, params, platforms), platforms);
			// Passive capabilities activate live — usable on the very next model request,
			// this same turn; hooks/MCP servers activate via the reload once the turn ends.
			const activation = ctx.activatePlugin(result.dest);
			const text = `${summarizeWrite(params.id, platforms, result.files, result.dest, "Authored")}\n${activation.message}`;
			ctx.ui.notify(`Authored plugin "${params.id}" (${platforms.join(", ")}).`, "info");
			return {
				content: [{ type: "text" as const, text }],
				details: { id: params.id, authored: true, confirmed: gate.gated },
			};
		},
	});
}

// ── UpdatePlugin (merge into an existing local plugin) ─────────────────────────

const updateParams = Type.Object(
	{ id: Type.String({ description: "Id of the existing local plugin to update." }), ...capabilityProps },
	{ additionalProperties: false },
);

export function createUpdatePluginToolDefinition(): ToolDefinition {
	return defineTool<typeof updateParams, AuthorPluginDetails>({
		name: UPDATE_PLUGIN_TOOL_NAME,
		label: UPDATE_PLUGIN_TOOL_NAME,
		description:
			"Merge inline-authored capabilities into an EXISTING local plugin (one you authored). Skills/commands/subagents " +
			"are added or replaced by name; hooks and MCP servers are unioned with what's already there; metadata is " +
			"overwritten only where you supply it. Nothing is fetched from a remote. Passive additions apply autonomously; " +
			"executable additions (hooks, MCP servers, mutating subagents) require human confirmation. Use ProposePlugin to create.",
		promptSnippet: "Add/replace capabilities in an existing local plugin (executable additions ask to confirm).",
		promptGuidelines: [
			"Use UpdatePlugin to grow a plugin you already authored — e.g. add a skill to it, or attach a hook. Supply only the delta; existing capabilities are preserved (a matching name replaces just that one).",
			"Only executable *additions* trigger confirmation — adding a passive skill to an already-executable plugin does not re-prompt.",
			"Never grant a subagent any plugin-system tool (InstallPlugin, ProposePlugin, ...); that is always rejected.",
		],
		parameters: updateParams,
		async execute(_id, params: Static<typeof updateParams>, _signal, _onUpdate, ctx: ExtensionContext) {
			const violation = guardrailViolation(params);
			if (violation) return reject(params.id, violation);

			const existing = getPlugin(ctx.cwd, params.id);
			if (!existing) {
				return reject(
					params.id,
					`No plugin named "${params.id}" is installed. Use ProposePlugin to create it first.`,
				);
			}
			if (
				!hasExecutable(params) &&
				!params.skills &&
				!params.commands &&
				!params.subagents &&
				!params.version &&
				!params.description
			) {
				return reject(params.id, "Nothing to update. Provide skills, commands, subagents, hooks, or mcpServers.");
			}

			// Gate on the DELTA only — existing executables aren't re-confirmed.
			const gate = await passExecutableGate(params.id, params, ctx);
			if (!gate.ok) return gate.result;

			const result = mergePluginDraft(
				ctx.cwd,
				params.id,
				draftFrom(params.id, params, existing.supportPlatform),
				params.platforms,
			);
			const platforms = result.plugin?.supportPlatform ?? existing.supportPlatform;
			const activation = ctx.activatePlugin(result.dest);
			const text = `${summarizeWrite(params.id, platforms, result.files, result.dest, "Updated")}\n${activation.message}`;
			ctx.ui.notify(`Updated plugin "${params.id}" (${platforms.join(", ")}).`, "info");
			return {
				content: [{ type: "text" as const, text }],
				details: { id: params.id, authored: true, confirmed: gate.gated },
			};
		},
	});
}

/** Both authoring tool definitions, for registration on the top-level agent. */
export function createProposePluginToolDefinitions(): ToolDefinition[] {
	return [createProposePluginToolDefinition(), createUpdatePluginToolDefinition()];
}
