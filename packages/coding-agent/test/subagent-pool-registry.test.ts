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
fs.writeFileSync(p.join(process.cwd(), "argv.json"), JSON.stringify(argv));
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

function readArgv(cwd: string, _taskId: string): string[] {
	const path = join(cwd, "argv.json");
	return JSON.parse(readFileSync(path, "utf-8")) as string[];
}

function createModelFallbackRecorder(dir: string): string {
	const path = join(dir, "mock-model-fallback.js");
	const content = `#!/usr/bin/env node
const fs = require("node:fs");
const p = require("node:path");
const argv = process.argv.slice(2);
const argvListPath = p.join(process.cwd(), "argv-list.json");
const argvList = fs.existsSync(argvListPath) ? JSON.parse(fs.readFileSync(argvListPath, "utf-8")) : [];
argvList.push(argv);
fs.writeFileSync(argvListPath, JSON.stringify(argvList));
const modelIdx = argv.indexOf("--model");
const model = modelIdx >= 0 ? argv[modelIdx + 1] : undefined;
if (model !== "parent-model") {
  console.error("No API key found for preferred model");
  process.exit(1);
}
const i = argv.indexOf("--task-id");
const taskId = i >= 0 ? argv[i + 1] : "unknown";
const outDir = p.join(process.cwd(), ".hoocode", "dispatch", taskId);
fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(p.join(outDir, "result.json"), JSON.stringify({ summary: "ok", files_changed: [], confidence: 0.9, status: "complete" }));
console.log(JSON.stringify({ type: "done", reason: "stop", message: { role: "assistant", content: [{ type: "text", text: "ok" }] } }));
process.exit(0);
`;
	writeFileSync(path, content);
	chmodSync(path, 0o755);
	return path;
}

function readArgvList(cwd: string): string[][] {
	const path = join(cwd, "argv-list.json");
	return JSON.parse(readFileSync(path, "utf-8")) as string[][];
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

	test("uses the built-in agent's frontmatter tool allowlist", async () => {
		const mock = createArgvRecorder(cwd);
		pool = new SubagentPool({ executable: process.execPath, prefixArgs: [mock], cwd });

		const result = await pool.dispatch("run a read-only scan", { forceAgent: "explore" });
		const argv = readArgv(cwd, result.task_id!);

		// Built-in explore template declares its read-only allowlist in frontmatter.
		const toolsIdx = argv.indexOf("--tools");
		expect(toolsIdx).toBeGreaterThanOrEqual(0);
		expect(argv[toolsIdx + 1]).toBe("read,grep,find,ls");
	});

	test("retries built-in agents with the inherited model when the preferred model is unavailable", async () => {
		const mock = createModelFallbackRecorder(cwd);
		pool = new SubagentPool({ executable: process.execPath, prefixArgs: [mock], cwd });

		const result = await pool.dispatch("run a read-only scan", {
			forceAgent: "explore",
			model: "parent-model",
			provider: "parent-provider",
		});
		expect(result.result?.ok).toBe(true);

		const argvList = readArgvList(cwd);
		expect(argvList).toHaveLength(2);

		const firstModelIdx = argvList[0].indexOf("--model");
		expect(firstModelIdx).toBeGreaterThanOrEqual(0);
		expect(argvList[0][firstModelIdx + 1]).toBe("haiku");

		const fallbackModelIdx = argvList[1].indexOf("--model");
		expect(fallbackModelIdx).toBeGreaterThanOrEqual(0);
		expect(argvList[1][fallbackModelIdx + 1]).toBe("parent-model");
	});

	test("falls back to the inherited model when only the parent model is known (no provider)", async () => {
		const mock = createModelFallbackRecorder(cwd);
		pool = new SubagentPool({ executable: process.execPath, prefixArgs: [mock], cwd });

		// Parent threads through a model but no provider (e.g. gateway routing). The
		// preferred model still fails, so the retry must inherit the parent model.
		const result = await pool.dispatch("run a read-only scan", {
			forceAgent: "explore",
			model: "parent-model",
		});
		expect(result.result?.ok).toBe(true);

		const argvList = readArgvList(cwd);
		expect(argvList).toHaveLength(2);
		expect(argvList[0][argvList[0].indexOf("--model") + 1]).toBe("haiku");
		expect(argvList[1][argvList[1].indexOf("--model") + 1]).toBe("parent-model");
	});
});
