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
import { LOOP_AUTO_CHANGED, LOOP_HALT } from "./loop.js";

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

	// Track whether an autonomous /loop is running so we never block on a human
	// who isn't there. The loop extension broadcasts this on the shared bus.
	let autoLoopActive = false;
	pi.events.on(LOOP_AUTO_CHANGED, (data) => {
		autoLoopActive = !!(data as { active?: boolean })?.active;
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

			// Autonomous loop: no human is watching, so never block on the pane.
			// Decide any question that carries a recommended default and proceed;
			// if any question lacks one, there is no safe default — halt the loop
			// and let the model report the blocker instead of guessing.
			if (autoLoopActive) {
				const blockers = params.questions.filter((q) => !q.options.some((o) => o.recommended));
				if (blockers.length) {
					const list = blockers.map((q) => `  • ${q.question}`).join("\n");
					pi.events.emit(LOOP_HALT, {
						reason: `ask_options had ${blockers.length} question(s) with no recommended default.`,
					});
					return {
						content: [
							{
								type: "text",
								text:
									`Autonomous loop: no user is available to answer, and ${blockers.length} question(s) have ` +
									`no recommended default to fall back on:\n${list}\n\n` +
									`The loop has been stopped. Do not guess — stop and report this blocker to the user, ` +
									`explaining what decision is needed and the options you were weighing.`,
							},
						],
						details: undefined,
					};
				}
				const text = params.questions
					.map((q) => {
						const rec = q.options.find((o) => o.recommended);
						return `${q.question}\n  → ${rec?.label} (auto-selected recommended default; autonomous loop, no user present)`;
					})
					.join("\n\n");
				return {
					content: [{ type: "text", text }],
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
