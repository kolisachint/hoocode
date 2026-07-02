/**
 * Options pane — the ask_options tool.
 *
 * The model calls this tool when it needs the user to make a decision before
 * continuing. Each question is shown in an inline options pane where the user
 * moves with up/down, advances with right, and may type a custom answer.
 */

import type { AgentToolResult, AgentToolUpdateCallback } from "@kolisachint/hoocode-agent-core";
import { type Static, Type } from "typebox";
import type { AskQuestion, ExtensionAPI, ExtensionContext, SessionStartEvent } from "../../core/extensions/types.js";

const askOptionsSchema = Type.Object({
	questions: Type.Array(
		Type.Object({
			question: Type.String({ description: "The question to ask the user." }),
			detail: Type.Optional(Type.String({ description: "Optional clarifying sub-text shown under the question." })),
			options: Type.Array(
				Type.Object({
					label: Type.String({ description: "The option text; returned verbatim when chosen." }),
					description: Type.Optional(
						Type.String({ description: "Optional short description shown next to the option." }),
					),
					recommended: Type.Optional(
						Type.Boolean({
							description: "When true, the option is marked '(recommended)' to help the user choose.",
						}),
					),
				}),
				{ description: "The options the user can choose from." },
			),
			allow_custom: Type.Optional(
				Type.Boolean({
					description: "When true, the user can type a free-form answer instead of choosing an option.",
				}),
			),
		}),
		{ description: "One or more decisions to ask the user, in order." },
	),
});

export function setupAskOptions(pi: ExtensionAPI): void {
	// Capture the latest context so the tool can reach the interactive UI.
	let activeCtx: ExtensionContext | undefined;
	pi.on("session_start", (_event: SessionStartEvent, ctx: ExtensionContext) => {
		activeCtx = ctx;
	});

	pi.registerTool({
		name: "ask_options",
		label: "Ask the user",
		description:
			"Ask the user to make one or more decisions before continuing. Each question is presented " +
			"in an interactive options pane where the user selects an option (or types a custom answer). " +
			"Use this when you genuinely need input to proceed and cannot reasonably decide yourself. " +
			"Returns the user's answer for each question; if the user skips, no answers are returned.",
		parameters: askOptionsSchema,
		async execute(
			_toolCallId: string,
			params: Static<typeof askOptionsSchema>,
			signal: AbortSignal,
			_onUpdate: AgentToolUpdateCallback,
		): Promise<AgentToolResult<undefined>> {
			if (!activeCtx || !activeCtx.hasUI) {
				return {
					content: [
						{
							type: "text",
							text: "Cannot ask the user: no interactive UI is available in this session. Proceed using your best judgement.",
						},
					],
					details: undefined,
				};
			}

			if (!params.questions.length) {
				return {
					content: [{ type: "text", text: "No questions were provided." }],
					details: undefined,
				};
			}

			const questions: AskQuestion[] = params.questions.map((q) => ({
				question: q.question,
				detail: q.detail,
				options: q.options.map((o) => ({ label: o.label, description: o.description, recommended: o.recommended })),
				allowCustom: q.allow_custom,
			}));

			const answers = await activeCtx.ui.askOptions(questions, { signal });

			if (!answers) {
				return {
					content: [
						{
							type: "text",
							text: "The user skipped the question(s) without answering. Ask how they would like to proceed.",
						},
					],
					details: undefined,
				};
			}

			const text = questions.map((q, i) => `${q.question}\n  → ${answers[i] ?? "(no answer)"}`).join("\n\n");
			return {
				content: [{ type: "text", text }],
				details: undefined,
			};
		},
	});
}
