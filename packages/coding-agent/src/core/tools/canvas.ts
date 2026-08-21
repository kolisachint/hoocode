/**
 * The three agent-facing canvas tools.
 *
 * Design: `docs/canvas-extensions-design.md` §11.5. Copilot names its own shape in
 * the SDK types — `list_canvas_capabilities` to discover, `invoke_canvas_action` to
 * invoke — and hoocode mirrors it, for its own reason as well as fidelity:
 * `AGENTS.md` budgets the prompt at ~4,140 tokens with ~2,710 of it tool schemas,
 * and every active tool's schema is re-sent on every request. One tool per open
 * action would make that surface grow with how many canvases are open. Fixed tools
 * keep it flat.
 *
 * `reload_canvas` is hoocode's, not Copilot's, and it is what makes a canvas
 * something the agent can *iterate on* rather than only drive: an edit to
 * `extension.mjs` is invisible until the child forked from the old bytes is
 * replaced. See {@link createReloadTool} for why reloading is the agent's to do
 * while opening is not.
 *
 * **There is deliberately no "open a canvas" tool.** Opening forks a process and
 * binds a listening socket; that is a person's decision, gated by workspace trust
 * (§5). The agent drives a surface a human has already opened. This also keeps the
 * injection surface flat: a poisoned issue title rendered into a canvas can at most
 * cause an action on an instance the person chose to open.
 *
 * These are optional tools, created only when canvas support is available and at
 * least one canvas is open — so a repository without canvases pays nothing.
 */

import { type Static, Type } from "typebox";
import type { CanvasRegistry } from "../canvas/registry.js";
import { defineTool, type ToolDefinition } from "../extensions/types.js";

/** Tool name for capability discovery, matching Copilot's. */
export const LIST_CANVAS_CAPABILITIES_TOOL_NAME = "list_canvas_capabilities";
/** Tool name for action invocation, matching Copilot's. */
export const INVOKE_CANVAS_ACTION_TOOL_NAME = "invoke_canvas_action";
/**
 * Tool name for re-forking an edited extension. hoocode's own — Copilot has no
 * equivalent because its `/create-canvas` flow reloads the panel itself.
 */
export const RELOAD_CANVAS_TOOL_NAME = "reload_canvas";

/**
 * Ceiling on a serialized action result, in characters.
 *
 * Whatever an action returns lands in the model's context window.
 * `pr-artifact-explorer` truncates its own payloads (`entries.slice(0, 200)`), but
 * nothing in the contract obliges a canvas to, so the host caps it too.
 */
export const CANVAS_RESULT_MAX_CHARS = 8_000;

const listParams = Type.Object({}, { additionalProperties: false });

const reloadParams = Type.Object(
	{
		extensionId: Type.String({
			description: "The extension whose code changed. From list_canvas_capabilities (the `extension` field).",
		}),
	},
	{ additionalProperties: false },
);

type ReloadParams = Static<typeof reloadParams>;

const invokeParams = Type.Object(
	{
		instanceId: Type.String({ description: "From list_canvas_capabilities." }),
		action: Type.String({ description: "Action name declared by that instance's canvas." }),
		input: Type.Optional(Type.Unknown({ description: "Action input, matching the action's declared schema." })),
	},
	{ additionalProperties: false },
);

type InvokeParams = Static<typeof invokeParams>;

/** What `list_canvas_capabilities` reports. */
export interface CanvasCapabilitiesDetails {
	instances: number;
	actions: number;
}

/** What `reload_canvas` reports. */
export interface CanvasReloadDetails {
	extensionId: string;
	reopened: number;
	dropped: number;
}

/** What `invoke_canvas_action` reports. */
export interface CanvasInvokeDetails {
	instanceId: string;
	action: string;
	truncated: boolean;
}

function textResult(text: string) {
	return { content: [{ type: "text" as const, text }] };
}

/** Serialize an action result, capped so a chatty canvas cannot flood the context. */
function renderResult(value: unknown): { text: string; truncated: boolean } {
	const serialized = value === undefined ? "null" : JSON.stringify(value, null, 1);
	if (serialized.length <= CANVAS_RESULT_MAX_CHARS) return { text: serialized, truncated: false };
	return {
		text: `${serialized.slice(0, CANVAS_RESULT_MAX_CHARS)}\n… truncated at ${CANVAS_RESULT_MAX_CHARS} characters.`,
		truncated: true,
	};
}

