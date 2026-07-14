/**
 * /loop — cron scheduler, Cron* tools, and autonomous continuation.
 *
 * `/loop` schedules prompts via cron and drives autonomous continuation. The same
 * scheduler backs the agent-callable CronCreate/CronList/CronDelete tools.
 *
 *   /loop "<cron>" <prompt>     schedule recurring (5-field cron, local time)
 *   /loop <5m|2h|1d> <prompt>   schedule recurring at a simple interval
 *   /loop once "<cron>" <prompt>  schedule a one-shot
 *   /loop list | /loop delete <id> | /loop stop
 *   /loop auto [--max-turns N] <task>   keep iterating until the task says LOOP_DONE
 */

import { join } from "node:path";
import { Type } from "typebox";
import type {
	ExtensionAPI,
	ExtensionCommandContext,
	ExtensionContext,
	SessionStartEvent,
} from "../../core/extensions/types.js";
import { defineTool } from "../../core/extensions/types.js";
import { TaskScheduler } from "../../core/scheduler.js";

const AUTO_LOOP_DONE_TOKEN = "LOOP_DONE";
const DEFAULT_AUTO_MAX_TURNS = 10;

/**
 * Event-bus channel: the autonomous-loop active state changed.
 * Payload: `{ active: boolean }`. Emitted whenever `/loop auto` starts or stops
 * so other extensions (e.g. ask_options) can adapt to running unattended.
 */
export const LOOP_AUTO_CHANGED = "loop:auto-changed";

/**
 * Event-bus channel: request to halt the autonomous loop.
 * Payload: `{ reason: string }`. Sent by another extension when it hits a
 * blocker that requires a human decision the loop cannot safely make on its own.
 */
export const LOOP_HALT = "loop:halt";

/** Convert a simple interval token ("5m", "2h", "1d") to a 5-field cron, or null. */
function intervalToCron(token: string): string | null {
	const m = /^(\d+)(m|h|d)$/.exec(token.trim());
	if (!m) return null;
	const n = Number(m[1]);
	if (n < 1) return null;
	if (m[2] === "m") return `*/${n} * * * *`;
	if (m[2] === "h") return `0 */${n} * * *`;
	return `0 0 */${n} * *`; // days
}

/** Pull a quoted cron expression off the front of an argument string. */
function extractQuotedCron(args: string): { cron: string; rest: string } | null {
	const m = /^"([^"]+)"\s*(.*)$/.exec(args.trim());
	return m ? { cron: m[1].trim(), rest: m[2].trim() } : null;
}

function isFiveFieldCron(expr: string): boolean {
	return expr.trim().split(/\s+/).length === 5;
}

/** Flatten an assistant message's text blocks. */
function assistantText(message: { content: unknown }): string {
	const content = message.content;
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	return content
		.filter((b): b is { type: "text"; text: string } => !!b && (b as { type?: string }).type === "text")
		.map((b) => b.text)
		.join("\n");
}

