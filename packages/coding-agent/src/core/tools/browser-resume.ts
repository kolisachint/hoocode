/**
 * `browser_resume` tool: answer a `NeedsParent` suspension from `browser_flow`.
 *
 * Looks up the paused serve session by its `ResumeToken`, issues `flow_resume`
 * with the parent's decision (a `ParentResponse` object), and maps the next
 * outcome the same way `browser_flow` does — completing, failing, or suspending
 * again with a fresh token. The live serve process (and its browser state) is
 * reused across rounds. See {@link browsertools-shared}.
 */

import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { type Static, Type } from "typebox";
import { defineTool, type ToolDefinition } from "../extensions/types.js";
import { advanceFlow, type BrowserFlowDetails } from "./browser-flow.js";
import {
	type BrowsertoolsToolOptions,
	type FlowOutcome,
	resolveBrowsertoolsOptions,
	takeSession,
} from "./browsertools-shared.js";
import { wrapToolDefinition } from "./tool-definition-wrapper.js";

const browserResumeSchema = Type.Object({
	token: Type.String({ description: "The resume token returned by a browser_flow NeedsParent result." }),
	response: Type.Record(Type.String(), Type.Unknown(), {
		description:
			"The ParentResponse object answering the request, e.g. " +
			'{ "response": "state", "state": "logged_in" } or { "response": "verified", "passed": true }.',
	}),
});

export type BrowserResumeInput = Static<typeof browserResumeSchema>;

export interface BrowserResumeToolOptions extends BrowsertoolsToolOptions {}

export function createBrowserResumeToolDefinition(
	// cwd is part of the factory signature for parity with other tools, but resume
	// reuses the serve process parked by browser_flow, so it is not needed here.
	_cwd: string,
	options?: BrowserResumeToolOptions,
): ToolDefinition<typeof browserResumeSchema, BrowserFlowDetails> {
	const opts = resolveBrowsertoolsOptions(options);
	return defineTool({
		name: "browser_resume",
		label: "browser resume",
		description:
			"Resume a browser flow that suspended with a NeedsParent request. Pass the token from the " +
			"browser_flow result and a ParentResponse object answering the request. The flow continues " +
			"deterministically and either completes, fails, or suspends again with a new token — in which " +
			"case read the new screenshot and call browser_resume again, looping until the outcome is " +
			"`complete`. Do not abandon the loop to read the page with webfetch. ParentResponse by request " +
			'kind: decide_next_action -> { response: "next_action", action: <Action e.g. {action:"click", ' +
			'selector, fallbacks?}> }; classify_state -> { response: "state", state }; verify_visual -> ' +
			'{ response: "verified", passed }; extract_semantic -> { response: "extracted", fields }; ' +
			'reidentify_element -> { response: "element", selector }. Off by default; enabled with ' +
			"--enable-browsertools.",
		parameters: browserResumeSchema,
		async execute(_toolCallId, params: BrowserResumeInput, signal) {
			if (signal?.aborted) throw new Error("Operation aborted");

			const session = takeSession(params.token);
			if (!session) {
				throw new Error(
					`No paused browser flow for token "${params.token}" — it may have completed, expired (idle ` +
						`timeout), or never existed. Start a new flow with browser_flow.`,
				);
			}

			const { client } = session;
			const onAbort = () => client.dispose();
			signal?.addEventListener("abort", onAbort, { once: true });
			try {
				const outcome = await client.request<FlowOutcome>("flow_resume", {
					token: params.token,
					response: params.response,
				});
				if (signal?.aborted) {
					client.dispose();
					throw new Error("Operation aborted");
				}
				// Count this resume as one more NeedsParent round if it suspends again.
				return await advanceFlow(client, outcome, session.rounds + 1, opts);
			} catch (error) {
				client.dispose();
				throw error;
			} finally {
				signal?.removeEventListener("abort", onAbort);
			}
		},
	});
}

export function createBrowserResumeTool(
	cwd: string,
	options?: BrowserResumeToolOptions,
): AgentTool<typeof browserResumeSchema> {
	return wrapToolDefinition(createBrowserResumeToolDefinition(cwd, options));
}
