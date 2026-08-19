/**
 * The two agent-facing canvas tools.
 *
 * Design: `docs/canvas-extensions-design.md` §11.5. Copilot names its own shape in
 * the SDK types — `list_canvas_capabilities` to discover, `invoke_canvas_action` to
 * invoke — and hoocode mirrors it, for its own reason as well as fidelity:
 * `AGENTS.md` budgets the prompt at ~4,140 tokens with ~2,710 of it tool schemas,
 * and every active tool's schema is re-sent on every request. One tool per open
 * action would make that surface grow with how many canvases are open. Two fixed
 * tools keep it flat.
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
 * Ceiling on a serialized action result, in characters.
 *
 * Whatever an action returns lands in the model's context window.
 * `pr-artifact-explorer` truncates its own payloads (`entries.slice(0, 200)`), but
 * nothing in the contract obliges a canvas to, so the host caps it too.
 */
export const CANVAS_RESULT_MAX_CHARS = 8_000;

const listParams = Type.Object({}, { additionalProperties: false });

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
		description:
			"Invoke an action on an open canvas. Actions read or change that canvas's own state and may move the view the person is looking at; they cannot edit files or run commands.",
		promptSnippet: "Act on an open canvas the user is looking at",
		parameters: invokeParams,
		async execute(_toolCallId, params: InvokeParams) {
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
				const result = await registry.invokeAction(instance, params.action, params.input as never);
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
 * The canvas tools, or none.
 *
 * Returns an empty array while nothing is open, so the two schemas are absent from
 * the prompt in the overwhelmingly common case of a repository with no canvases —
 * the same reason `registry.activeActions()` is empty until an instance exists
 * (§7). Callers re-derive this when the open set changes.
 */
export function createCanvasToolDefinitions(registry: CanvasRegistry): ToolDefinition[] {
	if (registry.listInstances().length === 0) return [];
	return [createListCapabilitiesTool(registry), createInvokeActionTool(registry)];
}
