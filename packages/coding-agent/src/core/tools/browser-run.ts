/**
 * `browser_run` tool: start a deterministic browsertools flow.
 *
 * Spawns a `browsertools serve` process and issues `flow_start`. If the flow
 * runs to completion deterministically it returns the evidence inline. If replay
 * hits a point only an LLM can resolve, the serve process suspends with
 * `Outcome::NeedsParent`: this tool fetches the suspension screenshot, parks the
 * live session under its `ResumeToken`, and returns the typed `ParentRequest`
 * (plus the screenshot as an image) to the agent. The agent reasons and answers
 * with the companion `browser_continue` tool. See {@link browsertools-shared}.
 */

import { exec } from "node:child_process";
import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import type { ImageContent, TextContent } from "@kolisachint/hoocode-ai";
import { type Static, Type } from "typebox";
import type { AgentToolResult } from "../extensions/types.js";
import { defineTool, type ToolDefinition } from "../extensions/types.js";
import {
	type BrowserClientConfig,
	BrowsertoolsServeClient,
	type BrowsertoolsToolOptions,
	type FlowOutcome,
	type GetResourceResult,
	type ParentRequest,
	parkIdleClient,
	parkSession,
	type ResumeToken,
	resolveBrowsertoolsBinary,
	resolveBrowsertoolsOptions,
	takeIdleClient,
} from "./browsertools-shared.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";

