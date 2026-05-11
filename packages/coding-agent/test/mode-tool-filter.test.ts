import { describe, expect, it } from "vitest";
import {
	buildApproveMessage,
	type HooConfig,
	mergeConfigs,
	parsePlanSections,
} from "../src/extensions/core/hoo-core.js";

describe("mergeConfigs", () => {
	describe("mode enabled_tools", () => {
		it("uses global enabled_tools when project has none", () => {
			const global: HooConfig = {
				modes: {
					plan: { enabled_tools: ["read", "ls", "grep"] },
				},
			};
			const project: HooConfig = {};
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.plan?.enabled_tools).toEqual(["read", "ls", "grep"]);
		});

		it("project enabled_tools overrides global", () => {
			const global: HooConfig = {
				modes: {
					plan: { enabled_tools: ["read", "ls"] },
				},
			};
			const project: HooConfig = {
				modes: {
					plan: { enabled_tools: ["read", "grep"] },
				},
			};
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.plan?.enabled_tools).toEqual(["read", "grep"]);
		});

		it("project enabled_tools is used even when global has different mode", () => {
			const global: HooConfig = {
				modes: {
					build: { enabled_tools: ["read", "write", "edit"] },
				},
			};
			const project: HooConfig = {
				modes: {
					plan: { enabled_tools: ["read"] },
				},
			};
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.plan?.enabled_tools).toEqual(["read"]);
			expect(merged.modes?.build?.enabled_tools).toEqual(["read", "write", "edit"]);
		});
	});

	describe("mode allowed_write_paths", () => {
		it("uses global allowed_write_paths when project has none", () => {
			const global: HooConfig = {
				modes: {
					plan: { allowed_write_paths: [".hoocode/plan.md"] },
				},
			};
			const project: HooConfig = {};
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.plan?.allowed_write_paths).toEqual([".hoocode/plan.md"]);
		});

		it("unions global and project allowed_write_paths", () => {
			const global: HooConfig = {
				modes: {
					plan: { allowed_write_paths: [".hoocode/plan.md"] },
				},
			};
			const project: HooConfig = {
				modes: {
					plan: { allowed_write_paths: [".hoocode/notes.md"] },
				},
			};
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.plan?.allowed_write_paths).toEqual([".hoocode/plan.md", ".hoocode/notes.md"]);
		});

		it("removes duplicates in allowed_write_paths union", () => {
			const global: HooConfig = {
				modes: {
					plan: { allowed_write_paths: [".hoocode/plan.md"] },
				},
			};
			const project: HooConfig = {
				modes: {
					plan: { allowed_write_paths: [".hoocode/plan.md", ".hoocode/notes.md"] },
				},
			};
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.plan?.allowed_write_paths).toEqual([".hoocode/plan.md", ".hoocode/notes.md"]);
		});
	});

	describe("auto_allow still works", () => {
		it("unions auto_allow arrays", () => {
			const global: HooConfig = {
				modes: {
					build: { auto_allow: ["bash"] },
				},
			};
			const project: HooConfig = {
				modes: {
					build: { auto_allow: ["write"] },
				},
			};
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.build?.auto_allow).toEqual(["bash", "write"]);
		});
	});
});

describe("parsePlanSections", () => {
	it("parses all standard sections", () => {
		const plan = `
## Goal
Implement feature X

## Files to modify
- src/foo.ts

## New files
- src/bar.ts

## Tests
- test/foo.test.ts

## Verification
Run tests
`;
		const sections = parsePlanSections(plan);
		expect(sections.goal).toBe("Implement feature X");
		expect(sections.filesToModify).toBe("- src/foo.ts");
		expect(sections.newFiles).toBe("- src/bar.ts");
		expect(sections.tests).toBe("- test/foo.test.ts");
		expect(sections.verification).toBe("Run tests");
	});

	it("returns raw content when no sections found", () => {
		const plan = "Just some text";
		const sections = parsePlanSections(plan);
		expect(sections.raw).toBe("Just some text");
		expect(sections.goal).toBeUndefined();
	});

	it("handles bold section headers", () => {
		const plan = `
**Goal**
Implement feature X

**Files to modify**
- src/foo.ts
`;
		const sections = parsePlanSections(plan);
		expect(sections.goal).toBe("Implement feature X");
		expect(sections.filesToModify).toBe("- src/foo.ts");
	});
});

describe("buildApproveMessage", () => {
	it("builds message with all sections", () => {
		const sections = {
			goal: "Implement feature",
			filesToModify: "- src/foo.ts",
			newFiles: "- src/bar.ts",
			tests: "- Add tests",
			verification: "Run tests",
			raw: "",
		};
		const message = buildApproveMessage(sections);
		expect(message).toContain("**Goal:** Implement feature");
		expect(message).toContain("**Step 1 — Modify existing files:**");
		expect(message).toContain("**Step 2 — Create new files:**");
		expect(message).toContain("**Step 3 — Update tests:**");
		expect(message).toContain("**Step 4 — Verify:**");
	});

	it("uses raw content when no sections parsed", () => {
		const sections = {
			raw: "Just do something",
		};
		const message = buildApproveMessage(sections);
		expect(message).toBe("Execute the following plan:\n\nJust do something");
	});
});
