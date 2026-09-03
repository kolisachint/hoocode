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
	isAuthoredPlugin,
	mergePluginDraft,
	pluginExists,
	promoteDraft,
	removeFromPlugin,
	resolvePluginPlatforms,
	writePluginDraft,
} from "../extensions/plugins/authoring.js";
import type { PluginPlatform } from "../extensions/plugins/formats/platform-targets.js";
import type { MarketplacePlatform, PluginDraft } from "../extensions/plugins/formats/types.js";
import { formatGateFindings, runStaticGates, withFindings } from "../extensions/plugins/gates.js";
import { discardDraftDir } from "../extensions/plugins/locations.js";
import { runSmokeGate } from "../extensions/plugins/smoke.js";
import type { ExtensionContext } from "../extensions/types.js";
import { defineTool, type ToolDefinition } from "../extensions/types.js";
import {
	PROPOSE_PLUGIN_TOOL_NAME,
	REMOVE_PLUGIN_CAPABILITY_TOOL_NAME,
	UPDATE_PLUGIN_TOOL_NAME,
} from "./plugin-tool-names.js";

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
					"Comma-separated allowed-tools, e.g. 'read, search'. Read-only grants are autonomous; " +
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
	skills?: Static<typeof skillSchema>[];
	commands?: Static<typeof commandSchema>[];
	subagents?: Static<typeof subagentSchema>[];
	hooks?: Static<typeof hookSchema>[];
	mcpServers?: Static<typeof mcpServerSchema>[];
}

