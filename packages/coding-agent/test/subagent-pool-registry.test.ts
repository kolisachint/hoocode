import { chmodSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { SubagentPool } from "../src/core/subagent-pool.js";

/**
 * Mock executable that records the argv it was spawned with and writes a valid
 * result.json so the dispatch completes. Lets us assert that SubagentPool feeds
 * the spawned child the registry-defined system prompt, tool allowlist, and model.
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
fs.writeFileSync(p.join(outDir, "result.json"), JSON.stringify({ summary: "ok", files_changed: [], confidence: 0.9, status: "complete" }));
console.log(JSON.stringify({ type: "done", reason: "stop", message: { role: "assistant", content: [{ type: "text", text: "ok" }] } }));
process.exit(0);
`;
	writeFileSync(path, content);
	chmodSync(path, 0o755);
	return path;
}

function writeProjectAgent(cwd: string, name: string, frontmatter: string, body: string): void {
	const dir = join(cwd, ".hoocode", "agents");
	mkdirSync(dir, { recursive: true });
	writeFileSync(join(dir, `${name}.md`), `---\nname: ${name}\n${frontmatter}---\n${body}`);
}

function readArgv(cwd: string, taskId: string): string[] {
	const path = join(cwd, ".hoocode", "dispatch", taskId, "argv.json");
	return JSON.parse(readFileSync(path, "utf-8")) as string[];
}

describe("SubagentPool registry wiring", () => {
	let cwd: string;
	let pool: SubagentPool | undefined;

	beforeEach(() => {
		cwd = join(tmpdir(), `pool-registry-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
	});

	afterEach(() => {
		pool?.dispose();
		pool = undefined;
		if (existsSync(cwd)) rmSync(cwd, { recursive: true, force: true });
	});

	test("a project agent overrides the built-in prompt, tools, and model", async () => {
		writeProjectAgent(
			cwd,
			"explore",
			"description: Project explore agent for testing.\ntools: Read, Glob\nmodel: haiku\n",
			"PROJECT EXPLORE PROMPT",
		);
		const mock = createArgvRecorder(cwd);
		pool = new SubagentPool({ executable: process.execPath, prefixArgs: [mock], cwd });

		const result = await pool.dispatch("investigate the module", { forceAgent: "explore" });
		expect(result.handled_inline).toBe(false);
		expect(result.result?.ok).toBe(true);

		const argv = readArgv(cwd, result.task_id!);

		// Registry-defined system prompt body, not the built-in template.
		const promptIdx = argv.indexOf("--system-prompt");
		expect(promptIdx).toBeGreaterThanOrEqual(0);
		expect(argv[promptIdx + 1]).toBe("PROJECT EXPLORE PROMPT");

		// Normalized tool allowlist (Glob -> find), not the built-in MODE_TOOLS set.
		const toolsIdx = argv.indexOf("--tools");
		expect(toolsIdx).toBeGreaterThanOrEqual(0);
		expect(argv[toolsIdx + 1]).toBe("read,find");

		// Declared model wins over any caller default.
		const modelIdx = argv.indexOf("--model");
		expect(modelIdx).toBeGreaterThanOrEqual(0);
		expect(argv[modelIdx + 1]).toBe("haiku");
	});

	test("falls back to the built-in mode allowlist when a definition omits tools", async () => {
		const mock = createArgvRecorder(cwd);
		pool = new SubagentPool({ executable: process.execPath, prefixArgs: [mock], cwd });

		const result = await pool.dispatch("run a read-only scan", { forceAgent: "explore" });
		const argv = readArgv(cwd, result.task_id!);

		// Built-in explore template declares no `tools`, so MODE_TOOLS applies.
		const toolsIdx = argv.indexOf("--tools");
		expect(toolsIdx).toBeGreaterThanOrEqual(0);
		expect(argv[toolsIdx + 1]).toBe("read,grep,find,ls,bash");
	});
});
