import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { collectAncestorAgentsSkillDirs } from "../src/core/package-resource-discovery.js";
import { loadSkills } from "../src/core/skills.js";
import { buildSystemPrompt } from "../src/core/system-prompt.js";

/**
 * Demonstrates that skills are visible to spawned subagents.
 *
 * A subagent is a fresh `hoocode` process launched by SubagentPool. It is given
 * `--system-prompt <agent body>` (the agent's frontmatter body only, NOT the full
 * prompt) and is NOT given `--no-skills`, so the child runs the same skill
 * discovery as the root. `buildSystemPrompt` then appends the discovered skill
 * cards on the customPrompt path — provided the agent has the `read` tool (skills
 * are loaded by reading their SKILL.md). This test drives those exact functions.
 */
describe("skills are visible to subagents", () => {
	let projectDir: string;
	let agentDir: string;

	beforeEach(() => {
		projectDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-subagent-skill-proj-"));
		agentDir = fs.mkdtempSync(path.join(os.tmpdir(), "hoo-subagent-skill-agent-"));
		// A project skill on the cross-vendor `.agents/` surface.
		const skillDir = path.join(projectDir, ".agents", "skills", "greeting");
		fs.mkdirSync(skillDir, { recursive: true });
		fs.writeFileSync(
			path.join(skillDir, "SKILL.md"),
			`---\nname: greeting\ndescription: Say hello in a friendly, on-brand way. Use when greeting a user.\n---\n\n# Greeting\n\nAlways greet warmly.\n`,
		);
	});

	afterEach(() => {
		fs.rmSync(projectDir, { recursive: true, force: true });
		fs.rmSync(agentDir, { recursive: true, force: true });
	});

	// Mirror the SubagentPool child launch: the agent body becomes --system-prompt,
	// skill discovery runs (no --no-skills), and the tool allowlist is the agent's.
	function buildSubagentPrompt(tools: string[]): string {
		// The PackageManager resolves `.agents/skills` dirs (ancestor-walk) and hands
		// them to loadSkills as explicit paths — this reproduces that flow so the
		// discovery matches what a real subagent process performs.
		const { skills } = loadSkills({
			cwd: projectDir,
			agentDir,
			skillPaths: collectAncestorAgentsSkillDirs(projectDir),
			includeDefaults: true,
			includeClaude: false,
		});
		return buildSystemPrompt({
			customPrompt:
				"You are an explore-only agent running inside hoocode. You read code and produce summaries. You NEVER edit files.",
			selectedTools: tools,
			skills,
			cwd: projectDir,
		});
	}

	it("discovers the project skill (same discovery the child process runs)", () => {
		// The PackageManager resolves `.agents/skills` dirs (ancestor-walk) and hands
		// them to loadSkills as explicit paths — this reproduces that flow so the
		// discovery matches what a real subagent process performs.
		const { skills } = loadSkills({
			cwd: projectDir,
			agentDir,
			skillPaths: collectAncestorAgentsSkillDirs(projectDir),
			includeDefaults: true,
			includeClaude: false,
		});
		expect(skills.map((s) => s.name)).toContain("greeting");
	});

	it("injects the skill card into a read-capable subagent's system prompt (explore/plan tools)", () => {
		const prompt = buildSubagentPrompt(["read", "grep", "find", "ls"]);
		expect(prompt).toContain("<available_skills>");
		expect(prompt).toContain("<name>greeting</name>");
		expect(prompt).toContain("Say hello in a friendly, on-brand way");
		// Prove the card carries the on-disk location the subagent will read from.
		expect(prompt).toContain(path.join(".agents", "skills", "greeting"));

		// Visible in the run output so the behavior can be eyeballed.
		const start = prompt.indexOf("<available_skills>");
		console.log(`\n----- subagent system prompt (skills section) -----\n${prompt.slice(start)}\n`);
	});

	it("omits skills when the subagent has no read tool (skills load via read)", () => {
		// An agent whose allowlist excludes `read` cannot load skill files, so the
		// prompt builder deliberately does not advertise them.
		const prompt = buildSubagentPrompt(["bash"]);
		expect(prompt).not.toContain("<available_skills>");
		expect(prompt).not.toContain("greeting");
	});
});
