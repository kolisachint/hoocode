import { chmodSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { loadAgentRegistry } from "../src/core/agent-registry.js";
import { SubagentPool } from "../src/core/subagent-pool.js";

/**
 * End-to-end coverage for a Claude Code-format agent definition placed in
 * `<cwd>/.claude/agents/`. Proves the `.claude/agents` discovery path, Claude
 * tool-name normalization (Glob -> find), and that SubagentPool feeds the
 * spawned child the definition's prompt, normalized tools, and model.
 *
 * Uses a mock executable (not a real provider) so no API keys/tokens are used.
 */
function createArgvRecorder(dir: string): string {
	const path = join(dir, "mock-argv.js");
	const content = `#!/usr/bin/env node
const fs = require("node:fs");
const p = require("node:path");
const argv = process.argv.slice(2);
const i = argv.indexOf("--task-id");
const taskId = i >= 0 ? argv[i + 1] : "unknown";
const outDir = p.join(process.cwd(), ".hoocode", "dispatch", taskId);
fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(p.join(outDir, "argv.json"), JSON.stringify(argv));
fs.writeFileSync(p.join(outDir, "result.json"), JSON.stringify({ summary: "audit complete", files_changed: [], confidence: 0.95, status: "complete" }));
console.log(JSON.stringify({ type: "done", reason: "stop", message: { role: "assistant", content: [{ type: "text", text: "audit complete" }] } }));
process.exit(0);
`;
	writeFileSync(path, content);
	chmodSync(path, 0o755);
	return path;
}

/** Claude Code format: comma-separated tool names, `model` alias, prose body. */
const CLAUDE_AGENT = `---
name: security-reviewer
description: Use to audit code for injection, auth, and secret-handling flaws.
tools: Read, Grep, Glob, Bash
model: sonnet
---
You are a security reviewer. Audit the given code for vulnerabilities and report findings ordered by severity.
`;

function writeClaudeProjectAgent(cwd: string, name: string, content: string): void {
	const dir = join(cwd, ".claude", "agents");
	mkdirSync(dir, { recursive: true });
	writeFileSync(join(dir, `${name}.md`), content);
}

function readArgv(cwd: string, taskId: string): string[] {
	const path = join(cwd, ".hoocode", "dispatch", taskId, "argv.json");
	return JSON.parse(readFileSync(path, "utf-8")) as string[];
}

describe("Claude-format subagent definition", () => {
	let cwd: string;
	let pool: SubagentPool | undefined;

	beforeEach(() => {
		cwd = join(tmpdir(), `claude-agent-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
		writeClaudeProjectAgent(cwd, "security-reviewer", CLAUDE_AGENT);
	});

	afterEach(() => {
		pool?.dispose();
		pool = undefined;
		if (existsSync(cwd)) rmSync(cwd, { recursive: true, force: true });
	});

	test("registry discovers it from .claude/agents and normalizes Claude tool names", () => {
		// includeClaude defaults true; disable user-dir discovery noise by pointing
		// agentDir at the isolated cwd (no agents/ there).
		const registry = loadAgentRegistry({ cwd, agentDir: join(cwd, ".hoocode") });
		const def = registry.get("security-reviewer");
		expect(def).toBeDefined();
		expect(def?.source).toBe("claude-project");
		// Glob -> find; order and the rest preserved.
		expect(def?.tools).toEqual(["read", "grep", "find", "bash"]);
		expect(def?.model).toBe("sonnet");
		expect(def?.prompt).toContain("You are a security reviewer.");
	});

	test("SubagentPool spawns the child with the Claude agent's prompt, tools, and model", async () => {
		const mock = createArgvRecorder(cwd);
		pool = new SubagentPool({ executable: process.execPath, prefixArgs: [mock], cwd });

		const result = await pool.dispatch("Audit src/login.ts for SQL injection", {
			forceAgent: "security-reviewer",
		});
		expect(result.handled_inline).toBe(false);
		expect(result.result?.ok).toBe(true);
		expect((result.result?.result_data as { summary?: string } | undefined)?.summary).toBe("audit complete");

		const argv = readArgv(cwd, result.task_id!);

		const promptIdx = argv.indexOf("--system-prompt");
		expect(promptIdx).toBeGreaterThanOrEqual(0);
		expect(argv[promptIdx + 1]).toContain("You are a security reviewer.");

		const toolsIdx = argv.indexOf("--tools");
		expect(toolsIdx).toBeGreaterThanOrEqual(0);
		expect(argv[toolsIdx + 1]).toBe("read,grep,find,bash");

		const modelIdx = argv.indexOf("--model");
		expect(modelIdx).toBeGreaterThanOrEqual(0);
		expect(argv[modelIdx + 1]).toBe("sonnet");
	});

	test("a Claude project agent overrides a built-in of the same name", () => {
		writeClaudeProjectAgent(
			cwd,
			"explore",
			`---\nname: explore\ndescription: Project-specific explore override.\ntools: Read\nmodel: haiku\n---\nProject explore override prompt.\n`,
		);
		const registry = loadAgentRegistry({ cwd, agentDir: join(cwd, ".hoocode") });
		const def = registry.get("explore");
		expect(def?.source).toBe("claude-project");
		expect(def?.tools).toEqual(["read"]);
		expect(def?.prompt).toBe("Project explore override prompt.");
	});
});
