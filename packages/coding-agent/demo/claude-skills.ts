/**
 * Demo (vs pi #2): Skill interop — Claude `SKILL.md` skills "just work"
 * --------------------------------------------------------------------
 * The same interop story for skills. `loadSkills` discovers `SKILL.md` skills
 * from Claude Code's `.claude/skills/` and HooCode's native `.hoocode/skills/`,
 * normalizes each skill's `allowed-tools` from Claude tool names (Read, Edit,
 * Bash, …) to HooCode's tool names, and reports collisions. Upstream pi has no
 * notion of the Claude skills tree.
 *
 * This calls the REAL exported `loadSkills`, in a hermetic temp workspace with an
 * isolated HOME so your real skills aren't scanned.
 *
 * Run:  npx tsx packages/coding-agent/demo/claude-skills.ts
 */

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import chalk from "chalk";

// Hermetic: isolate HOME (loadSkills scans ~/.claude/skills and ~/.hoocode/skills).
const home = mkdtempSync(join(tmpdir(), "hoo-home-"));
process.env.HOME = home;
process.env.USERPROFILE = home;

import { loadSkills } from "../src/core/skills.js";

const proj = mkdtempSync(join(tmpdir(), "hoo-proj-"));
const writeSkill = (dir: string, name: string, description: string, allowedTools: string) => {
	const skillDir = join(proj, dir, name);
	mkdirSync(skillDir, { recursive: true });
	writeFileSync(
		join(skillDir, "SKILL.md"),
		`---\nname: ${name}\ndescription: ${description}\nallowed-tools: ${allowedTools}\n---\nSteps for ${name}.\n`,
	);
};

// A skill authored for Claude Code (vendor tree), using Claude tool names.
writeSkill(".claude/skills", "changelog", "Update CHANGELOG.md from staged diffs (Claude format)", "Read, Edit, Bash");
// A duplicate name in HooCode's own tree — triggers a collision diagnostic.
writeSkill(".hoocode/skills", "changelog", "Update the changelog (HooCode-native duplicate)", "Read, Edit");
// A HooCode-native-only skill.
writeSkill(".hoocode/skills", "release", "Cut a release", "Bash");

const userAgentDir = mkdtempSync(join(tmpdir(), "hoo-user-"));
const { skills, diagnostics } = loadSkills({
	cwd: proj,
	agentDir: userAgentDir,
	skillPaths: [],
	includeDefaults: true,
	includeClaude: true,
});

console.log(chalk.bold.cyan("\nHooCode · skill interop (reads Claude .claude/skills + .hoocode/skills)\n"));
console.log(chalk.bold("Discovered skills:"));
for (const s of skills) {
	console.log(`  ${chalk.bold(s.name.padEnd(12))} ${chalk.dim(`tools=[${(s.allowedTools ?? ["*"]).join(", ")}]`)}`);
	console.log(`               ${chalk.dim(relative(proj, s.filePath))}  — ${s.description}`);
}

if (diagnostics.length) {
	console.log(chalk.bold("\nDiagnostics (collisions across trees):"));
	for (const d of diagnostics) {
		const c = d.collision;
		const detail = c ? chalk.dim(` keeps ${relative(proj, c.winnerPath)}, drops ${relative(proj, c.loserPath)}`) : "";
		console.log(`  ${chalk.yellow("•")} ${d.message}${detail}`);
	}
}

console.log(
	chalk.bold.cyan(
		`\n→ ${skills.length} skills loaded; the Claude "changelog" SKILL.md was read natively and its tools normalized to HooCode names.`,
	),
);
console.log(
	chalk.dim(
		"  A skill written for Claude runs in HooCode unchanged, and duplicates across trees are flagged, not silently merged.\n",
	),
);
process.exit(0);
