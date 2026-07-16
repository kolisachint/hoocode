import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { getModel } from "@kolisachint/hoocode-ai";
import { Type } from "typebox";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { parseArgs } from "../src/cli/args.js";
import {
	createLightTools,
	LIGHT_MODE_ENV,
	LIGHT_SYSTEM_PROMPT,
	LIGHT_TOOL_NAMES,
	measurePromptSurface,
} from "../src/core/light.js";
import { createAgentSession } from "../src/core/sdk.js";
import { SessionManager } from "../src/core/session-manager.js";
import { SettingsManager } from "../src/core/settings-manager.js";
import { setupMode } from "../src/extensions/core/modes.js";
import { createHarness, createHarnessWithExtensions, type Harness } from "./test-harness.js";
import { createTestResourceLoader } from "./utilities.js";

const SHORT_DESCRIPTIONS: Record<string, string> = {
	read: "Read a file. args: path, offset?, limit?",
	write: "Write file (overwrites). args: path, content",
	edit: "Replace exact text. args: path, oldText, newText",
	bash: "Run a shell command. args: command, timeout?",
};

function createLightResourceLoader() {
	return {
		...createTestResourceLoader(),
		getSystemPrompt: () => LIGHT_SYSTEM_PROMPT,
	};
}

describe("light mode", () => {
	let tempDir: string;
	let agentDir: string;
	let harness: Harness | undefined;
	const savedLightEnv = process.env[LIGHT_MODE_ENV];

	beforeEach(() => {
		tempDir = join(tmpdir(), `pi-light-test-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		agentDir = join(tempDir, "agent");
		mkdirSync(agentDir, { recursive: true });
		delete process.env[LIGHT_MODE_ENV];
	});

	afterEach(() => {
		harness?.cleanup();
		harness = undefined;
		if (savedLightEnv === undefined) {
			delete process.env[LIGHT_MODE_ENV];
		} else {
			process.env[LIGHT_MODE_ENV] = savedLightEnv;
		}
		if (tempDir && existsSync(tempDir)) {
			rmSync(tempDir, { recursive: true, force: true });
		}
	});

	it("parses --light and defaults the setting to off", () => {
		expect(parseArgs(["--light"]).light).toBe(true);
		expect(parseArgs([]).light).toBeUndefined();
		expect(parseArgs(["--print-token-surface"]).printTokenSurface).toBe(true);

		const settingsManager = SettingsManager.create(tempDir, agentDir);
		expect(settingsManager.getLight()).toBe(false);
		settingsManager.applyOverrides({ light: true });
		expect(settingsManager.getLight()).toBe(true);
	});

	it("exposes exactly the four light tools and keeps Task/TodoWrite inactive", async () => {
		const settingsManager = SettingsManager.create(tempDir, agentDir);
		const sessionManager = SessionManager.inMemory();

		const { session } = await createAgentSession({
			cwd: tempDir,
			agentDir,
			model: getModel("anthropic", "claude-sonnet-4-5")!,
			settingsManager,
			sessionManager,
			resourceLoader: createLightResourceLoader(),
			tools: [...LIGHT_TOOL_NAMES],
			baseToolsOverride: createLightTools(tempDir),
			// Simulate an embedder registering the heavyweight tools anyway: the
			// light allowlist must keep them out of the active set.
			customTools: [
				{
					name: "Task",
					label: "Task",
					description: "Should never activate in light mode",
					parameters: Type.Object({}),
					execute: async () => ({ content: [{ type: "text", text: "no" }], details: {} }),
				},
				{
					name: "TodoWrite",
					label: "TodoWrite",
					description: "Should never activate in light mode",
					parameters: Type.Object({}),
					execute: async () => ({ content: [{ type: "text", text: "no" }], details: {} }),
				},
			],
		});

		expect(session.getActiveToolNames().sort()).toEqual([...LIGHT_TOOL_NAMES].sort());
		for (const [name, description] of Object.entries(SHORT_DESCRIPTIONS)) {
			expect(session.getToolDefinition(name)?.description).toBe(description);
		}
		// Stripped schemas: no per-property descriptions anywhere.
		for (const name of LIGHT_TOOL_NAMES) {
			expect(JSON.stringify(session.getToolDefinition(name)?.parameters)).not.toContain("description");
		}

		expect(session.systemPrompt).toContain("You are a coding agent.");
		expect(session.systemPrompt).not.toContain("hoo-core: mode=");
		expect(session.systemPrompt).not.toContain("Available tools:");
		expect(session.systemPrompt).not.toContain("# Project Context");

		session.dispose();
	});

	it("sends the short descriptions and stripped schemas to the provider", async () => {
		harness = createHarness({
			responses: ["ok"],
			baseToolsOverride: createLightTools(process.cwd()),
			resourceLoader: createLightResourceLoader(),
		});

		await harness.session.prompt("hi");

		const context = harness.faux.contexts[0];
		expect(context.tools?.map((tool) => tool.name).sort()).toEqual([...LIGHT_TOOL_NAMES].sort());
		for (const tool of context.tools ?? []) {
			expect(tool.description).toBe(SHORT_DESCRIPTIONS[tool.name]);
			expect(JSON.stringify(tool.parameters)).not.toContain("description");
		}
		expect(context.systemPrompt).toContain("You are a coding agent.");
		expect(context.systemPrompt).not.toContain("hoo-core: mode=");
	});

	it("suppresses the hoo-core mode appendix when the light env flag is set", async () => {
		process.env[LIGHT_MODE_ENV] = "1";
		harness = await createHarnessWithExtensions({
			responses: ["ok"],
			extensionFactories: [(pi) => setupMode(pi)],
		});
		await harness.session.bindExtensions({});
		await harness.session.prompt("hi");

		expect(harness.faux.contexts[0].systemPrompt).not.toContain("<!-- hoo-core: mode=");
	});

	it("keeps the hoo-core mode appendix without the light env flag", async () => {
		harness = await createHarnessWithExtensions({
			responses: ["ok"],
			extensionFactories: [(pi) => setupMode(pi)],
		});
		await harness.session.bindExtensions({});
		await harness.session.prompt("hi");

		expect(harness.faux.contexts[0].systemPrompt).toContain("<!-- hoo-core: mode=");
	});

	it("executes a flat light edit through the real edit tool", async () => {
		const workDir = join(tempDir, "work");
		mkdirSync(workDir, { recursive: true });
		const filePath = join(workDir, "hello.txt");
		writeFileSync(filePath, "hello old world\n");

		harness = createHarness({
			responses: [
				{ toolCalls: [{ name: "edit", args: { path: filePath, oldText: "old", newText: "new" } }] },
				"done",
			],
			baseToolsOverride: createLightTools(workDir),
		});

		await harness.session.prompt("edit the file");

		expect(readFileSync(filePath, "utf8")).toBe("hello new world\n");
		const toolResults = harness.session.messages.filter((m) => m.role === "toolResult");
		expect(toolResults).toHaveLength(1);
	});

	it("keeps the fixed per-turn surface small", async () => {
		const settingsManager = SettingsManager.create(tempDir, agentDir);
		const sessionManager = SessionManager.inMemory();
		const { session } = await createAgentSession({
			cwd: tempDir,
			agentDir,
			model: getModel("anthropic", "claude-sonnet-4-5")!,
			settingsManager,
			sessionManager,
			resourceLoader: createLightResourceLoader(),
			tools: [...LIGHT_TOOL_NAMES],
			baseToolsOverride: createLightTools(tempDir),
		});

		const surface = measurePromptSurface(session);
		console.log(
			`light mode fixed per-turn surface: ${surface.totalTokens} tokens ` +
				`(system prompt ${surface.systemPromptTokens}, tool schemas ${surface.toolSchemaTokens})`,
		);
		expect(surface.tools).toHaveLength(4);
		expect(surface.totalTokens).toBeGreaterThan(0);
		expect(surface.totalTokens).toBeLessThan(400);

		session.dispose();
	});
});
