/**
 * `browser_flow` tool: start a deterministic browsertools flow.
 *
 * Spawns a `browsertools serve` process and issues `flow_start`. If the flow
 * runs to completion deterministically it returns the evidence inline. If replay
 * hits a point only an LLM can resolve, the serve process suspends with
 * `Outcome::NeedsParent`: this tool fetches the suspension screenshot, parks the
 * live session under its `ResumeToken`, and returns the typed `ParentRequest`
 * (plus the screenshot as an image) to the agent. The agent reasons and answers
 * with the companion `browser_resume` tool. See {@link browsertools-shared}.
 */

import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import type { ImageContent, TextContent } from "@kolisachint/hoocode-ai";
import { type Static, Type } from "typebox";
import type { AgentToolResult } from "../extensions/types.js";
import { defineTool, type ToolDefinition } from "../extensions/types.js";
import {
	BrowsertoolsServeClient,
	type BrowsertoolsToolOptions,
	type FlowOutcome,
	type GetResourceResult,
	type ParentRequest,
	parkSession,
	type ResumeToken,
	resolveBrowsertoolsBinary,
	resolveBrowsertoolsOptions,
} from "./browsertools-shared.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";

const browserFlowSchema = Type.Object({
	flow_path: Type.Optional(
		Type.String({ description: "Path to the .flow.json file to execute. Provide this or `flow`." }),
	),
	flow: Type.Optional(
		Type.Record(Type.String(), Type.Unknown(), {
			description: "Inline flow definition object (alternative to `flow_path`).",
		}),
	),
	vars: Type.Optional(
		Type.Record(Type.String(), Type.Unknown(), {
			description: "Variables interpolated into the flow ({{var}} placeholders).",
		}),
	),
	store: Type.Optional(Type.String({ description: "Path to the evidence store directory for this run." })),
});

export type BrowserFlowInput = Static<typeof browserFlowSchema>;

/** Structured details surfaced alongside the model-facing content. */
export interface BrowserFlowDetails {
	status: "complete" | "needs_parent";
	/** Present when status is "needs_parent": resume with this token. */
	token?: ResumeToken;
	/** Present when status is "needs_parent": the kind of parent request. */
	requestKind?: ParentRequest["request"];
	/** Present when status is "complete": the flow result/evidence. */
	result?: unknown;
}

export interface BrowserFlowToolOptions extends BrowsertoolsToolOptions {}

/** ParentResponse shape hint per ParentRequest kind (mirrors contract.rs). */
function parentResponseHint(kind: ParentRequest["request"]): string {
	switch (kind) {
		case "classify_state":
			return 'Reply with browser_resume response: { "response": "state", "state": "<your label>" }';
		case "verify_visual":
			return 'Reply with browser_resume response: { "response": "verified", "passed": true | false }';
		case "extract_semantic":
			return 'Reply with browser_resume response: { "response": "extracted", "fields": { "<field>": "<value>", ... } }';
		case "decide_next_action":
			return 'Reply with browser_resume response: { "response": "next_action", "action": <action object> }';
		case "reidentify_element":
			return 'Reply with browser_resume response: { "response": "element", "selector": "<css selector>" }';
		default:
			return "Reply with browser_resume providing the appropriate ParentResponse object.";
	}
}

/** Fetch the suspension screenshot for a ParentRequest as an ImageContent block.
 *  Best-effort: a fetch failure degrades to no image rather than failing the flow. */
async function fetchScreenshot(
	client: BrowsertoolsServeClient,
	request: ParentRequest,
): Promise<ImageContent | undefined> {
	if (!request.screenshot_ref) return undefined;
	try {
		const resource = await client.request<GetResourceResult>("get_resource", { ref: request.screenshot_ref });
		if (!resource?.png_base64) return undefined;
		return { type: "image", data: resource.png_base64, mimeType: resource.mime || "image/png" };
	} catch {
		return undefined;
	}
}

/**
 * Map a `flow_start`/`flow_resume` outcome to a tool result, owning the live
 * client: dispose it on terminal outcomes, or park it under the new resume token
 * when the flow suspends again. `rounds` is the number of NeedsParent yields seen
 * so far for this flow (including the one being processed), used for the cap.
 */
