import { mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
	type FauxProviderRegistration,
	fauxAssistantMessage,
	fauxToolCall,
	type Model,
	registerFauxProvider,
} from "@kolisachint/hoocode-ai";
import { afterEach, describe, expect, it } from "vitest";
import { AuthStorage } from "../../src/core/auth-storage.js";
import type { ExtensionContext } from "../../src/core/extensions/types.js";
import { ModelRegistry } from "../../src/core/model-registry.js";
import { createAgentSession } from "../../src/core/sdk.js";
import { SessionManager } from "../../src/core/session-manager.js";
import { SettingsManager } from "../../src/core/settings-manager.js";
import { runSubagent } from "../../src/core/subagent.js";
import { taskStore } from "../../src/core/task-store.js";
import { createSubagentToolDefinition } from "../../src/core/tools/subagent.js";
import { createTestResourceLoader } from "../utilities.js";

interface FauxSetup {
	faux: FauxProviderRegistration;
	model: Model<string>;
	modelRegistry: ModelRegistry;
	authStorage: AuthStorage;
}

const cleanups: Array<() => void> = [];

function makeTempDir(): string {
	const dir = join(tmpdir(), `pi-subagent-${Date.now()}-${Math.random().toString(36).slice(2)}`);
	mkdirSync(dir, { recursive: true });
	cleanups.push(() => rmSync(dir, { recursive: true, force: true }));
	return dir;
}

function setupFaux(): FauxSetup {
	const faux = registerFauxProvider({});
	faux.setResponses([]);
	cleanups.push(() => faux.unregister());
	const model = faux.getModel();
	const authStorage = AuthStorage.inMemory();
	authStorage.setRuntimeApiKey(model.provider, "faux-key");
	const modelRegistry = ModelRegistry.inMemory(authStorage);
	modelRegistry.registerProvider(model.provider, {
		baseUrl: model.baseUrl,
		apiKey: "faux-key",
		api: faux.api,
		models: faux.models.map((m) => ({
			id: m.id,
			name: m.name,
			api: m.api,
			reasoning: m.reasoning,
			input: m.input,
			cost: m.cost,
			contextWindow: m.contextWindow,
			maxTokens: m.maxTokens,
			baseUrl: m.baseUrl,
		})),
	});
	return { faux, model, modelRegistry, authStorage };
}

afterEach(() => {
	while (cleanups.length > 0) {
		cleanups.pop()?.();
	}
});

describe("runSubagent execution", () => {
	it("returns the subagent's final answer", async () => {
		const { faux, model, modelRegistry } = setupFaux();
		faux.setResponses([fauxAssistantMessage("the answer")]);

		const result = await runSubagent({
			task: "what is the answer",
			mode: "explore",
			cwd: makeTempDir(),
			model,
			modelRegistry,
		});

		expect(result.ok).toBe(true);
		expect(result.answer).toBe("the answer");
		expect(result.mode).toBe("explore");
	});

	it("returns ONLY the final answer, stripping intermediate tool-call turns", async () => {
		const { faux, model, modelRegistry } = setupFaux();
		// Turn 1: a tool call (no final text). Turn 2: the final answer.
		faux.setResponses([
			fauxAssistantMessage(fauxToolCall("ls", { path: "." }), { stopReason: "toolUse" }),
			fauxAssistantMessage("FINAL ONLY"),
		]);

		const result = await runSubagent({
			task: "look around then answer",
			mode: "explore",
			cwd: makeTempDir(),
			model,
			modelRegistry,
		});

		expect(result.ok).toBe(true);
		// The intermediate ls tool call and its result are not part of the answer.
		expect(result.answer).toBe("FINAL ONLY");
	});

	it("reports failure when the subagent ends in an error", async () => {
		const { faux, model, modelRegistry } = setupFaux();
		faux.setResponses([fauxAssistantMessage("", { stopReason: "error", errorMessage: "boom" })]);

		const result = await runSubagent({
			task: "explode",
			mode: "edit",
			cwd: makeTempDir(),
			model,
			modelRegistry,
		});

		expect(result.ok).toBe(false);
		expect(result.answer).toBe("");
		expect(result.error).toContain("boom");
	});
});

describe("subagent tool (opt-in) execution and task integration", () => {
	function makeCtx(setup: FauxSetup, cwd: string): ExtensionContext {
		return {
			cwd,
			model: setup.model,
			modelRegistry: setup.modelRegistry,
			hasUI: true,
			signal: undefined,
			ui: {
				setStatus: (_key: string, _text: string | undefined) => {},
			},
		} as unknown as ExtensionContext;
	}

	it("runs the subagent, returns its answer, and tracks a task to completion", async () => {
		const setup = setupFaux();
		setup.faux.setResponses([fauxAssistantMessage("done exploring")]);
		const ctx = makeCtx(setup, makeTempDir());

		const before = taskStore.list().length;
		const tool = createSubagentToolDefinition();
		const result = await tool.execute(
			"call-1",
			{ task: "explore the repo", context: "", mode: "explore" },
			undefined,
			undefined,
			ctx,
		);

		expect(result.content[0]).toEqual({ type: "text", text: "done exploring" });

		const created = taskStore.list().slice(before);
		expect(created).toHaveLength(1);
		expect(created[0].status).toBe("done");
		expect(created[0].subagentMode).toBe("explore");
	});

	it("marks the task failed and throws when the subagent errors", async () => {
		const setup = setupFaux();
		setup.faux.setResponses([fauxAssistantMessage("", { stopReason: "error", errorMessage: "nope" })]);
		const ctx = makeCtx(setup, makeTempDir());

		const before = taskStore.list().length;
		const tool = createSubagentToolDefinition();

		// A hard subagent failure surfaces as a thrown tool error (idiomatic for this runtime).
		await expect(
			tool.execute(
				"call-2",
				{ task: "do a thing", context: "some context", mode: "fix" },
				undefined,
				undefined,
				ctx,
			),
		).rejects.toThrow("nope");

		const created = taskStore.list().slice(before);
		expect(created[0].status).toBe("failed");
	});
});

describe("subagent tool gating (opt-in vs opt-out)", () => {
	async function activeToolNames(withSubagent: boolean): Promise<string[]> {
		const setup = setupFaux();
		const { session } = await createAgentSession({
			cwd: makeTempDir(),
			model: setup.model,
			modelRegistry: setup.modelRegistry,
			authStorage: setup.authStorage,
			settingsManager: SettingsManager.inMemory(),
			sessionManager: SessionManager.inMemory(),
			resourceLoader: createTestResourceLoader(),
			customTools: withSubagent ? [createSubagentToolDefinition()] : [],
		});
		const names = session.getActiveToolNames();
		session.dispose();
		return names;
	}

	it("activates the subagent tool when registered (opted in)", async () => {
		const names = await activeToolNames(true);
		expect(names).toContain("subagent");
		expect(names).toContain("read");
	});

	it("does not expose the subagent tool when not registered (opted out)", async () => {
		const names = await activeToolNames(false);
		expect(names).not.toContain("subagent");
		expect(names).toContain("read");
	});
});
