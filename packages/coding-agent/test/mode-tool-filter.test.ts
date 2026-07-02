import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Isolate HOOCODE_DIR before hoo-core imports so the precedence tests are
// not affected by whatever the developer has installed at ~/.hoocode/.
// vi.hoisted runs before all imports in this file.
vi.hoisted(() => {
	process.env.HOOCODE_CODING_AGENT_DIR = `${require("node:os").tmpdir()}/hoocode-test-isolated-${process.pid}/agent`;
});

import { type HooConfig, mergeConfigs } from "../src/extensions/core/config.js";
import { buildApproveMessage, buildSystemPrompt, parsePlanSections } from "../src/extensions/core/modes.js";

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

	describe("denied_tools", () => {
		it("uses global denied_tools when project has none", () => {
			const global: HooConfig = { modes: { build: { denied_tools: ["write"] } } };
			const merged = mergeConfigs(global, {});
			expect(merged.modes?.build?.denied_tools).toEqual(["write"]);
		});

		it("unions global and project denied_tools", () => {
			const global: HooConfig = { modes: { build: { denied_tools: ["write"] } } };
			const project: HooConfig = { modes: { build: { denied_tools: ["edit"] } } };
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.build?.denied_tools).toEqual(["write", "edit"]);
		});

		it("deduplicates denied_tools", () => {
			const global: HooConfig = { modes: { build: { denied_tools: ["write"] } } };
			const project: HooConfig = { modes: { build: { denied_tools: ["write", "edit"] } } };
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.build?.denied_tools).toEqual(["write", "edit"]);
		});

		it("is undefined when neither side declares it", () => {
			const merged = mergeConfigs({ modes: { build: {} } }, { modes: { build: {} } });
			expect(merged.modes?.build?.denied_tools).toEqual([]);
		});
	});

	describe("allowed_bash_commands", () => {
		it("uses global allowed_bash_commands when project has none", () => {
			const global: HooConfig = { modes: { build: { allowed_bash_commands: ["^git\\s"] } } };
			const merged = mergeConfigs(global, {});
			expect(merged.modes?.build?.allowed_bash_commands).toEqual(["^git\\s"]);
		});

		it("project allowed_bash_commands overrides global", () => {
			const global: HooConfig = { modes: { build: { allowed_bash_commands: ["^git\\s"] } } };
			const project: HooConfig = { modes: { build: { allowed_bash_commands: ["^npm\\s"] } } };
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.build?.allowed_bash_commands).toEqual(["^npm\\s"]);
		});

		it("falls back to global when project does not declare", () => {
			const global: HooConfig = { modes: { build: { allowed_bash_commands: ["^ls"] } } };
			const project: HooConfig = { modes: { build: {} } };
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.build?.allowed_bash_commands).toEqual(["^ls"]);
		});
	});

	describe("denied_bash_commands", () => {
		it("uses global denied_bash_commands when project has none", () => {
			const global: HooConfig = { modes: { build: { denied_bash_commands: ["\\brm\\b"] } } };
			const merged = mergeConfigs(global, {});
			expect(merged.modes?.build?.denied_bash_commands).toEqual(["\\brm\\b"]);
		});

		it("unions global and project denied_bash_commands", () => {
			const global: HooConfig = { modes: { build: { denied_bash_commands: ["\\brm\\b"] } } };
			const project: HooConfig = { modes: { build: { denied_bash_commands: ["\\bsudo\\b"] } } };
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.build?.denied_bash_commands).toEqual(["\\brm\\b", "\\bsudo\\b"]);
		});

		it("deduplicates denied_bash_commands", () => {
			const global: HooConfig = { modes: { build: { denied_bash_commands: ["\\brm\\b"] } } };
			const project: HooConfig = { modes: { build: { denied_bash_commands: ["\\brm\\b", "\\bsudo\\b"] } } };
			const merged = mergeConfigs(global, project);
			expect(merged.modes?.build?.denied_bash_commands).toEqual(["\\brm\\b", "\\bsudo\\b"]);
		});
	});

	describe("mode_paths", () => {
		it("project paths come before global paths and dedupe is applied", () => {
			const global: HooConfig = { mode_paths: ["/global/a", "/shared"] };
			const project: HooConfig = { mode_paths: ["/project/a", "/shared"] };
			const merged = mergeConfigs(global, project);
			expect(merged.mode_paths).toEqual(["/project/a", "/shared", "/global/a"]);
		});

		it("undefined when neither side declares them", () => {
			const merged = mergeConfigs({}, {});
			expect(merged.mode_paths).toBeUndefined();
		});

		it("uses global when project does not declare", () => {
			const merged = mergeConfigs({ mode_paths: ["/g"] }, {});
			expect(merged.mode_paths).toEqual(["/g"]);
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

describe("buildSystemPrompt search-path precedence", () => {
	let cwd: string;
	let externalA: string;
	let externalB: string;

	beforeEach(() => {
		cwd = mkdtempSync(join(tmpdir(), "hoo-cwd-"));
		externalA = mkdtempSync(join(tmpdir(), "hoo-extA-"));
		externalB = mkdtempSync(join(tmpdir(), "hoo-extB-"));
	});

	afterEach(() => {
		for (const dir of [cwd, externalA, externalB]) {
			rmSync(dir, { recursive: true, force: true });
		}
	});

	function writeMode(root: string, name: string, body: string) {
		const dir = join(root, name);
		mkdirSync(dir, { recursive: true });
		writeFileSync(join(dir, "system.md"), body, "utf8");
	}

	it("project mode wins over external dirs", () => {
		writeMode(join(cwd, ".hoocode", "modes"), "ask", "PROJECT MODE");
		writeMode(externalA, "ask", "EXTERNAL A");
		const prompt = buildSystemPrompt("ask", cwd, { modePaths: [externalA] });
		expect(prompt).toContain("PROJECT MODE");
		expect(prompt).not.toContain("EXTERNAL A");
	});

	it("external dirs are searched in declared order when project + user have no override", () => {
		writeMode(externalA, "custom", "FIRST EXTERNAL");
		writeMode(externalB, "custom", "SECOND EXTERNAL");
		const prompt = buildSystemPrompt("custom", cwd, { modePaths: [externalA, externalB] });
		expect(prompt).toBe("FIRST EXTERNAL");
	});

	it("falls through to second external when first does not have the mode", () => {
		writeMode(externalB, "only-in-b", "FROM B");
		const prompt = buildSystemPrompt("only-in-b", cwd, { modePaths: [externalA, externalB] });
		expect(prompt).toBe("FROM B");
	});

	it("falls back to MODE_DEFAULTS when nothing matches", () => {
		const prompt = buildSystemPrompt("ask", cwd, {});
		// Built-in default for "ask" mentions ASK mode
		expect(prompt).toContain("ASK mode");
	});
});
