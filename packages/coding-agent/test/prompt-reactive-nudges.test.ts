/**
 * Tests for the runtime plugin-reuse nudge (extensions/core/prompt-reactive).
 *
 * Proves the nudge becomes available after relevant tool/turn activity, that it
 * stays quiet for non-triggering work, that it fires at most once per category
 * per session, and that the enablement gate suppresses it entirely.
 */

import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import { beforeEach, describe, expect, it } from "vitest";
import type { ContextEventResult, ExtensionContext } from "../src/core/extensions/types.js";
import { setupPromptReactiveNudges } from "../src/extensions/core/prompt-reactive/nudges.js";
import {
	clearArmedReuseNudges,
	getArmedReuseNudges,
	matchReuseNudges,
} from "../src/extensions/core/prompt-reactive/policy.js";

// ── A minimal fake ExtensionAPI that records handlers so tests can emit events ──

type Handler = (event: any, ctx: ExtensionContext) => any;

function createFakePi() {
	const handlers = new Map<string, Handler[]>();
	const pi = {
		on(event: string, handler: Handler) {
			const list = handlers.get(event) ?? [];
			list.push(handler);
			handlers.set(event, list);
		},
	};
	const ctx = { cwd: "/tmp/reuse-nudge-test" } as ExtensionContext;
	const emit = (event: string, payload: Record<string, unknown> = {}): ContextEventResult | undefined => {
		let result: ContextEventResult | undefined;
		for (const h of handlers.get(event) ?? []) {
			const r = h({ type: event, ...payload }, ctx);
			if (r) result = r;
		}
		return result;
	};
	return { pi: pi as any, emit };
}

/** A single user turn, the shape transformContext hands the `context` hook. */
function userMessages(text: string): AgentMessage[] {
	return [{ role: "user", content: [{ type: "text", text }], timestamp: Date.now() }] as AgentMessage[];
}

function noteTexts(messages: AgentMessage[]): string[] {
	const out: string[] = [];
	for (const m of messages as any[]) {
		if (Array.isArray(m.content)) {
			for (const b of m.content) {
				if (b?.type === "text" && typeof b.text === "string" && b.text.includes("[reuse-nudge]")) out.push(b.text);
			}
		}
	}
	return out;
}

beforeEach(() => {
	clearArmedReuseNudges();
});

describe("reuse policy matcher", () => {
	it("matches the curated cues", () => {
		expect(matchReuseNudges("please prefer JSON for the output").map((n) => n.id)).toContain("format-prefer-json");
		expect(matchReuseNudges("write in active voice").map((n) => n.id)).toContain("style-active-voice");
		expect(matchReuseNudges("avoid repetition throughout").map((n) => n.id)).toContain("style-avoid-repetition");
	});

	it("stays quiet for benign text", () => {
		expect(matchReuseNudges("just read the file and summarize it")).toEqual([]);
		expect(matchReuseNudges("")).toEqual([]);
		expect(matchReuseNudges(undefined)).toEqual([]);
	});
});

describe("setupPromptReactiveNudges", () => {
	it("attaches a nudge after a relevant tool result and arms it for the plugin layer", () => {
		const { pi, emit } = createFakePi();
		setupPromptReactiveNudges(pi, { isEnabled: () => true });
		emit("session_start");

		// A Read whose content carries a style directive — the live-repro shape.
		emit("tool_execution_end", {
			toolName: "Read",
			result: { content: [{ type: "text", text: "House style: prefer JSON everywhere." }] },
			isError: false,
		});

		const result = emit("context", { messages: userMessages("ok, exporting now") }) as ContextEventResult;
		const notes = noteTexts(result.messages!);
		expect(notes).toHaveLength(1);
		expect(notes[0]).toContain("JSON");
		// Available to the plugin layer even though no tool asked for it.
		expect(getArmedReuseNudges().map((n) => n.id)).toContain("format-prefer-json");
	});

	it("injects from turn text via before_agent_start", () => {
		const { pi, emit } = createFakePi();
		setupPromptReactiveNudges(pi, { isEnabled: () => true });
		emit("session_start");
		emit("before_agent_start", { prompt: "rewrite the docs in active voice" });
		const result = emit("context", { messages: userMessages("working on it") }) as ContextEventResult;
		expect(noteTexts(result.messages!)).toHaveLength(1);
	});

	it("does NOT inject for non-triggering activity", () => {
		const { pi, emit } = createFakePi();
		setupPromptReactiveNudges(pi, { isEnabled: () => true });
		emit("session_start");
		emit("tool_execution_end", {
			toolName: "Read",
			result: { content: [{ type: "text", text: "const answer = 42; // nothing reusable here" }] },
			isError: false,
		});
		const result = emit("context", { messages: userMessages("done") });
		expect(result).toBeFalsy();
	});

	it("fires at most once per category per session", () => {
		const { pi, emit } = createFakePi();
		setupPromptReactiveNudges(pi, { isEnabled: () => true });
		emit("session_start");

		emit("tool_execution_end", {
			toolName: "Read",
			result: { content: [{ type: "text", text: "prefer JSON" }] },
			isError: false,
		});
		const first = emit("context", { messages: userMessages("a") }) as ContextEventResult;
		expect(noteTexts(first.messages!)).toHaveLength(1);

		// New turn, same category cue again → already delivered, so nothing.
		emit("turn_start", { turnIndex: 1, timestamp: Date.now() });
		emit("tool_execution_end", {
			toolName: "Read",
			result: { content: [{ type: "text", text: "again, prefer JSON" }] },
			isError: false,
		});
		const second = emit("context", { messages: userMessages("b") });
		expect(second).toBeFalsy();
	});

	it("injects at most one note per turn", () => {
		const { pi, emit } = createFakePi();
		setupPromptReactiveNudges(pi, { isEnabled: () => true });
		emit("session_start");

		// Two different categories armed in the same turn.
		emit("before_agent_start", { prompt: "prefer JSON and write in active voice" });
		const first = emit("context", { messages: userMessages("a") }) as ContextEventResult;
		expect(noteTexts(first.messages!)).toHaveLength(1);

		// Second provider call within the SAME turn: no second note.
		const again = emit("context", { messages: userMessages("a2") });
		expect(again).toBeFalsy();

		// Next turn: the still-pending second category injects.
		emit("turn_start", { turnIndex: 1, timestamp: Date.now() });
		const next = emit("context", { messages: userMessages("b") }) as ContextEventResult;
		expect(noteTexts(next.messages!)).toHaveLength(1);
	});

	it("stays fully silent when the autonomous plugin system is disabled", () => {
		const { pi, emit } = createFakePi();
		setupPromptReactiveNudges(pi, { isEnabled: () => false });
		emit("session_start");
		emit("tool_execution_end", {
			toolName: "Read",
			result: { content: [{ type: "text", text: "prefer JSON" }] },
			isError: false,
		});
		const result = emit("context", { messages: userMessages("done") });
		expect(result).toBeFalsy();
		expect(getArmedReuseNudges()).toEqual([]);
	});

	it("is idempotent — a second setup on the same pi does not double-register", () => {
		const { pi, emit } = createFakePi();
		setupPromptReactiveNudges(pi, { isEnabled: () => true });
		setupPromptReactiveNudges(pi, { isEnabled: () => true });
		emit("session_start");
		emit("before_agent_start", { prompt: "prefer JSON" });
		const result = emit("context", { messages: userMessages("a") }) as ContextEventResult;
		// Double-registration would append two notes; the guard keeps it at one.
		expect(noteTexts(result.messages!)).toHaveLength(1);
	});
});
