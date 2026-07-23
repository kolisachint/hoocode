import { describe, expect, it } from "vitest";
import { createBashToolDefinition } from "../src/core/tools/bash.js";

describe("bash promptSnippet", () => {
	it("does not advertise tools that have dedicated alternatives", () => {
		const { promptSnippet } = createBashToolDefinition(process.cwd());
		// Regression guard: the old snippet said "Execute bash commands (ls, grep,
		// find, etc.)", which contradicted the system prompt's steer toward the
		// dedicated read/search/grep/find/ls tools.
		expect(promptSnippet).toBeDefined();
		expect(promptSnippet).not.toMatch(/\b(ls|grep|find|cat|head|tail|sed)\b/);
	});

	it("directs bash at its own lane (builds/tests/git/etc.)", () => {
		const { promptSnippet } = createBashToolDefinition(process.cwd());
		expect(promptSnippet).toMatch(/build|test|lint|git|package/i);
	});
});
