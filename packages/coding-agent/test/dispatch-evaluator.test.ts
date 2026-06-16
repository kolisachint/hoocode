/**
 * DispatchEvaluator tests
 *
 * The evaluator no longer routes (the parent agent picks the subagent). It only
 * enforces the depth guard and reports a heuristic complexity estimate.
 */

import { describe, expect, it } from "vitest";
import { DispatchEvaluator } from "../src/core/dispatch-evaluator.js";

const evaluator = new DispatchEvaluator();

describe("DispatchEvaluator", () => {
	it("delegates by default", () => {
		const analysis = evaluator.evaluate("Add console.log to src/index.ts line 10");
		expect(analysis.should_delegate).toBe(true);
	});

	it("estimates low complexity for a single-file change", () => {
		const analysis = evaluator.evaluate("Add console.log to src/index.ts line 10");
		expect(analysis.estimated_complexity).toBe("low");
	});

	it("estimates high complexity for a multi-file refactor", () => {
		const analysis = evaluator.evaluate("Refactor database layer across 10 files to use Prisma");
		expect(analysis.estimated_complexity).toBe("high");
	});

	it("prevents subagents from spawning subagents at the default cap", () => {
		const prevDepth = process.env.HOOCODE_SUBAGENT_DEPTH;
		const prevMax = process.env.HOOCODE_SUBAGENT_MAX_DEPTH;
		process.env.HOOCODE_SUBAGENT_DEPTH = "1";
		delete process.env.HOOCODE_SUBAGENT_MAX_DEPTH;
		try {
			const analysis = evaluator.evaluate("Implement login endpoint, write tests, do security review");
			expect(analysis.should_delegate).toBe(false);
			expect(analysis.reason).toBe("Subagents cannot spawn subagents");
		} finally {
			restoreEnv("HOOCODE_SUBAGENT_DEPTH", prevDepth);
			restoreEnv("HOOCODE_SUBAGENT_MAX_DEPTH", prevMax);
		}
	});

	it("allows one level of nesting when the cap is raised to 2", () => {
		const prevDepth = process.env.HOOCODE_SUBAGENT_DEPTH;
		const prevMax = process.env.HOOCODE_SUBAGENT_MAX_DEPTH;
		process.env.HOOCODE_SUBAGENT_MAX_DEPTH = "2";
		try {
			// A depth-1 subagent may still delegate when the cap is 2...
			process.env.HOOCODE_SUBAGENT_DEPTH = "1";
			expect(evaluator.evaluate("Refactor module").should_delegate).toBe(true);
			// ...but the depth-2 grandchild is blocked, with a depth-aware reason.
			process.env.HOOCODE_SUBAGENT_DEPTH = "2";
			const blocked = evaluator.evaluate("Refactor module");
			expect(blocked.should_delegate).toBe(false);
			expect(blocked.reason).toBe("Maximum subagent depth (2) reached");
		} finally {
			restoreEnv("HOOCODE_SUBAGENT_DEPTH", prevDepth);
			restoreEnv("HOOCODE_SUBAGENT_MAX_DEPTH", prevMax);
		}
	});
});

function restoreEnv(key: string, value: string | undefined): void {
	if (value === undefined) {
		delete process.env[key];
	} else {
		process.env[key] = value;
	}
}
