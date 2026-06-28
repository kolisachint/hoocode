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

import { exec } from "node:child_process";
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
			description:
				"Inline flow definition object (alternative to `flow_path`). " +
				"Required fields: id (string), name (string), version (number), start_url (string), " +
				"steps (array of Step objects). Each step has: id (string), action (Action object). " +
				"Available actions: { action: 'navigate', url: string }, { action: 'click', selector: string }, " +
				"{ action: 'fill', selector: string, value_tpl: string }, { action: 'wait_settle' }, " +
				"{ action: 'checkpoint', asserts: [...] }, { action: 'decide', goal: string }. " +
				"Example: { id: 'nav', name: 'Navigate', version: 1, start_url: 'https://example.com', " +
				"steps: [{ id: 's1', action: { action: 'navigate', url: 'https://example.com' } }, " +
				"{ id: 's2', action: { action: 'wait_settle' } }] }",
		}),
	),
	vars: Type.Optional(
		Type.Record(Type.String(), Type.Unknown(), {
			description: "Variables interpolated into the flow ({{var}} placeholders).",
		}),
	),
	store: Type.Optional(Type.String({ description: "Path to the evidence store directory for this run." })),
	live_view: Type.Optional(
		Type.Boolean({
			description:
				"Start a live viewer that streams the page and the agent's tool-call log over a local " +
				"WebSocket, and auto-open it in your default browser. Set HOOCODE_BROWSERTOOLS_NO_OPEN=1 to " +
				"print the URL without opening. Best for flows that suspend or run long.",
		}),
	),
	headful: Type.Optional(
		Type.Boolean({
			description:
				"Launch a real on-screen Chromium window instead of a headless browser. Requires a desktop " +
				"display; unlike live_view it does not show the tool-call log.",
		}),
	),
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

/** Best-effort: open a URL in the OS default browser. Never throws. Suppressed by
 *  HOOCODE_BROWSERTOOLS_NO_OPEN (the URL is still surfaced to the agent). */
function openInBrowser(url: string): boolean {
	const suppress = process.env.HOOCODE_BROWSERTOOLS_NO_OPEN?.trim();
	if (suppress === "1" || suppress?.toLowerCase() === "true") return false;
	const openCmd = process.platform === "darwin" ? "open" : process.platform === "win32" ? "start" : "xdg-open";
	try {
		exec(`${openCmd} "${url}"`);
		return true;
	} catch {
		return false;
	}
}

/** Start the streamed live viewer and auto-open it. Best-effort: a failure here
 *  must not abort the flow, so it degrades to returning undefined. Returns a
 *  human-readable status line to prepend to the tool result, or undefined. */
async function startLiveView(client: BrowsertoolsServeClient): Promise<string | undefined> {
	try {
		const result = await client.request<{ url?: string; error?: string }>("live_view_start", {});
		if (!result?.url) return undefined;
		const opened = openInBrowser(result.url);
		return opened ? `Live view opened in your browser: ${result.url}` : `Live view available at: ${result.url}`;
	} catch {
		return undefined;
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
			"enabled with --enable-browsertools.\n\n" +
			"AGENTIC LOOP (preferred for exploration): for any task where you must read or navigate based on " +
			"page content, build the flow from `decide`/`extract_semantic`/`classify`/`verify_visual` steps. " +
			"Each such step SUSPENDS and hands you a screenshot of the current page. Read the screenshot, then " +
			"call browser_resume with the next action, and keep looping until the outcome is `complete`. Do " +
			"NOT fall back to webfetch/curl to read page content you could read from the screenshot — that " +
			"bypasses the live session and breaks on auth-gated or JS-rendered pages. A flow ENDS as soon as " +
			"its last step runs, so chain several `decide` steps (interleaved with `wait_settle`) when you " +
			"need a multi-step journey (search -> open result -> scroll -> extract).\n\n" +
			"RESUME RESPONSE SHAPES (browser_resume `response` field): decide_next_action -> " +
			'{ response: "next_action", action: <Action> }; classify_state -> { response: "state", state: "<label>" }; ' +
			'verify_visual -> { response: "verified", passed: true|false }; extract_semantic -> ' +
			'{ response: "extracted", fields: { <field>: <value> } }; reidentify_element -> ' +
			'{ response: "element", selector: "<css>" }.\n\n' +
			"ACTION (for next_action) is the same shape as a flow step's action: { action: 'navigate', url }, " +
			"{ action: 'click', selector, fallbacks?: string[] }, { action: 'fill', selector, value_tpl }, " +
			"{ action: 'select', selector, value_tpl }, { action: 'wait_settle' }. Prefer stable CSS/id " +
			"selectors, and ALWAYS pass a `fallbacks` array of alternate selectors for click/fill, because " +
			"the primary selector often drifts (e.g. click '.suggestion-link' with fallbacks " +
			"['a.mw-searchSuggest-link', '#typeahead-suggestions a']).\n\n" +
			"VISIBILITY: pass headful:true to launch a real on-screen browser window the user can watch; " +
			"live_view:true additionally streams a mirror + tool-call log to a local URL (set live_view:false " +
			"to suppress the mirror when the instance defaults it on).",
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
				headful: params.headful ?? opts.headful,
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

				// Bring up the live viewer before the flow runs so the page render and
				// tool-call log are visible from the first step. The per-call param wins
				// over the instance default (--enable-browser-live-preview).
				const liveViewEnabled = params.live_view ?? opts.liveView;
				const liveViewStatus = liveViewEnabled ? await startLiveView(client) : undefined;

				const outcome = await client.request<FlowOutcome>("flow_start", startParams);
				if (signal?.aborted) {
					client.dispose();
					throw new Error("Operation aborted");
				}
				const result = await advanceFlow(client, outcome, 1, opts);
				if (liveViewStatus) {
					result.content.unshift({ type: "text", text: liveViewStatus });
				}
				return result;
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