export function setupLoop(pi: ExtensionAPI): void {
	let scheduler: TaskScheduler | undefined;
	let auto: { remaining: number } | null = null;
	let activeCtx: ExtensionContext | undefined;

	/** Set the autonomous-loop state and broadcast the active flag on the bus. */
	function setAuto(next: { remaining: number } | null): void {
		const was = auto !== null;
		auto = next;
		if (was !== (next !== null)) pi.events.emit(LOOP_AUTO_CHANGED, { active: next !== null });
	}

	// Another extension (e.g. ask_options) hit a decision it cannot safely make
	// while unattended. Stop iterating and let the model report the blocker.
	pi.events.on(LOOP_HALT, (data) => {
		if (!auto) return;
		const reason = (data as { reason?: string })?.reason?.trim() || "a decision that needs the user.";
		setAuto(null);
		activeCtx?.ui.notify(`Autonomous loop halted: ${reason}`, "warning");
	});

	pi.on("session_start", (_event: SessionStartEvent, ctx: ExtensionContext) => {
		activeCtx = ctx;
		if (scheduler) return;
		const isIdle = () => {
			try {
				return ctx.isIdle();
			} catch {
				return true;
			}
		};
		scheduler = new TaskScheduler({
			// `.agents/` is the primary, cross-vendor home; the legacy `.hoocode/`
			// store is read once and migrates forward on the next persist.
			storePath: join(ctx.cwd, ".agents", "scheduled_tasks.json"),
			legacyStorePath: join(ctx.cwd, ".hoocode", "scheduled_tasks.json"),
			fire: (prompt) => pi.sendUserMessage(prompt, { deliverAs: "followUp" }),
			isIdle,
		});
		scheduler.start();
	});

	pi.on("session_shutdown", () => {
		scheduler?.stop();
		setAuto(null);
	});

	// Autonomous continuation: re-prompt on each agent_end until LOOP_DONE or budget.
	pi.on("agent_end", (event, ctx) => {
		if (!auto) return;
		if (ctx.hasPendingMessages()) return; // user is steering — yield
		const last = [...event.messages].reverse().find((m) => m.role === "assistant");
		const text = last ? assistantText(last) : "";
		if (text.includes(AUTO_LOOP_DONE_TOKEN)) {
			setAuto(null);
			ctx.ui.notify("Autonomous loop complete.", "info");
			return;
		}
		if (auto.remaining <= 0) {
			setAuto(null);
			ctx.ui.notify("Autonomous loop stopped: max turns reached.", "warning");
			return;
		}
		auto.remaining -= 1;
		pi.sendUserMessage(`Continue working toward the goal. Reply with ${AUTO_LOOP_DONE_TOKEN} when fully complete.`, {
			deliverAs: "followUp",
		});
	});

	// ── Cron* tools (agent-callable) ──────────────────────────────────────────
	const toolText = (s: string) => ({ content: [{ type: "text" as const, text: s }], details: undefined });

	pi.registerTool(
		defineTool({
			name: "CronCreate",
			label: "Schedule Task",
			description:
				"Schedule a prompt to be re-submitted on a cron schedule (5-field, local time: minute hour day-of-month month day-of-week). recurring=false fires once then deletes.",
			parameters: Type.Object({
				cron: Type.String({ description: "5-field cron expression in local time" }),
				prompt: Type.String({ description: "Prompt to enqueue at each fire time" }),
				recurring: Type.Optional(Type.Boolean({ description: "Fire repeatedly (default true) or once" })),
			}),
			async execute(_id, params) {
				if (!scheduler) return toolText("Scheduler not ready.");
				if (!isFiveFieldCron(params.cron)) return toolText(`Invalid cron "${params.cron}" (need 5 fields).`);
				const task = scheduler.create({
					cron: params.cron,
					prompt: params.prompt,
					recurring: params.recurring ?? true,
				});
				return toolText(`Scheduled ${task.id}: "${task.cron}" (${task.recurring ? "recurring" : "once"})`);
			},
		}),
	);

	pi.registerTool(
		defineTool({
			name: "CronList",
			label: "List Scheduled Tasks",
			description: "List all scheduled tasks (id, cron, recurring, prompt).",
			parameters: Type.Object({}),
			async execute() {
				const tasks = scheduler?.list() ?? [];
				if (tasks.length === 0) return toolText("No scheduled tasks.");
				return toolText(
					tasks
						.map((t) => `${t.id}  ${t.cron}  ${t.recurring ? "recurring" : "once"}  ${JSON.stringify(t.prompt)}`)
						.join("\n"),
				);
			},
		}),
	);

	pi.registerTool(
		defineTool({
			name: "CronDelete",
			label: "Delete Scheduled Task",
			description: "Delete a scheduled task by id.",
			parameters: Type.Object({ id: Type.String({ description: "Task id from CronCreate/CronList" }) }),
			async execute(_id, params) {
				const removed = scheduler?.delete(params.id) ?? false;
				return toolText(removed ? `Deleted ${params.id}.` : `No task ${params.id}.`);
			},
		}),
	);

	// ── /loop command ─────────────────────────────────────────────────────────
	pi.registerCommand("loop", {
		description:
			'Schedule prompts via cron or run an autonomous loop. /loop "<cron>" <prompt> | /loop <5m|2h> <prompt> | /loop once "<cron>" <prompt> | /loop list | /loop delete <id> | /loop stop | /loop auto [--max-turns N] <task>',
		getArgumentCompletions: (prefix: string) =>
			["list", "delete", "stop", "once", "auto"]
				.filter((s) => s.startsWith(prefix))
				.map((s) => ({ value: s, label: s })),
		handler: async (args: string, ctx: ExtensionCommandContext): Promise<void> => {
			const trimmed = args.trim();
			if (!scheduler) {
				ctx.ui.notify("Scheduler not ready yet.", "warning");
				return;
			}

			if (!trimmed || trimmed === "list") {
				const tasks = scheduler.list();
				ctx.ui.notify(
					tasks.length === 0
						? auto
							? `Autonomous loop active (${auto.remaining} turns left).`
							: "No scheduled tasks."
						: tasks.map((t) => `${t.id}: ${t.cron} ${t.recurring ? "" : "(once) "}— ${t.prompt}`).join("\n"),
					"info",
				);
				return;
			}

			if (trimmed === "stop") {
				const had = scheduler.list().length > 0 || auto !== null;
				scheduler.clear();
				setAuto(null);
				ctx.ui.notify(had ? "Stopped all loops and scheduled tasks." : "Nothing to stop.", "info");
				return;
			}

			if (trimmed.startsWith("delete")) {
				const id = trimmed.slice("delete".length).trim();
				if (!id) {
					ctx.ui.notify("Usage: /loop delete <id>", "warning");
					return;
				}
				ctx.ui.notify(scheduler.delete(id) ? `Deleted ${id}.` : `No task ${id}.`, "info");
				return;
			}

			if (trimmed.startsWith("auto")) {
				let rest = trimmed.slice("auto".length).trim();
				let maxTurns = DEFAULT_AUTO_MAX_TURNS;
				const flag = /^--max-turns\s+(\d+)\s*(.*)$/.exec(rest);
				if (flag) {
					maxTurns = Number(flag[1]);
					rest = flag[2].trim();
				}
				if (!rest) {
					ctx.ui.notify("Usage: /loop auto [--max-turns N] <task>", "warning");
					return;
				}
				setAuto({ remaining: maxTurns });
				pi.sendUserMessage(
					`${rest}\n\n(Autonomous loop: keep working until the task is fully complete, then reply with ${AUTO_LOOP_DONE_TOKEN}.)`,
					{ deliverAs: "followUp" },
				);
				ctx.ui.notify(`Autonomous loop started (max ${maxTurns} turns). Stop with /loop stop.`, "info");
				return;
			}

			// Scheduling: one-shot or recurring, via quoted cron or interval token.
			let recurring = true;
			let body = trimmed;
			if (body.startsWith("once")) {
				recurring = false;
				body = body.slice("once".length).trim();
			}

			let cron: string | null = null;
			let prompt = "";
			const quoted = extractQuotedCron(body);
			if (quoted) {
				cron = quoted.cron;
				prompt = quoted.rest;
			} else {
				const [first, ...restWords] = body.split(/\s+/);
				cron = intervalToCron(first);
				prompt = restWords.join(" ").trim();
			}

			if (!cron || !isFiveFieldCron(cron)) {
				ctx.ui.notify('Usage: /loop "<cron>" <prompt>  or  /loop <5m|2h|1d> <prompt>', "warning");
				return;
			}
			if (!prompt) {
				ctx.ui.notify("Nothing to schedule — provide a prompt after the schedule.", "warning");
				return;
			}

			const task = scheduler.create({ cron, prompt, recurring });
			ctx.ui.notify(
				`Scheduled ${task.id}: "${cron}" ${recurring ? "recurring" : "once"} — "${prompt}". Manage with /loop list • /loop delete ${task.id}.`,
				"info",
			);
		},
	});
}