/**
 * Discovery: every open instance, its canvas, and the actions it declares with
 * their input schemas.
 *
 * Takes no parameters. A filter would add schema bytes on every request to save
 * bytes in a response the model reads once.
 */
function createListCapabilitiesTool(registry: CanvasRegistry): ToolDefinition {
	return defineTool<typeof listParams, CanvasCapabilitiesDetails>({
		name: LIST_CANVAS_CAPABILITIES_TOOL_NAME,
		label: LIST_CANVAS_CAPABILITIES_TOOL_NAME,
		description:
			"List the open canvases and the actions each one accepts, with their input schemas. Call this before invoke_canvas_action to learn the instanceId and the action's schema.",
		promptSnippet: "Discover open canvases and the actions they accept",
		parameters: listParams,
		async execute() {
			const instances = registry.listInstances();
			const bindings = registry.activeActions();
			if (instances.length === 0) {
				return {
					...textResult("No canvas is open. A person opens a canvas; you can then drive it."),
					details: { instances: 0, actions: 0 },
				};
			}
			const report = instances.map((instance) => ({
				instanceId: instance.instanceId,
				canvas: instance.canvasId,
				extension: instance.extensionId,
				title: instance.title,
				status: instance.status,
				actions: bindings
					.filter((binding) => binding.instanceId === instance.instanceId)
					.map((binding) => binding.action),
			}));
			return {
				...textResult(JSON.stringify(report, null, 1)),
				details: { instances: instances.length, actions: bindings.length },
			};
		},
	});
}

/** Invocation: run one declared action against one open instance. */
function createInvokeActionTool(registry: CanvasRegistry): ToolDefinition {
	return defineTool<typeof invokeParams, CanvasInvokeDetails>({
		name: INVOKE_CANVAS_ACTION_TOOL_NAME,
		label: INVOKE_CANVAS_ACTION_TOOL_NAME,
		// The description deliberately makes no safety claim. An earlier version said
		// actions "cannot edit files or run commands", which is false: a canvas
		// extension is arbitrary Node code running with the user's privileges, and
		// `pr-artifact-explorer` really does download artifacts to disk and call the
		// GitHub API. Those side effects never pass hoocode's permission gate, because
		// the gate sits in front of hoocode's own tools, not inside a forked
		// extension — the workspace-trust gate (canvas/trust.ts) is the control here,
		// not a sentence in a tool schema. Never tell the model a safety property the
		// runtime does not enforce.
		description:
			"Invoke an action on an open canvas. Actions are implemented by the canvas extension itself: an action may change what the person is looking at and can have side effects of its own, so read the action's description before calling it.",
		promptSnippet: "Act on an open canvas the user is looking at",
		parameters: invokeParams,
		async execute(_toolCallId, params: InvokeParams, signal) {
			// instanceId is a UUID and unique across every canvas, so the model does not
			// have to carry the extension and canvas ids too — the registry already knows
			// which instance a given id belongs to.
			const instance = registry.listInstances().find((open) => open.instanceId === params.instanceId);
			if (!instance) {
				// Tools report failure by throwing here, as the built-ins do; the loop turns
				// a rejection into the model's tool result.
				const open = registry.listInstances().map((other) => other.instanceId);
				throw new Error(
					open.length === 0
						? "No canvas is open, so there is nothing to act on."
						: `No open canvas instance "${params.instanceId}". Open instances: ${open.join(", ")}.`,
				);
			}

			try {
				// Honour the turn's abort signal: without this, aborting a turn leaves the
				// request running and its answer arriving for a turn nobody awaits.
				const result = await registry.invokeAction(instance, params.action, params.input as never, { signal });
				const { text, truncated } = renderResult(result);
				return {
					...textResult(text),
					details: { instanceId: params.instanceId, action: params.action, truncated },
				};
			} catch (cause) {
				// Canvas handlers throw CanvasError with a machine-readable code, which the
				// runner preserves across the process boundary as CanvasCallError.code. Only
				// the message is rendered to the model, so fold the code into it rather than
				// letting the typed-error intent (§8) stop at the tool boundary.
				const code = cause instanceof Error && "code" in cause ? String(cause.code) : undefined;
				const message = cause instanceof Error ? cause.message : String(cause);
				throw new Error(code ? `${code}: ${message}` : message);
			}
		},
	});
}

