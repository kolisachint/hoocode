/**
 * DispatchEvaluator tests
 *
 * Deterministic routing logic: no LLM call, keyword + heuristic only.
 */

import { describe, expect, it } from "vitest";
import { classifyWithConfidence, DispatchEvaluator } from "../src/core/dispatch-evaluator.js";

const evaluator = new DispatchEvaluator();

describe("DispatchEvaluator", () => {
	it("routes simple single-file change inline", () => {
		const analysis = evaluator.evaluate("Add console.log to src/index.ts line 10");
		expect(analysis.should_delegate).toBe(false);
		expect(analysis.estimated_complexity).toBe("low");
	});

	it("routes exploration to explore agent", () => {
		const analysis = evaluator.evaluate("Understand how auth middleware works");
		expect(analysis.should_delegate).toBe(true);
		expect(analysis.agent_type).toBe("explore");
	});

	it("splits multi-domain tasks into subagents", () => {
		const analysis = evaluator.evaluate("Implement login endpoint, write tests, do security review");
		expect(analysis.parallelizable).toBe(true);
		const split = evaluator.shouldSplit("Implement login endpoint, write tests, do security review");
		expect(split.split).toBe(true);
		expect(split.subtasks.length).toBeGreaterThanOrEqual(3);
		const types = split.subtasks.map((s) => s.agent_type);
		expect(types).toContain("edit");
		expect(types).toContain("test");
		expect(types).toContain("review");
	});

	it("routes security audit to review", () => {
		const analysis = evaluator.evaluate("Audit for SQL injection vulnerabilities");
		expect(analysis.agent_type).toBe("review");
		expect(analysis.reason.toLowerCase()).toContain("review");
	});

	it("routes documentation to doc", () => {
		const analysis = evaluator.evaluate("Write README for API endpoints");
		expect(analysis.agent_type).toBe("doc");
	});

	it("marks complex refactor as single edit with high complexity", () => {
		const analysis = evaluator.evaluate("Refactor database layer across 10 files to use Prisma");
		expect(analysis.estimated_complexity).toBe("high");
		expect(analysis.agent_type).toBe("edit");
		expect(analysis.parallelizable).toBe(false);
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

	it("canHandleInline returns true for trivial read-only tasks", () => {
		expect(evaluator.canHandleInline("Find where auth is configured")).toBe(true);
	});

	it("canHandleInline returns false for multi-file edits", () => {
		expect(evaluator.canHandleInline("Refactor database layer across 10 files to use Prisma")).toBe(false);
	});

	it("getReason returns the analysis reason", () => {
		const analysis = evaluator.evaluate("Write tests for the new parser");
		expect(evaluator.getReason(analysis)).toBe(analysis.reason);
	});

	it("reports full confidence when all keyword signal agrees", () => {
		const analysis = evaluator.evaluate("Audit for SQL injection vulnerabilities");
		expect(analysis.agent_type).toBe("review");
		expect(analysis.confidence).toBe(1);
	});

	it("reports a confidence in [0, 1] for every analysis", () => {
		for (const task of [
			"Understand how auth middleware works",
			"Add console.log to src/index.ts line 10",
			"Refactor database layer across 10 files to use Prisma",
		]) {
			const { confidence } = evaluator.evaluate(task);
			expect(confidence).toBeGreaterThanOrEqual(0);
			expect(confidence).toBeLessThanOrEqual(1);
		}
	});

	it("breaks ties deterministically by agent priority", () => {
		// "search" (explore) and "rename" (edit) each match once — explore wins.
		const result = classifyWithConfidence("search and rename");
		expect(result.agent_type).toBe("explore");
		expect(result.confidence).toBeCloseTo(0.5, 5);
	});

	it("returns null type with zero confidence when nothing matches", () => {
		const result = classifyWithConfidence("qwerty zxcvb");
		expect(result.agent_type).toBeNull();
		expect(result.confidence).toBe(0);
	});

	it("defaults an ambiguous delegated task to explore with zero confidence", () => {
		const analysis = evaluator.evaluate("Rearchitect the whole platform across many files");
		expect(analysis.should_delegate).toBe(true);
		expect(analysis.agent_type).toBe("explore");
		expect(analysis.confidence).toBe(0);
	});
});
