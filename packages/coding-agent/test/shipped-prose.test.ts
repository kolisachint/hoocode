/**
 * Prose hoocode ships lives in exactly one place.
 *
 * Two prompts had drifted into second, hand-written copies inside `.ts`:
 * the four mode prompts (a `Record<string, string>` alongside the
 * `templates/modes/` files `/init` scaffolds, with genuinely different rules by
 * the time anyone looked) and the `/grill` phases. Both now read from
 * `templates/`, embedded at build time.
 *
 * These tests fail if a copy comes back. They compare against the *files*, not
 * against a literal here — a test that restates the prompt is a third copy.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { DEFAULT_MODE_PROMPTS } from "../src/core/mode-prompts.js";
import { buildTaskMainPrompt } from "../src/core/tools/subagent.js";
import { buildGrillMessage, buildSystemPrompt, type PlanSections } from "../src/extensions/core/modes.js";
import { EMBEDDED_MODES, EMBEDDED_PROMPTS } from "../src/init-templates.generated.js";

const templatesDir = join(__dirname, "..", "templates");

function template(...parts: string[]): string {
	return readFileSync(join(templatesDir, ...parts), "utf-8");
}

describe("mode prompts have one source", () => {
	it("serves the built-in defaults straight from templates/modes", () => {
		for (const mode of ["ask", "build", "debug", "plan"]) {
			expect(DEFAULT_MODE_PROMPTS[mode], mode).toBe(template("modes", mode, "system.md"));
		}
	});

	it("is what the mode resolver falls back to", () => {
		// No project or user override in this repo's temp-free path, so the
		// fallback is what a fresh install gets.
		expect(buildSystemPrompt("build", "/nonexistent", {})).toBe(DEFAULT_MODE_PROMPTS.build);
	});

	it("keeps the plan mode's substitution token", () => {
		// The mode extension replaces {{PLAN_PATH}} per session; a template that
		// loses the token silently stops telling the agent where to write.
		expect(DEFAULT_MODE_PROMPTS.plan).toContain("{{PLAN_PATH}}");
	});
});

describe("grill prompts have one source", () => {
	const sections: PlanSections = { goal: "Ship it.", raw: "Ship it." };

	it("embeds the template files verbatim", () => {
		expect(EMBEDDED_PROMPTS["grill-me"]).toBe(template("prompts", "grill-me.md"));
		expect(EMBEDDED_PROMPTS["grill-plan"]).toBe(template("prompts", "grill-plan.md"));
		expect(EMBEDDED_PROMPTS["grill-bridge"]).toBe(template("prompts", "grill-bridge.md"));
	});

	it("builds each target out of those files and nothing else", () => {
		const me = template("prompts", "grill-me.md").trim();
		const critique = template("prompts", "grill-plan.md").trim();
		const bridge = template("prompts", "grill-bridge.md").trim();

		expect(buildGrillMessage(sections, "me")).toBe(`${me}\n\n---\n\n**Goal**\nShip it.`);
		expect(buildGrillMessage(sections, "plan")).toBe(`${critique}\n\n---\n\n**Goal**\nShip it.`);
		// Both phases, in order: underspecification is upstream of plan weakness.
		expect(buildGrillMessage(sections, "both")).toBe(
			`${me}\n\n${bridge}\n\n${critique}\n\n---\n\n**Goal**\nShip it.`,
		);
	});
});

describe("the Task delegation prompt has one source", () => {
	it("is assembled from templates/prompts and nothing else", () => {
		const main = template("prompts", "task-main.md");
		// The built-in explore/plan agents are background:true, so a default cwd
		// takes the with-background branch.
		const background = template("prompts", "task-background-agents.md").trim();
		expect(buildTaskMainPrompt()).toBe(main.replace("{{BACKGROUND_GUIDANCE}}", background).trim());
	});

	it("keeps the substitution token the two background variants fill", () => {
		// Losing the token silently drops the background/barrier guidance rather
		// than failing: the replace becomes a no-op and the prompt still renders.
		expect(template("prompts", "task-main.md")).toContain("{{BACKGROUND_GUIDANCE}}");
		expect(EMBEDDED_PROMPTS["task-background-agents"]).toBeTruthy();
		expect(EMBEDDED_PROMPTS["task-background-none"]).toBeTruthy();
	});

	it("leaves no substitution token in the rendered prompt", () => {
		expect(buildTaskMainPrompt()).not.toContain("{{");
	});
});

describe("modes are embedded for the standalone binary", () => {
	it("carries every mode template", () => {
		// The Bun-compiled binary has no templates/ next to it; a mode missing from
		// the embed is a mode that silently has no prompt there.
		expect(Object.keys(EMBEDDED_MODES).sort()).toEqual(["ask", "build", "debug", "plan"]);
	});
});
