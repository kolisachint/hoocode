import { describe, expect, it } from "vitest";
import { SUBAGENT_PROGRESS_EVENTS, SUBAGENT_STDOUT_EVENT_TYPES } from "../src/core/subagent-events.js";
import { FORWARDED_SUBAGENT_EVENTS } from "../src/core/subagent-pool.js";

describe("subagent stdout event contract", () => {
	it("parent progress forwarding stays in lockstep with the shared set", () => {
		expect(FORWARDED_SUBAGENT_EVENTS).toBe(SUBAGENT_PROGRESS_EVENTS);
	});

	it("the child emits every event the parent consumes (progress + message_end)", () => {
		// If the child stops emitting one of these at the source, the parent's
		// task panel or token budget silently goes dark — this guards that drift.
		for (const type of SUBAGENT_PROGRESS_EVENTS) {
			expect(SUBAGENT_STDOUT_EVENT_TYPES.has(type)).toBe(true);
		}
		expect(SUBAGENT_STDOUT_EVENT_TYPES.has("message_end")).toBe(true);
	});

	it("the child drops the per-delta firehose at the source", () => {
		for (const type of ["message_start", "message_update", "tool_execution_update", "turn_start"]) {
			expect(SUBAGENT_STDOUT_EVENT_TYPES.has(type)).toBe(false);
		}
	});
});