const browserRunSchema = Type.Object({
	flow_path: Type.Optional(
		Type.String({ description: "Path to the .flow.json file to execute. Provide this or `flow`." }),
	),
	flow: Type.Optional(
		Type.Record(Type.String(), Type.Unknown(), {
			description:
				"Inline flow definition object (alternative to `flow_path`). Shape: " +
				"{ id: string, name: string, version: number, start_url: string, steps: Step[] }. " +
				"Each Step is { id: string, action: Action }. Action is a tagged object keyed by " +
				"`action`; the exact fields per variant (do NOT add extras like `goal` to the wrong " +
				"variant): " +
				'{ action: "navigate", url: string } | ' +
				'{ action: "click", selector: string, fallbacks?: string[] } | ' +
				'{ action: "fill", selector: string, value_tpl: string } | ' +
				'{ action: "select", selector: string, value_tpl: string } | ' +
				'{ action: "wait_settle" } | ' +
				'{ action: "checkpoint", asserts: Invariant[] } | ' +
				'{ action: "decide", goal: string } | ' +
				'{ action: "classify" } | ' +
				'{ action: "verify_visual", expected_state: string } | ' +
				'{ action: "extract_semantic", fields: string[] }. ' +
				"IMPORTANT: `extract_semantic.fields` MUST be an array of field-name strings " +
				'(e.g. ["person", "movie_count"]), never an object/map, and `extract_semantic` takes ' +
				"no `goal` (only `decide` does). Invariant (for checkpoint asserts) is one of: " +
				'{ kind: "element_present", selector: string } | ' +
				'{ kind: "text_present", selector?: string, substr: string } | ' +
				'{ kind: "url_matches", pattern: string }.',
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

export type BrowserRunInput = Static<typeof browserRunSchema>;

/** Structured details surfaced alongside the model-facing content. */
export interface BrowserRunDetails {
	status: "complete" | "needs_parent";
	/** Present when status is "needs_parent": resume with this token. */
	token?: ResumeToken;
	/** Present when status is "needs_parent": the kind of parent request. */
	requestKind?: ParentRequest["request"];
	/** Present when status is "complete": the flow result/evidence. */
	result?: unknown;
}

export interface BrowserRunToolOptions extends BrowsertoolsToolOptions {}

/** ParentResponse shape hint per ParentRequest kind (mirrors contract.rs). */
function parentResponseHint(kind: ParentRequest["request"]): string {
	switch (kind) {
		case "classify_state":
			return 'Reply with browser_continue response: { "response": "state", "state": "<your label>" }';
		case "verify_visual":
			return 'Reply with browser_continue response: { "response": "verified", "passed": true | false }';
		case "extract_semantic":
			return 'Reply with browser_continue response: { "response": "extracted", "fields": { "<field>": "<value>", ... } }';
		case "decide_next_action":
			return 'Reply with browser_continue response: { "response": "next_action", "action": <action object> }';
		case "reidentify_element":
			return 'Reply with browser_continue response: { "response": "element", "selector": "<css selector>" }';
		default:
			return "Reply with browser_continue providing the appropriate ParentResponse object.";
	}
}

/** A control fact from the browsertools observation (mirrors observe.rs InputFact). */
interface ObservationInput {
	kind?: string;
	accessible_name?: string;
	selector_hint?: string;
}

/** A landmark fact from the browsertools observation (mirrors observe.rs Landmark). */
interface ObservationLandmark {
	role?: string;
	name?: string;
}

/** The HTML-derived observation the engine attaches to decide/classify requests. */
interface Observation {
	url?: string;
	title?: string;
	inputs?: ObservationInput[];
	landmarks?: ObservationLandmark[];
	text_blocks?: string[];
	has_error_region?: boolean;
}

/** Truncate a string to `max` chars with an ellipsis, for compact rendering. */
function clip(s: string, max = 80): string {
	const t = s.trim();
	return t.length > max ? `${t.slice(0, max - 1)}\u2026` : t;
}

/** Render one observation control as a single compact line. */
function renderControl(i: ObservationInput): string {
	const sel = i.selector_hint && i.selector_hint !== "button" ? i.selector_hint : undefined;
	const name = i.accessible_name ? `"${clip(i.accessible_name, 60)}"` : undefined;
	const parts = [i.kind, name, sel].filter(Boolean);
	return `  - ${parts.join(" ")}`;
}

/**
 * Summarize a verbose `decide_next_action`/`classify_state` observation into a
 * compact, readable block instead of dumping the raw JSON (which on busy pages is
 * thousands of mostly-empty entries straight into the model context). The
 * screenshot already conveys the page; this is just the actionable text layer:
 * url/title, named controls, salient headings. Returns undefined when there is no
 * observation to summarize.
 */
function summarizeObservation(observation: Observation | undefined): string | undefined {
	if (!observation || typeof observation !== "object") return undefined;
	const lines: string[] = [];
	if (observation.title) lines.push(`page: ${clip(observation.title, 100)}`);
	if (observation.url) lines.push(`url: ${clip(observation.url, 120)}`);
	if (observation.has_error_region) lines.push("note: page has an error/alert region");

	const controls = (observation.inputs ?? []).filter((i) => i.kind !== "_more");
	if (controls.length) {
		lines.push(`controls (${controls.length}):`);
		for (const c of controls.slice(0, 25)) lines.push(renderControl(c));
		if (controls.length > 25) lines.push(`  - …+${controls.length - 25} more`);
	}

	const text = (observation.text_blocks ?? []).filter((t) => t?.trim());
	if (text.length) {
		lines.push(
			`headings: ${text
				.slice(0, 12)
				.map((t) => clip(t, 50))
				.join(" · ")}`,
		);
	}
	return lines.length ? lines.join("\n") : undefined;
}

/**
 * Build the compact, model-facing text for a `NeedsParent` suspension. Renders
 * only the fields that matter per request kind (goal, expected_state, fields,
 * description) plus a summarized observation, rather than pretty-printing the
 * entire request — which is the dominant token sink for browser flows.
 */
function formatParentRequest(request: ParentRequest): string {
	const kind = request.request;
	const lines: string[] = [`request: ${kind}`];
	const r = request as ParentRequest & {
		goal?: string;
		expected_state?: string;
		description?: string;
		fields?: string[];
		observation?: Observation;
	};
	if (r.goal) lines.push(`goal: ${r.goal}`);
	if (r.expected_state) lines.push(`expected_state: ${r.expected_state}`);
	if (r.description) lines.push(`element: ${r.description}`);
	if (Array.isArray(r.fields) && r.fields.length) lines.push(`fields: ${r.fields.join(", ")}`);
	const summary = summarizeObservation(r.observation);
	if (summary) lines.push(summary);
	return lines.join("\n");
}

/** Compact, model-facing reminder of the inline-flow action schema, appended to
 *  an `invalid inline flow` error so the model can self-correct in one turn
 *  instead of guessing field shapes across several rounds. */
const FLOW_SCHEMA_HINT =
	"Inline flow shape: { id, name, version, start_url, steps: [{ id, action }] }. " +
	"Action variants (use EXACTLY these fields): " +
	'{ action: "navigate", url } | { action: "click", selector, fallbacks? } | ' +
	'{ action: "fill", selector, value_tpl } | { action: "select", selector, value_tpl } | ' +
	'{ action: "wait_settle" } | { action: "checkpoint", asserts } | ' +
	'{ action: "decide", goal } | { action: "classify" } | ' +
	'{ action: "verify_visual", expected_state } | { action: "extract_semantic", fields }. ' +
	'`extract_semantic.fields` MUST be a string array like ["person","count"] (not an object), ' +
	"and it takes no `goal` — only `decide` does.";

/** If an error is a browsertools inline-flow validation failure, append the
 *  schema hint so the model fixes the flow on the next call. Other errors pass
 *  through unchanged. */
function enrichFlowError(error: unknown): unknown {
	const message = error instanceof Error ? error.message : String(error);
	if (message.includes("invalid inline flow") && !message.includes("Action variants")) {
		return new Error(`${message}\n\n${FLOW_SCHEMA_HINT}`);
	}
	return error;
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
 *  human-readable status line to prepend to the tool result, or undefined.
 *
 *  When the serve client is reused across browser_run calls (the
 *  reclaimed-idle path), the viewer is already running on the same port, so we
 *  skip re-issuing `live_view_start` and skip auto-opening a second OS tab —
 *  the user keeps the one tab they already have. */
async function startLiveView(client: BrowsertoolsServeClient): Promise<string | undefined> {
	if (client.liveViewUrl) {
		return `Live view already open at: ${client.liveViewUrl}`;
	}
	try {
		const result = await client.request<{ url?: string; error?: string }>("live_view_start", {});
		if (!result?.url) return undefined;
		client.liveViewUrl = result.url;
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
	browserConfig?: BrowserClientConfig,
): Promise<AgentToolResult<BrowserRunDetails>> {
	if (outcome.outcome === "complete") {
		if (browserConfig) {
			parkIdleClient(client, browserConfig.headful, browserConfig.browserPath, browserConfig.idleTimeoutMs);
		} else {
			client.dispose();
		}
		const result = outcome.result;
		return {
			content: [{ type: "text", text: `Flow complete.\n${JSON.stringify(result ?? {}, null, 2)}` }],
			details: { status: "complete", result },
		};
	}

	if (outcome.outcome === "failed") {
		if (browserConfig) {
			parkIdleClient(client, browserConfig.headful, browserConfig.browserPath, browserConfig.idleTimeoutMs);
		} else {
			client.dispose();
		}
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
		parkSession(
			token,
			client,
			rounds,
			browserConfig?.idleTimeoutMs ?? opts.idleTimeoutMs,
			browserConfig?.headful ?? false,
			browserConfig?.browserPath,
		);

		const text =
			`Flow suspended — parent decision required (NeedsParent).\n` +
			`${formatParentRequest(request)}\n` +
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

export function createBrowserRunToolDefinition(
	cwd: string,
	options?: BrowserRunToolOptions,
): ToolDefinition<typeof browserRunSchema, BrowserRunDetails> {
	const opts = resolveBrowsertoolsOptions(options);
	return defineTool({
		name: "browser_run",
		label: "browser flow",
		description:
			"Start a deterministic browser flow (browsertools). Runs a saved .flow.json (or inline flow) " +
			"against a headless browser and returns the evidence on completion. If the flow needs an LLM " +
			"decision mid-replay (classify a page state, verify a visual, extract a value, decide the next " +
			"action, or re-identify a drifted element) it suspends and returns a typed request plus a " +
			"screenshot; answer it with the browser_continue tool using the returned token. Off by default; " +
			"enabled with --enable-browsertools.\n\n" +
			"AGENTIC LOOP (preferred for exploration): for any task where you must read or navigate based on " +
			"page content, build the flow from `decide`/`extract_semantic`/`classify`/`verify_visual` steps. " +
			"Each such step SUSPENDS and hands you a screenshot of the current page. Read the screenshot, then " +
			"call browser_continue with the next action, and keep looping until the outcome is `complete`. Do " +
			"NOT fall back to webfetch/curl to read page content you could read from the screenshot — that " +
			"bypasses the live session and breaks on auth-gated or JS-rendered pages. A flow ENDS as soon as " +
			"its last step runs, so chain several `decide` steps (interleaved with `wait_settle`) when you " +
			"need a multi-step journey (search -> open result -> scroll -> extract).\n\n" +
			"RESUME RESPONSE SHAPES (browser_continue `response` field): decide_next_action -> " +
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
		parameters: browserRunSchema,
		async execute(_toolCallId, params: BrowserRunInput, signal) {
			if (signal?.aborted) throw new Error("Operation aborted");
			if (!params.flow_path && !params.flow) {
				throw new Error("browser_run requires either `flow_path` or `flow`");
			}

			const binaryPath = await resolveBrowsertoolsBinary(options);
			const headful = params.headful ?? opts.headful;
			// Reuse the shared idle client if browser config matches, otherwise create a new
			// one. This keeps a single Chromium process and live-view port across calls.
			let client = takeIdleClient(headful, opts.browserPath, opts.idleTimeoutMs);
			if (!client) {
				client = new BrowsertoolsServeClient(binaryPath, {
					cwd,
					browserPath: opts.browserPath,
					serveArgs: opts.serveArgs,
					requestTimeoutMs: opts.requestTimeoutMs,
					headful,
				});
			}

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
				const browserConfig: BrowserClientConfig = {
					headful,
					browserPath: opts.browserPath,
					idleTimeoutMs: opts.idleTimeoutMs,
				};
				const result = await advanceFlow(client, outcome, 1, opts, browserConfig);
				if (liveViewStatus) {
					result.content.unshift({ type: "text", text: liveViewStatus });
				}
				return result;
			} catch (error) {
				client.dispose();
				throw enrichFlowError(error);
			} finally {
				signal?.removeEventListener("abort", onAbort);
			}
		},
	});
}

export function createBrowserRunTool(cwd: string, options?: BrowserRunToolOptions): AgentTool<typeof browserRunSchema> {
	return wrapToolDefinition(createBrowserRunToolDefinition(cwd, options));
}
