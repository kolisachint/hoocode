/**
 * Demo (vs pi #1): Agent-spec-tree interop — Claude subagents "just work"
 * ----------------------------------------------------------------------
 * HooCode's reason to exist as a fork is interop: it ingests the resource trees
 * other tools use. `loadAgentRegistry` discovers subagent definitions from BOTH
 * Claude Code's `.claude/agents/` and the standard `.agents/agents/`, alongside
 * HooCode's native `.hoocode/agents/` — with deterministic precedence and
 * collision diagnostics. Upstream pi does not read Claude's tree.
 *
 * This calls the REAL exported `loadAgentRegistry`. It builds a throwaway
 * workspace in a temp dir and points HOME at an empty temp home so the demo is
 * hermetic (it won't pick up your real ~/.claude or ~/.hoocode agents).
 *
 * Run:  npx tsx packages/coding-agent/demo/claude-subagents.ts
 */

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import chalk from "chalk";

// Hermetic: isolate HOME (loaders scan ~/.claude, ~/.agents, ~/.hoocode).
const home = mkdtempSync(join(tmpdir(), "hoo-home-"));
process.env.HOME = home;
process.env.USERPROFILE = home;

import { loadAgentRegistry } from "../src/core/agent-registry.js";

const proj = mkdtempSync(join(tmpdir(), "hoo-proj-"));
const write = (rel: string, body: string) => {
	const full = join(proj, rel);
	mkdirSync(join(full, ".."), { recursive: true });
	writeFileSync(full, body);
};

// A subagent authored for Claude Code (vendor tree).
write(
	".claude/agents/reviewer.md",
	`---\nname: reviewer\ndescription: Reviews diffs for bugs (authored for Claude Code)\ntools: Read, Grep, Bash\nmodel: sonnet\n---\nYou are a code reviewer.\n`,
);
// HooCode's own override of the same name (native tree wins over the vendor one).
write(
	".hoocode/agents/reviewer.md",
	`---\nname: reviewer\ndescription: Reviews diffs (HooCode native — overrides the Claude copy)\ntools: Read, Grep\nmodel: opus\n---\nYou are a strict reviewer.\n`,
);
// A standard agent-spec agent (the cross-tool .agents/ convention).
write(
	".agents/agents/planner.md",
	`---\nname: planner\ndescription: Writes implementation plans\ntools: Read, Grep, Glob\n---\nYou write plans.\n`,
);

const userAgentDir = mkdtempSync(join(tmpdir(), "hoo-user-"));
const registry = loadAgentRegistry({
	cwd: proj,
	agentDir: userAgentDir,
	includeBuiltins: false,
	includeClaude: true,
});

console.log(chalk.bold.cyan("\nHooCode · agent-spec-tree interop (reads Claude + .agents + .hoocode)\n"));
console.log(chalk.bold("Discovered subagents:"));
for (const a of registry.list()) {
	const src = a.source.startsWith("claude") ? chalk.magenta(a.source) : chalk.green(a.source);
	console.log(
		`  ${chalk.bold(a.name.padEnd(10))} ${src.padEnd(22)} ${chalk.dim(`model=${a.model ?? "inherit"} tools=${(a.tools ?? ["*"]).join("/")}`)}`,
	);
	console.log(`             ${chalk.dim(relative(proj, a.filePath ?? ""))}  — ${a.description}`);
}

const diags = registry.getDiagnostics();
if (diags.length) {
	console.log(chalk.bold("\nDiagnostics (precedence / collisions):"));
	for (const d of diags) console.log(`  ${chalk.yellow("•")} ${d.message}`);
}

console.log(
	chalk.bold.cyan(
		`\n→ ${registry.list().length} agents loaded across three trees; the Claude-authored "reviewer" was overridden by the .hoocode copy.`,
	),
);
console.log(chalk.dim("  Drop a .claude/agents file in and HooCode finds it — no conversion, no copy. pi doesn't.\n"));
process.exit(0);
