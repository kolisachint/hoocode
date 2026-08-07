import { describe, expect, it } from "vitest";
import {
	buildGrillMessage,
	type GrillTarget,
	parseGrillTarget,
	parsePlanSections,
} from "../src/extensions/core/modes.js";

const samplePlan = `## Goal
Add a /grill command that stress-tests the current plan.

## Files to modify
- packages/coding-agent/src/extensions/core/modes.ts

## New files
- packages/coding-agent/test/grill-command.test.ts

## Tests
Cover target parsing and message construction.

## Verification
npm run check && npm test
`;

const sections = parsePlanSections(samplePlan);

describe("parseGrillTarget", () => {
	it("defaults to both phases for a bare /grill", () => {
		expect(parseGrillTarget("")).toBe("both");
		expect(parseGrillTarget("   ")).toBe("both");
	});

	it("parses the explicit subcommands", () => {
		expect(parseGrillTarget("me")).toBe("me");
		expect(parseGrillTarget("plan")).toBe("plan");
	});

	it("tolerates surrounding whitespace and casing", () => {
		expect(parseGrillTarget("  ME  ")).toBe("me");
		expect(parseGrillTarget("Plan")).toBe("plan");
	});

	it("returns undefined for unknown arguments so the caller shows usage", () => {
		expect(parseGrillTarget("everything")).toBeUndefined();
		expect(parseGrillTarget("me plan")).toBeUndefined();
		expect(parseGrillTarget("--hard")).toBeUndefined();
	});
});

describe("buildGrillMessage", () => {
	it("asks the user via ask_options in the 'me' phase, without critiquing", () => {
		const message = buildGrillMessage(sections, "me");
		expect(message).toContain("ask_options");
		expect(message).toContain("interrogating the *request*");
		expect(message).not.toContain("attacking the plan");
	});

	it("critiques the plan in the 'plan' phase, without asking questions", () => {
		const message = buildGrillMessage(sections, "plan");
		expect(message).toContain("attacking the plan");
		expect(message).not.toContain("ask_options");
	});

	it("runs questions before critique when both phases are requested", () => {
		const message = buildGrillMessage(sections, "both");
		const askIndex = message.indexOf("ask_options");
		const critiqueIndex = message.indexOf("attacking the plan");
		expect(askIndex).toBeGreaterThan(-1);
		expect(critiqueIndex).toBeGreaterThan(-1);
		// Underspecification is upstream of plan weakness, so it must come first.
		expect(askIndex).toBeLessThan(critiqueIndex);
	});

	it("forbids implementation in every phase", () => {
		for (const target of ["both", "me", "plan"] as GrillTarget[]) {
			const message = buildGrillMessage(sections, target);
			expect(message.toLowerCase()).toMatch(/do not (edit files|implement)/);
		}
	});

	it("embeds the parsed plan sections so the agent reviews a concrete artifact", () => {
		const message = buildGrillMessage(sections, "plan");
		expect(message).toContain("Add a /grill command that stress-tests the current plan.");
		expect(message).toContain("packages/coding-agent/src/extensions/core/modes.ts");
		expect(message).toContain("npm run check && npm test");
	});

	it("falls back to the raw plan when no sections are recognised", () => {
		const unstructured = parsePlanSections("just do the thing, no headings anywhere");
		const message = buildGrillMessage(unstructured, "plan");
		expect(message).toContain("just do the thing, no headings anywhere");
	});
});