/**
 * Reload: re-fork an extension whose source changed, carrying its open instances.
 *
 * This is the tool that makes a canvas *iterable* by the agent — "add a column",
 * "make the header sticky" — which is the whole point of authoring one in a
 * session. Editing `extension.mjs` alone changes nothing: the child forked from
 * the old bytes keeps serving until it is replaced.
 *
 * It reloads; it does not open. That distinction is what keeps §11.5's reasoning
 * intact. Opening is a person's decision because it starts a process from a
 * directory nobody has vouched for; reloading only restarts an extension the
 * person already opened, in a workspace they already trusted, from a path the
 * host already resolved. The model cannot reach a new extension through it, and
 * a poisoned string in some canvas's data still cannot cause one to start.
 *
 * It is not a safety boundary on the *contents* of the file, and must not be
 * described as one: whatever wrote `extension.mjs` — the model's own edit,
 * through the permission gate — is what runs.
 */
function createReloadTool(registry: CanvasRegistry): ToolDefinition {
	return defineTool<typeof reloadParams, CanvasReloadDetails>({
		name: RELOAD_CANVAS_TOOL_NAME,
		label: RELOAD_CANVAS_TOOL_NAME,
		description:
			"Restart an open canvas extension so your edits to its source take effect. Editing the extension's file does nothing on its own — the running process was forked from the old code. Call this after every edit. Open instances are carried across and keep their instanceId, but each gets a NEW url: tell the person the new url, because the tab they have open is now dead.",
		promptSnippet: "Restart an edited canvas so the change is live",
		parameters: reloadParams,
		async execute(_toolCallId, params: ReloadParams, signal) {
			const running = [...new Set(registry.listInstances().map((instance) => instance.extensionId))];
			if (!running.includes(params.extensionId)) {
				throw new Error(
					running.length === 0
						? "No canvas is open, so there is nothing to reload."
						: `Canvas extension "${params.extensionId}" has nothing open. Running: ${running.join(", ")}.`,
				);
			}

			// A failed reload is the common case while iterating — the edit did not
			// parse, or threw at module scope. The registry leaves the old child
			// serving in that case, so this reads as "your edit is broken and the
			// canvas is untouched", which is what the model needs to hear to fix it.
			const result = await registry.reload(params.extensionId, { signal });

			const lines = [`Reloaded ${params.extensionId}. It declares: ${result.canvases.join(", ") || "no canvases"}.`];
			if (result.reopened.length > 0) {
				lines.push(
					"Re-opened (give the person the new url — their old tab points at a closed port):",
					...result.reopened.map(
						(instance) =>
							`  ${instance.canvasId} (${instance.instanceId})${instance.url ? ` — ${instance.url}` : ""}`,
					),
				);
			}
			if (result.dropped.length > 0) {
				lines.push(
					"Did not come back:",
					...result.dropped.map((drop) => `  ${drop.canvasId} (${drop.instanceId}): ${drop.reason}`),
				);
			}
			if (result.reopened.length === 0 && result.dropped.length === 0) {
				lines.push("Nothing was open, so nothing was re-opened.");
			}
			return {
				...textResult(lines.join("\n")),
				details: {
					extensionId: params.extensionId,
					reopened: result.reopened.length,
					dropped: result.dropped.length,
				},
			};
		},
	});
}

/**
 * The canvas tools, or none.
 *
 * Returns an empty array while nothing is open, so the two schemas are absent from
 * the prompt in the overwhelmingly common case of a repository with no canvases —
 * the same reason `registry.activeActions()` is empty until an instance exists
 * (§7). Callers re-derive this when the open set changes.
 */
export function createCanvasToolDefinitions(registry: CanvasRegistry): ToolDefinition[] {
	if (registry.listInstances().length === 0) return [];
	return [createListCapabilitiesTool(registry), createInvokeActionTool(registry), createReloadTool(registry)];
}