export async function advanceFlow(
	client: BrowsertoolsServeClient,
	outcome: FlowOutcome,
	rounds: number,
	opts: ReturnType<typeof resolveBrowsertoolsOptions>,
): Promise<AgentToolResult<BrowserFlowDetails>> {
	if (outcome.outcome === "complete") {
		client.dispose();
		const result = outcome.result;
		return {
			content: [{ type: "text", text: `Flow complete.\n${JSON.stringify(result ?? {}, null, 2)}` }],
			details: { status: "complete", result },
		};
	}

	if (outcome.outcome === "failed") {
		client.dispose();
		const where = outcome.step_id ? ` at step "${outcome.step_id}"` : "";
		const kind = outcome.kind ? ` (${outcome.kind})` : "";
		throw new Error(`browsertools flow failed${where}: ${outcome.detail ?? "unknown error"}${kind}`);
	}

	if (outcome.outcome === "needs_parent") {
		if (rounds > opts.maxParentRounds) {
			client.dispose();
			throw new Error(
				`browsertools flow exceeded the maximum of ${opts.maxParentRounds} NeedsParent rounds; aborting to avoid a runaway loop`,
			);
		}
		const { request, token } = outcome;
		const image = await fetchScreenshot(client, request);
		parkSession(token, client, rounds, opts.idleTimeoutMs);

		const text =
			`Flow suspended — parent decision required (NeedsParent).\n` +
			`request: ${JSON.stringify(request, null, 2)}\n` +
			`resume token: ${token}\n` +
			`${parentResponseHint(request.request)}\n` +
			(image ? "A screenshot of the current page is attached." : "(no screenshot available)");
		const content: (TextContent | ImageContent)[] = [{ type: "text", text }];
		if (image) content.push(image);
		return {
			content,
			details: { status: "needs_parent", token, requestKind: request.request },
		};
	}

	client.dispose();
	throw new Error(`browsertools returned an unrecognized flow outcome: ${JSON.stringify(outcome)}`);
}

export function createBrowserFlowToolDefinition(
	cwd: string,
	options?: BrowserFlowToolOptions,
): ToolDefinition<typeof browserFlowSchema, BrowserFlowDetails> {
	const opts = resolveBrowsertoolsOptions(options);
	return defineTool({
		name: "browser_flow",
		label: "browser flow",
		description:
			"Start a deterministic browser flow (browsertools). Runs a saved .flow.json (or inline flow) " +
			"against a headless browser and returns the evidence on completion. If the flow needs an LLM " +
			"decision mid-replay (classify a page state, verify a visual, extract a value, decide the next " +
			"action, or re-identify a drifted element) it suspends and returns a typed request plus a " +
			"screenshot; answer it with the browser_resume tool using the returned token. Off by default; " +
			"enabled with --enable-browsertools.",
		promptSnippet: "Run a deterministic browser flow, pausing for LLM decisions when needed",
		parameters: browserFlowSchema,
		async execute(_toolCallId, params: BrowserFlowInput, signal) {
			if (signal?.aborted) throw new Error("Operation aborted");
			if (!params.flow_path && !params.flow) {
				throw new Error("browser_flow requires either `flow_path` or `flow`");
			}

			const binaryPath = await resolveBrowsertoolsBinary(options);
			const client = new BrowsertoolsServeClient(binaryPath, {
				cwd,
				browserPath: opts.browserPath,
				serveArgs: opts.serveArgs,
				requestTimeoutMs: opts.requestTimeoutMs,
			});

			// If the call is aborted before we hand the client to the registry, make
			// sure the serve process is torn down.
			const onAbort = () => client.dispose();
			signal?.addEventListener("abort", onAbort, { once: true });
			try {
				const startParams: Record<string, unknown> = {};
				if (params.flow_path) startParams.flow_path = params.flow_path;
				if (params.flow) startParams.flow = params.flow;
				if (params.vars) startParams.vars = params.vars;
				if (params.store) startParams.store = params.store;

				const outcome = await client.request<FlowOutcome>("flow_start", startParams);
				if (signal?.aborted) {
					client.dispose();
					throw new Error("Operation aborted");
				}
				return await advanceFlow(client, outcome, 1, opts);
			} catch (error) {
				client.dispose();
				throw error;
			} finally {
				signal?.removeEventListener("abort", onAbort);
			}
		},
	});
}

export function createBrowserFlowTool(
	cwd: string,
	options?: BrowserFlowToolOptions,
): AgentTool<typeof browserFlowSchema> {
	return wrapToolDefinition(createBrowserFlowToolDefinition(cwd, options));
}
