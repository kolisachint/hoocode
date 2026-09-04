import { describe, expect, test } from "vitest";
import { buildSystemPrompt } from "../src/core/system-prompt.js";

describe("buildSystemPrompt", () => {
	describe("empty tools", () => {
		test("shows (none) for empty tools list", () => {
			const prompt = buildSystemPrompt({
				selectedTools: [],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});

			expect(prompt).toContain("Available tools:\n(none)");
		});

		test("shows the code-citation guideline even with no tools", () => {
			const prompt = buildSystemPrompt({
				selectedTools: [],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});

			expect(prompt).toContain("Cite path:line when referring to code");
		});
	});

	describe("trailing session facts", () => {
		test("separates the date/cwd block from the guidelines list", () => {
			// A single \n glued "Current date:" onto the last guideline, so it
			// rendered as a malformed final bullet.
			const prompt = buildSystemPrompt({
				selectedTools: [],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});

			expect(prompt).toContain("\n\nCurrent date: ");
			expect(prompt).not.toMatch(/[^\n]\nCurrent date: /);
		});
	});

	describe("output constraints", () => {
		test("includes the default output-constraint guidelines", () => {
			const prompt = buildSystemPrompt({
				selectedTools: [],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});

			// One merged concision bullet, not three — the old trio said the same
			// thing three ways and was charged three times per turn.
			expect(prompt).toContain("no restating the task or summarizing what you just did");
			expect(prompt).toContain('"Let me know"');
			expect(prompt).not.toContain("Be concise in your responses");
			expect(prompt).toContain("Do not narrate routine tool calls or results");
			expect(prompt).toContain("Match the surrounding code's conventions");
			// Honesty about unverified or failed work is a default, not an add-on.
			expect(prompt).toContain("say so plainly");
		});
	});

	describe("default tools", () => {
		test("includes all default tools when snippets are provided", () => {
			const prompt = buildSystemPrompt({
				toolSnippets: {
					read: "Read file contents",
					bash: "Execute bash commands",
					edit: "Make surgical edits",
					write: "Create or overwrite files",
				},
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});

			expect(prompt).toContain("- read:");
			expect(prompt).toContain("- bash:");
			expect(prompt).toContain("- edit:");
			expect(prompt).toContain("- write:");
		});
	});

	describe("custom tool snippets", () => {
		test("includes custom tools in available tools section when promptSnippet is provided", () => {
			const prompt = buildSystemPrompt({
				selectedTools: ["read", "dynamic_tool"],
				toolSnippets: {
					dynamic_tool: "Run dynamic test behavior",
				},
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});

			expect(prompt).toContain("- dynamic_tool: Run dynamic test behavior");
		});

		test("omits custom tools from available tools section when promptSnippet is not provided", () => {
			const prompt = buildSystemPrompt({
				selectedTools: ["read", "dynamic_tool"],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});

			expect(prompt).not.toContain("dynamic_tool");
		});
	});

	describe("prompt guidelines", () => {
		test("appends promptGuidelines to default guidelines", () => {
			const prompt = buildSystemPrompt({
				selectedTools: ["read", "dynamic_tool"],
				promptGuidelines: ["Use dynamic_tool for project summaries."],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});

			expect(prompt).toContain("- Use dynamic_tool for project summaries.");
		});

		test("deduplicates and trims promptGuidelines", () => {
			const prompt = buildSystemPrompt({
				selectedTools: ["read", "dynamic_tool"],
				promptGuidelines: ["Use dynamic_tool for summaries.", "  Use dynamic_tool for summaries.  ", "   "],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});

			expect(prompt.match(/- Use dynamic_tool for summaries\./g)).toHaveLength(1);
		});
	});

	describe("SearchCodebase/bash routing", () => {
		test("emits the routing guideline only when both SearchCodebase and bash are active", () => {
			const both = buildSystemPrompt({
				selectedTools: ["SearchCodebase", "bash"],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});
			expect(both).toContain("shell out to rg/find/ls only for");

			const searchOnly = buildSystemPrompt({
				selectedTools: ["SearchCodebase"],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});
			expect(searchOnly).toContain("For code discovery use SearchCodebase");
			expect(searchOnly).not.toContain("shell out to rg/find/ls only for");
		});

		test("falls back to the shell guideline when SearchCodebase is absent", () => {
			const bashOnly = buildSystemPrompt({
				selectedTools: ["bash"],
				contextFiles: [],
				skills: [],
				cwd: process.cwd(),
			});
			expect(bashOnly).toContain("Use bash for file exploration");
			expect(bashOnly).not.toContain("Between SearchCodebase and bash:");
		});
	});
});
