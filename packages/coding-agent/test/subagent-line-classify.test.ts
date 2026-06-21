import { describe, expect, it } from "vitest";
import { classifySubagentLine } from "../src/core/subagent-pool.js";

describe("classifySubagentLine", () => {
	it("treats a ping line as a heartbeat", () => {
		expect(classifySubagentLine(`${JSON.stringify({ ping: true })}`)).toEqual({ kind: "heartbeat" });
	});

	it("forwards coarse lifecycle events as progress", () => {
		for (const type of ["turn_end", "tool_execution_start", "tool_execution_end"]) {
			const event = { type, foo: 1 };
			expect(classifySubagentLine(JSON.stringify(event))).toEqual({ kind: "progress", event });
		}
	});

	it("drops per-delta and large-body events (the firehose)", () => {
		for (const type of ["message_start", "message_update", "message_end", "tool_execution_update", "turn_start"]) {
			expect(classifySubagentLine(JSON.stringify({ type }))).toEqual({ kind: "ignore" });
		}
	});

	it("ignores non-JSON, non-object, and empty lines without throwing", () => {
		expect(classifySubagentLine("")).toEqual({ kind: "ignore" });
		expect(classifySubagentLine("   ")).toEqual({ kind: "ignore" });
		expect(classifySubagentLine("not json")).toEqual({ kind: "ignore" });
		expect(classifySubagentLine("[1,2,3]")).toEqual({ kind: "ignore" }); // array, not an object
		expect(classifySubagentLine('{"type": "turn_end"')).toEqual({ kind: "ignore" }); // truncated JSON
	});

	it("ignores an event whose type is not a string", () => {
		expect(classifySubagentLine(JSON.stringify({ type: 42 }))).toEqual({ kind: "ignore" });
	});

	it("tolerates surrounding whitespace on an otherwise valid line", () => {
		expect(classifySubagentLine(`  ${JSON.stringify({ type: "turn_end" })}  `)).toEqual({
			kind: "progress",
			event: { type: "turn_end" },
		});
	});
});
