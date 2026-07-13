/**
 * Runtime "plugin reuse nudge" — the reactive half of the plugin-productivity
 * guidance.
 *
 * The static `promptGuidelines` on the plugin tools are injected into the system
 * prompt once at session build and never re-surface when the work actually shows
 * a reusable-facing cue. This extension closes that gap: it watches tool output
 * and turn text for the cues in {@link REUSE_NUDGES}, and when one appears it
 * attaches a matching, plugin-facing note to the *next* turn's context — an
 * ephemeral message merged into the outgoing request via the `context` hook, so
 * nothing is written into persisted history.
 *
 * Policy (see ./policy.ts for the cue table — the single source of truth):
 *   - Cues are read from `before_agent_start` (user prompt), `tool_execution_start`
 *     (tool args, e.g. content being written) and `tool_execution_end` (tool
 *     output, e.g. content being read).
 *   - First-hit arming: a matching cue arms its nudge immediately.
 *   - One note per turn, and at most once per category per session — so a cue
 *     that keeps appearing never turns into a nag.
 *   - The whole thing is gated on the autonomous-plugin-system flag
 *     (`enablePluginTools`, default off) and never blocks normal flow.
 *
 * Wired once from hoo-core (the single default composition root); the guard below
 * makes double-registration a no-op for downstreams that compose extensions
 * differently, and the static import keeps it bundled in the compiled binary.
 */

import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import type { TextContent } from "@kolisachint/hoocode-ai";
import type {
	BeforeAgentStartEvent,
	ContextEvent,
	ExtensionAPI,
	ExtensionContext,
	SessionStartEvent,
	ToolExecutionEndEvent,
	ToolExecutionStartEvent,
	TurnStartEvent,
} from "../../../core/extensions/types.js";
import {
	armReuseNudge,
	clearArmedReuseNudges,
	isAutonomousPluginSystemEnabled,
	matchReuseNudges,
	type ReuseNudge,
} from "./policy.js";

/** Guards against double-registration when default extensions load more than once. */
const REGISTERED = Symbol.for("hoocode.promptReactiveNudges.registered");

/** Cap on how much tool text we scan per event — cues are short, files can be huge. */
const SCAN_CAP = 20_000;

export interface PromptReactiveNudgesOptions {
	/**
	 * Enablement gate. Defaults to the shared autonomous-plugin-system flag so the
	 * reactive nudge and the plugin tool surface flip together. Injectable for tests.
	 */
	isEnabled?: (cwd: string) => boolean;
}

/**
 * Pull scannable text out of an arbitrary tool result or args object. Prefers
 * `content[].text` blocks (the tool-result convention) and falls back to a
 * bounded JSON stringify so cues in nested fields are still caught.
 */
function extractText(value: unknown): string {
	if (value == null) return "";
	if (typeof value === "string") return value.slice(0, SCAN_CAP);
	if (typeof value === "object") {
		const obj = value as { content?: unknown };
		if (Array.isArray(obj.content)) {
			const text = obj.content
				.map((b) =>
					b && typeof b === "object" && typeof (b as { text?: unknown }).text === "string"
						? (b as { text: string }).text
						: "",
				)
				.join("\n");
			if (text) return text.slice(0, SCAN_CAP);
		}
		try {
			return JSON.stringify(value).slice(0, SCAN_CAP);
		} catch {
			return "";
		}
	}
	return String(value).slice(0, SCAN_CAP);
}

/**
 * Merge an ephemeral reuse note into the outgoing messages. Appends it as a text
 * block on the final user turn (valid alongside tool-result blocks) so the model
 * sees it right before it responds; falls back to a fresh user message when the
 * last message isn't a user turn. The input array is never mutated.
 */
function injectNote(messages: AgentMessage[], note: string): AgentMessage[] {
	const block: TextContent = { type: "text", text: note };
	const out = messages.slice();
	const last = out[out.length - 1] as { role?: string; content?: unknown } | undefined;
	if (last && last.role === "user") {
		const content = last.content;
		const merged =
			typeof content === "string"
				? [{ type: "text", text: content } as TextContent, block]
				: Array.isArray(content)
					? [...content, block]
					: null;
		if (merged) {
			out[out.length - 1] = { ...(last as object), content: merged } as AgentMessage;
			return out;
		}
	}
	out.push({ role: "user", content: [block], timestamp: Date.now() } as AgentMessage);
	return out;
}

/** Wrap a nudge snippet so it reads as a system aside rather than user text. */
function formatNote(nudge: ReuseNudge): string {
	return `[reuse-nudge] ${nudge.snippet}`;
}

/**
 * Install the runtime reuse-nudge extension. Idempotent — a second call on the
 * same `pi` is a no-op, so composing default extensions twice is harmless.
 */
export function setupPromptReactiveNudges(pi: ExtensionAPI, options: PromptReactiveNudgesOptions = {}): void {
	const guarded = pi as unknown as Record<symbol, boolean>;
	if (guarded[REGISTERED]) return;
	guarded[REGISTERED] = true;

	const isEnabled = options.isEnabled ?? isAutonomousPluginSystemEnabled;

	// Session-scoped state. Categories fire once per session; the pending queue
	// holds armed-but-not-yet-injected nudges; injectedThisTurn caps one per turn.
	const deliveredCategories = new Set<string>();
	const pending: ReuseNudge[] = [];
	let injectedThisTurn = false;
	// Cached enablement, recomputed lazily and reset on session start.
	let enabledCache: boolean | undefined;

	const enabled = (cwd: string): boolean => {
		if (enabledCache === undefined) enabledCache = isEnabled(cwd);
		return enabledCache;
	};

	const enqueue = (text: string, cwd: string): void => {
		if (!enabled(cwd)) return;
		for (const nudge of matchReuseNudges(text)) {
			if (deliveredCategories.has(nudge.category)) continue;
			if (pending.some((p) => p.category === nudge.category)) continue;
			pending.push(nudge);
		}
	};

	pi.on("session_start", (_event: SessionStartEvent) => {
		deliveredCategories.clear();
		pending.length = 0;
		injectedThisTurn = false;
		enabledCache = undefined;
		clearArmedReuseNudges();
	});

	pi.on("turn_start", (_event: TurnStartEvent) => {
		injectedThisTurn = false;
	});

	pi.on("before_agent_start", (event: BeforeAgentStartEvent, ctx: ExtensionContext) => {
		enqueue(event.prompt, ctx.cwd);
	});

	pi.on("tool_execution_start", (event: ToolExecutionStartEvent, ctx: ExtensionContext) => {
		enqueue(extractText(event.args), ctx.cwd);
	});

	pi.on("tool_execution_end", (event: ToolExecutionEndEvent, ctx: ExtensionContext) => {
		if (event.isError) return;
		enqueue(extractText(event.result), ctx.cwd);
	});

	// The injection point: fires before each provider request. transformContext
	// output is request-scoped (never written back to agent state), so the note
	// is ephemeral by construction.
	pi.on("context", (event: ContextEvent, ctx: ExtensionContext) => {
		if (!enabled(ctx.cwd) || injectedThisTurn) return undefined;
		// Drop any that raced to "delivered" via another path, then take the oldest.
		while (pending.length > 0 && deliveredCategories.has(pending[0]!.category)) pending.shift();
		const nudge = pending.shift();
		if (!nudge) return undefined;
		deliveredCategories.add(nudge.category);
		armReuseNudge(nudge);
		injectedThisTurn = true;
		return { messages: injectNote(event.messages, formatNote(nudge)) };
	});
}
