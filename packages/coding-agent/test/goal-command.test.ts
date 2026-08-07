import { describe, expect, it } from "vitest";
import { AUTO_LOOP_DONE_TOKEN } from "../src/extensions/core/loop.js";
import { buildGoalMessages, parseGoalArgs, parsePlanSections } from "../src/extensions/core/modes.js";

describe("parseGoalArgs", () => {
	it("treats a bare /goal as 'read the objective from the plan'", () => {
		expect(parseGoalArgs("")).toEqual({ objective: undefined, maxTurns: undefined });
		expect(parseGoalArgs("   ")).toEqual({ objective: undefined, maxTurns: undefined });
	});

	it("takes the whole argument as the objective", () => {
		expect(parseGoalArgs("make the parser tests pass")).toEqual({
			objective: "make the parser tests pass",
			maxTurns: undefined,
		});
	});

	it("parses --max-turns with and without a trailing objective", () => {
		expect(parseGoalArgs("--max-turns 25 ship the migration")).toEqual({
			objective: "ship the migration",
			maxTurns: 25,
		});
		expect(parseGoalArgs("--max-turns 3")).toEqual({ objective: undefined, maxTurns: 3 });
	});

	it("honours an explicit budget of zero rather than defaulting it", () => {
		expect(parseGoalArgs("--max-turns 0 do the thing")).toEqual({ objective: "do the thing", maxTurns: 0 });
	});

	it("rejects a malformed budget instead of silently using the default", () => {
		expect(parseGoalArgs("--max-turns")).toBeUndefined();
		expect(parseGoalArgs("--max-turns lots")).toBeUndefined();
		expect(parseGoalArgs("--max-turns -4")).toBeUndefined();
		expect(parseGoalArgs("--max-turns 2.5")).toBeUndefined();
	});

	it("only treats --max-turns as a flag at the start of the argument", () => {
		expect(parseGoalArgs("explain --max-turns to me")).toEqual({
			objective: "explain --max-turns to me",
			maxTurns: undefined,
		});
	});
});

describe("buildGoalMessages", () => {
	it("states the objective in both the opening task and the continuation", () => {
		const { task, continuePrompt } = buildGoalMessages("make the parser tests pass");
		expect(task).toContain("make the parser tests pass");
		expect(continuePrompt).toContain("make the parser tests pass");
	});

	it("asks for the done token only in the continuation, where the loop checks for it", () => {
		const { continuePrompt } = buildGoalMessages("make the parser tests pass");
		expect(continuePrompt).toContain(AUTO_LOOP_DONE_TOKEN);
	});

	it("promotes the plan's verification section into the completion condition", () => {
		const sections = parsePlanSections(`## Goal\nShip /goal.\n\n## Verification\nnpm run check && npm test\n`);
		const { task, continuePrompt } = buildGoalMessages(sections.goal ?? "", sections.verification);
		expect(task).toContain("npm run check && npm test");
		expect(continuePrompt).toContain("npm run check && npm test");
		// The condition is what makes "done" checkable rather than a judgement call.
		expect(task).toMatch(/complete only once this verification passes/i);
	});

	it("omits the verification clause entirely when the plan has none", () => {
		const { task, continuePrompt } = buildGoalMessages("make the parser tests pass");
		expect(task).not.toMatch(/verification/i);
		expect(continuePrompt).not.toMatch(/verification/i);
	});
});