function resolvePlatforms(): PluginPlatform[] {
	// No model-facing platform selection: the human picks the target ecosystem
	// with --platform, defaulting to claude. A plugin is a distribution unit, so
	// there is no vendor-neutral option here. See resolvePluginPlatforms.
	return resolvePluginPlatforms();
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

/** Total capabilities carried by the draft (empty arrays count as nothing). */
function capabilityCount(params: CapabilityInput): number {
	return (
		(params.skills?.length ?? 0) +
		(params.commands?.length ?? 0) +
		(params.subagents?.length ?? 0) +
		(params.hooks?.length ?? 0) +
		(params.mcpServers?.length ?? 0)
	);
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

interface AuthorPluginDetails {
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
	evidence?: string,
): Promise<{ ok: true; gated: boolean } | { ok: false; result: ReturnType<typeof reject> }> {
	if (!hasExecutable(params)) return { ok: true, gated: false };

	// The gate results go in the prompt, not just the tool result. A human shown a
	// bare shell command has no basis to approve it; "ran the checks, here is what
	// they found" is what makes the confirmation a decision rather than a formality.
	const review = evidence ? `${buildReview(id, params)}\n\n${evidence}` : buildReview(id, params);
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
			"Author a NEW portable, reusable plugin to fill a capability gap when no marketplace plugin fits. Accepts any " +
			"capability mix — skills, slash commands, subagents, hooks, MCP servers. Authored as one self-contained " +
			"artifact usable across sessions and projects. Passive content (skills, commands, read-only " +
			"subagents) is authored autonomously; executable content (hooks, MCP servers, mutating subagents) is shown " +
			"and requires human confirmation before it activates. To change an existing plugin, use UpdatePlugin.",
		promptSnippet:
			"Author a new portable, reusable plugin to fill a capability gap (passive is autonomous; executable asks to confirm).",
		// Trimmed to the trigger and the two hard prohibitions. The craft guidance —
		// portability rules, capability-not-task naming, the one-call shape — was
		// ~600 tok/turn of always-on prose restating a how-to. It is the
		// `plugin-authoring` skill in the bundled marketplace now, read on demand.
		promptGuidelines: [
			"Sense reusability proactively: a multi-step recipe you'd plausibly repeat (or repeated twice in one session) that SearchPlugins does not already cover is worth authoring. Read the plugin-authoring skill first if it is available; it decides naming, portability and shape.",
			"Announce what you authored and why — passive content activates without asking, so the user learns it exists only if you say so.",
			"Never grant a subagent any plugin-system tool (InstallPlugin, ProposePlugin, ...); that is always rejected.",
			"Publishing to a marketplace stays a human action — never do it autonomously.",
		],
		parameters: proposeParams,
		async execute(_id, params: Static<typeof proposeParams>, _signal, _onUpdate, ctx: ExtensionContext) {
			const violation = guardrailViolation(params);
			if (violation) return reject(params.id, violation);

			if (capabilityCount(params) === 0) {
				return reject(params.id, "Nothing to author. Provide skills, commands, subagents, hooks, or mcpServers.");
			}

			const platforms = resolvePlatforms();
			// Scoped to the target platform's home: the same id on another platform is
			// a separate ecosystem artifact, not a duplicate.
			if (pluginExists(params.id, platforms)) {
				return reject(
					params.id,
					`A ${platforms.join("/")} plugin named "${params.id}" already exists. ` +
						"Use UpdatePlugin to change it, or pick another id.",
				);
			}

			// Emit to an ephemeral draft first: the gates need something real to run
			// against, and a production home is live (Claude Code loads
			// ~/.claude/skills on its next session), so nothing may land there until
			// the checks pass and — for executable content — the human agrees.
			const draft = writePluginDraft(draftFrom(params.id, params, platforms), platforms, { promote: false });
			let evaluation = runStaticGates(draft.dest, { platform: platforms[0] });
			// G3 only for executable content, and only once the cheap gates are
			// happy: no point running a hook whose command G2 already rejected.
			if (evaluation.ok && evaluation.plugin && hasExecutable(params)) {
				evaluation = withFindings(evaluation, await runSmokeGate(evaluation.plugin));
			}
			if (!evaluation.ok) {
				discardDraftDir(draft.dest);
				return reject(
					params.id,
					`Plugin "${params.id}" was not authored — it failed its checks.\n` +
						`${formatGateFindings(evaluation)}\nFix the issues and try again.`,
				);
			}

			const gate = await passExecutableGate(params.id, params, ctx, formatGateFindings(evaluation));
			if (!gate.ok) {
				discardDraftDir(draft.dest);
				return gate.result;
			}

			const dest = promoteDraft(draft.dest, params.id, platforms);
			// Passive capabilities activate live — usable on the very next model request,
			// this same turn; hooks/MCP servers activate via the reload once the turn ends.
			const activation = ctx.activatePlugin(dest);
			const text =
				`${summarizeWrite(params.id, platforms, draft.files, dest, "Authored")}\n` +
				`${formatGateFindings(evaluation)}\n${activation.message}`;
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
			"Merge inline-authored capabilities into an EXISTING locally AUTHORED plugin (marketplace-installed plugins " +
			"are refused). Skills/commands/subagents are added or replaced by name; hooks and MCP servers are unioned " +
			"with what's already there; metadata is overwritten only where you supply it. Additive only — remove a " +
			"capability with RemovePluginCapability. Nothing is fetched from a remote. Keep additions as portable as " +
			"the original, and in its existing layout. Passive additions apply autonomously; executable additions (hooks, MCP " +
			"servers, mutating subagents) require human confirmation. Use ProposePlugin to create.",
		promptSnippet:
			"Add/replace capabilities in a portable plugin you authored (additive; executable additions ask to confirm).",
		promptGuidelines: [
			"Hooks cannot be modified in place: they have no name, so supplying a changed command ADDS a second hook alongside the old one (both fire). To change a hook, RemovePluginCapability the old one first, then add the new one here.",
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
			// Authored-only: marketplace installs land in the same directory but don't
			// round-trip losslessly through our emitters (see mergePluginDraft).
			if (!isAuthoredPlugin(ctx.cwd, params.id)) {
				return reject(
					params.id,
					`Plugin "${params.id}" was not authored in this workspace (likely installed from a marketplace). ` +
						"UpdatePlugin only modifies locally authored plugins — updating a marketplace plugin is a human " +
						"action (uninstall it and install a newer version instead).",
				);
			}
			if (capabilityCount(params) === 0 && !params.version && !params.description) {
				return reject(
					params.id,
					"Nothing to update. Provide skills, commands, subagents, hooks, mcpServers, or metadata.",
				);
			}

			// Gate on the DELTA only — existing executables aren't re-confirmed.
			const gate = await passExecutableGate(params.id, params, ctx);
			if (!gate.ok) return gate.result;

			// No model-facing platform selection: a merge keeps the plugin's existing
			// layout (mergePluginDraft defaults to existing.supportPlatform).
			const result = mergePluginDraft(ctx.cwd, params.id, draftFrom(params.id, params, existing.supportPlatform));
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

// ── RemovePluginCapability (subtract from an authored plugin) ─────────────────

const hookRemovalSchema = Type.Object(
	{
		event: Type.String({ description: "Event of the hook(s) to remove, e.g. PreToolUse." }),
		matcher: Type.Optional(Type.String({ description: "Narrow to hooks with exactly this matcher." })),
		command: Type.Optional(Type.String({ description: "Narrow to hooks with exactly this command." })),
	},
	{ additionalProperties: false },
);

const removeParams = Type.Object(
	{
		id: Type.String({ description: "Id of the authored plugin to remove capabilities from." }),
		skills: Type.Optional(Type.Array(Type.String(), { description: "Skill names to remove." })),
		commands: Type.Optional(Type.Array(Type.String(), { description: "Command names to remove." })),
		subagents: Type.Optional(Type.Array(Type.String(), { description: "Subagent names to remove." })),
		mcpServers: Type.Optional(Type.Array(Type.String(), { description: "MCP server names to remove." })),
		hooks: Type.Optional(
			Type.Array(hookRemovalSchema, {
				description: "Hooks to remove, matched by event and narrowed by matcher/command when provided.",
			}),
		),
	},
	{ additionalProperties: false },
);

interface RemovePluginCapabilityDetails {
	id: string;
	removed: string[];
	missing: string[];
}

export function createRemovePluginCapabilityToolDefinition(): ToolDefinition {
	return defineTool<typeof removeParams, RemovePluginCapabilityDetails>({
		name: REMOVE_PLUGIN_CAPABILITY_TOOL_NAME,
		label: REMOVE_PLUGIN_CAPABILITY_TOOL_NAME,
		description:
			"Remove named capabilities from a locally AUTHORED plugin — skills, commands, subagents, and MCP servers by " +
			"name; hooks by event (narrowed by matcher/command). The subtractive half of UpdatePlugin. Removal is " +
			"low-risk and autonomous (deleting capabilities cannot execute code). To remove the whole plugin, use " +
			"UninstallPlugin; marketplace-installed plugins are refused here.",
		promptSnippet: "Remove capabilities from a plugin you authored (low risk; autonomous).",
		promptGuidelines: [
			"Removal runs autonomously (the low-risk direction) — announce what you removed and why.",
			"To CHANGE a hook (hooks have no name to replace by): RemovePluginCapability the old hook, then UpdatePlugin the new one (which asks for confirmation).",
		],
		parameters: removeParams,
		async execute(_id, params: Static<typeof removeParams>, _signal, _onUpdate, ctx: ExtensionContext) {
			const noDetails = (msg: string) => ({
				content: [{ type: "text" as const, text: msg }],
				details: { id: params.id, removed: [], missing: [] },
			});

			const existing = getPlugin(ctx.cwd, params.id);
			if (!existing) {
				return noDetails(`No plugin named "${params.id}" is installed.`);
			}
			if (!isAuthoredPlugin(ctx.cwd, params.id)) {
				return noDetails(
					`Plugin "${params.id}" was not authored in this workspace (likely installed from a marketplace). ` +
						"RemovePluginCapability only edits locally authored plugins — use UninstallPlugin to remove it entirely.",
				);
			}
			const requested =
				(params.skills?.length ?? 0) +
				(params.commands?.length ?? 0) +
				(params.subagents?.length ?? 0) +
				(params.mcpServers?.length ?? 0) +
				(params.hooks?.length ?? 0);
			if (requested === 0) {
				return noDetails("Nothing to remove. Name skills, commands, subagents, mcpServers, or hooks.");
			}

			const result = removeFromPlugin(ctx.cwd, params.id, {
				skills: params.skills,
				commands: params.commands,
				subagents: params.subagents,
				mcpServers: params.mcpServers,
				hooks: params.hooks,
			});
			const lines: string[] = [];
			if (result.removed.length > 0) {
				lines.push(`Removed from plugin "${params.id}":`, ...result.removed.map((r) => `  ${r}`));
			}
			if (result.missing.length > 0) {
				lines.push(`Not found (nothing removed):`, ...result.missing.map((m) => `  ${m}`));
			}
			const text = lines.join("\n");
			// Removal takes effect through the reload path, same as UninstallPlugin.
			if (result.removed.length > 0) ctx.requestReloadWhenIdle();
			ctx.ui.notify(text, result.removed.length > 0 ? "info" : "warning");
			return {
				content: [{ type: "text" as const, text }],
				details: { id: params.id, removed: result.removed, missing: result.missing },
			};
		},
	});
}

/** All three authoring tool definitions, for registration on the top-level agent. */
export function createProposePluginToolDefinitions(): ToolDefinition[] {
	return [
		createProposePluginToolDefinition(),
		createUpdatePluginToolDefinition(),
		createRemovePluginCapabilityToolDefinition(),
	];
}
