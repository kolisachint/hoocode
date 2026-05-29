/**
 * Subagent tool: delegate a focused task to a fresh, isolated agent loop.
 *
 * The tool registers a task in the shared task store (visible in the footer),
 * runs the subagent to completion, and returns ONLY the subagent's final
 * answer. It is an optional, opt-in tool (enabled via --subagent or the
 * `enableSubagent` setting); see buildSessionOptions in main.ts.
 */

import { Text } from "@kolisachint/hoocode-tui";
import { type Static, Type } from "typebox";
import type { ToolDefinition } from "../extensions/types.js";
import { defineTool } from "../extensions/types.js";
import { runSubagent, type SubagentMode } from "../subagent.js";
import { taskStore } from "../task-store.js";

const subagentParams = Type.Object({
	task: Type.String({
		description:
			"The task to delegate. Make it specific and self-contained; the subagent cannot see this conversation.",
	}),
	context: Type.String({
		description:
			'Context distilled from the conversation the subagent needs (files, constraints, prior findings). Pass "" if none.',
	}),
	mode: Type.Union(
		[
			Type.Literal("explore"),
			Type.Literal("edit"),
			Type.Literal("test"),
			Type.Literal("fix"),
			Type.Literal("review"),
		],
		{
			description:
				"explore: read-only investigation. edit: make a focused code change. test: run tests and report. fix: diagnose and fix a failure. review: read-only code review.",
		},
	),
});

type SubagentParams = Static<typeof subagentParams>;

export interface SubagentToolDetails {
	mode: SubagentMode;
	ok: boolean;
	error?: string;
}

/** First line of the task, trimmed to a short summary for the task list/footer. */
function summarize(task: string): string {
	const firstLine = task.trim().split("\n")[0] ?? "";
	return firstLine.length > 60 ? `${firstLine.slice(0, 57)}...` : firstLine || "(task)";
}

/** Create the subagent tool definition. Registered as a customTool when enabled. */
export function createSubagentToolDefinition(): ToolDefinition {
	return defineTool<typeof subagentParams, SubagentToolDetails>({
		name: "subagent",
		label: "Subagent",
		description: [
			"Delegate a focused task to a subagent that runs in a fresh, isolated context (it cannot see this conversation).",
			"Pass everything it needs via `context`. The subagent returns only its final answer.",
			"Modes: explore, edit, test, fix, review.",
			"Use it to parallelize investigation or contain a self-contained chunk of work without polluting your own context.",
		].join(" "),
		promptSnippet: "delegate a self-contained task to an isolated subagent (modes: explore/edit/test/fix/review)",
		parameters: subagentParams,

		async execute(_toolCallId, params: SubagentParams, signal, _onUpdate, ctx) {
			const mode = params.mode as SubagentMode;
			const summary = summarize(params.task);

			const task = taskStore.create(summary, { subagentMode: mode });
			taskStore.update(task.id, { status: "in_progress" });
			if (ctx.hasUI) {
				ctx.ui.setStatus("subagent", `subagent:${mode} ${summary}`);
			}

			try {
				const result = await runSubagent({
					task: params.task,
					context: params.context,
					mode,
					cwd: ctx.cwd,
					model: ctx.model,
					modelRegistry: ctx.modelRegistry,
					signal: signal ?? ctx.signal,
				});

				if (!result.ok) {
					// Signal failure by throwing: the agent loop derives a tool's error
					// state from a thrown error, not from a returned flag.
					taskStore.update(task.id, { status: "failed" });
					throw new Error(`Subagent (${mode}) failed: ${result.error ?? "unknown error"}`);
				}

				taskStore.update(task.id, { status: "done" });
				return {
					content: [{ type: "text", text: result.answer || "(subagent returned no output)" }],
					details: { mode, ok: true },
				};
			} catch (error) {
				taskStore.update(task.id, { status: "failed" });
				throw error;
			} finally {
				if (ctx.hasUI) {
					ctx.ui.setStatus("subagent", undefined);
				}
			}
		},

		renderCall(args, theme) {
			const mode = args.mode ?? "explore";
			const preview = summarize(args.task ?? "");
			const text =
				theme.fg("toolTitle", theme.bold("subagent ")) +
				theme.fg("accent", `[${mode}]`) +
				theme.fg("dim", ` ${preview}`);
			return new Text(text, 0, 0);
		},
	});
}
