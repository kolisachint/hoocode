/**
 * Tool-outcome-driven thinking escalation.
 *
 * Raise the thinking level for the turn(s) following a tool error, then restore
 * it. On by default; configured via `thinking_escalation` in hoo-config.json.
 *
 * Timing model (per agent run):
 *   turn N: assistant calls tools → tool_execution_end(s) → turn_end
 *           a failing tool here escalates the level used by turn N+1
 *   turn N+1: runs escalated; its turn_end decrements the cooldown and, at zero,
 *             restores the captured baseline so turn N+2 is fast again
 *
 * `escalatedThisTurn` ensures the error turn's own `turn_end` does not consume a
 * cooldown step — the cooldown counts the turns *after* the failure.
 */

import type { ThinkingLevel } from "@kolisachint/hoocode-agent-core";
import type {
	AgentStartEvent,
	ExtensionAPI,
	ExtensionContext,
	ToolExecutionEndEvent,
	TurnEndEvent,
} from "../../core/extensions/types.js";
import { readMergedConfig } from "./config.js";

export function setupThinkingEscalation(pi: ExtensionAPI): void {
	// Captured user level to restore to; null means "not currently escalated".
	let baseline: ThinkingLevel | null = null;
	let remaining = 0;
	let escalatedThisTurn = false;

	const restore = (): void => {
		if (baseline !== null) {
			pi.setThinkingLevel(baseline);
		}
		baseline = null;
		remaining = 0;
		escalatedThisTurn = false;
	};

	// A fresh user prompt starts clean — restore any lingering escalation.
	pi.on("agent_start", (_event: AgentStartEvent) => {
		restore();
	});

	pi.on("tool_execution_end", (event: ToolExecutionEndEvent, ctx: ExtensionContext) => {
		if (!event.isError) return;

		const cfg = readMergedConfig(ctx.cwd).thinking_escalation;
		// On by default: only an explicit `enabled: false` disables it. `cfg` may be
		// undefined when no thinking_escalation block is configured at all.
		if (cfg?.enabled === false) return;
		if (cfg?.tools && cfg.tools.length > 0 && !cfg.tools.includes(event.toolName)) return;

		const target = cfg?.on_error ?? "high";
		const cooldown = Math.max(1, cfg?.cooldown_turns ?? 1);

		// Capture the user's level only on the first escalation so repeated errors
		// extend the window without overwriting the restore point with "high".
		if (baseline === null) {
			baseline = pi.getThinkingLevel();
		}
		pi.setThinkingLevel(target);
		remaining = cooldown;
		escalatedThisTurn = true;
	});

	pi.on("turn_end", (_event: TurnEndEvent) => {
		if (baseline === null) return; // not escalated
		if (escalatedThisTurn) {
			// This is the turn_end of the turn that failed; the cooldown applies to
			// subsequent turns, so don't consume a step here.
			escalatedThisTurn = false;
			return;
		}
		remaining -= 1;
		if (remaining <= 0) {
			restore();
		}
	});
}
