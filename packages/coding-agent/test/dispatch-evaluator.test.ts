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

	it("prevents subagents from spawning subagents", () => {
		const prev = process.env.HOOCODE_SUBAGENT_DEPTH;
		process.env.HOOCODE_SUBAGENT_DEPTH = "1";
		try {
			const analysis = evaluator.evaluate("Implement login endpoint, write tests, do security review");
			expect(analysis.should_delegate).toBe(false);
			expect(analysis.reason).toBe("Subagents cannot spawn subagents");
		} finally {
			if (prev === undefined) {
				delete process.env.HOOCODE_SUBAGENT_DEPTH;
			} else {
				process.env.HOOCODE_SUBAGENT_DEPTH = prev;
			}
		}
	});
});
