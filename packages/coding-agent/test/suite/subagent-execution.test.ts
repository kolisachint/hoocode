import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type FauxProviderRegistration, type Model, registerFauxProvider } from "@kolisachint/hoocode-ai";
import { afterEach, describe, expect, it } from "vitest";
import { AuthStorage } from "../../src/core/auth-storage.js";
import type { ExtensionContext } from "../../src/core/extensions/types.js";
import { ModelRegistry } from "../../src/core/model-registry.js";
import { markProviderExhausted, resetProviderHealthForTesting } from "../../src/core/provider-health.js";
import { createAgentSession } from "../../src/core/sdk.js";
import { SessionManager } from "../../src/core/session-manager.js";
import { SettingsManager } from "../../src/core/settings-manager.js";
import type { SubagentPool, TaskResult } from "../../src/core/subagent-pool.js";
import { setSubagentPoolForTesting } from "../../src/core/subagent-pool-instance.js";
import type { SubagentResultFile } from "../../src/core/subagent-result.js";
import { taskStore } from "../../src/core/task-store.js";
import { createTaskToolDefinition } from "../../src/core/tools/subagent.js";
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

/**
 * A minimal fake SubagentPool that returns a canned result without spawning a
 * child process. Lets us drive the tool's success/failure paths deterministically.
 */
function makeFakePool(result: TaskResult): SubagentPool {
	return {
		dispatch: async () => result,
	} as unknown as SubagentPool;
}

/** A pool that fails the test if dispatch is ever called (used to prove a skip). */
function makeNeverDispatchPool(): SubagentPool {
	return {
		dispatch: async () => {
			throw new Error("dispatch must not be called when the provider is flagged exhausted");
		},
	} as unknown as SubagentPool;
}

function fakeResult(ok: boolean, data: Partial<SubagentResultFile>, error?: string): TaskResult {
	return {
		handled_inline: false,
		task_id: "fake-1",
		agent_type: "explore",
		reason: "test",
		duration: 1,
		result: {
			task_id: "fake-1",
			ok,
			stdout: "",
			stderr: "",
			exit_code: ok ? 0 : 1,
			error,
			status: ok ? "complete" : "failed",
			result_data: { summary: "", files_changed: [], confidence: 0.9, status: "complete", ...data },
		},
	};
}

afterEach(() => {
	setSubagentPoolForTesting(undefined);
	resetProviderHealthForTesting();
	while (cleanups.length > 0) {
		cleanups.pop()?.();
	}
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
		setSubagentPoolForTesting(makeFakePool(fakeResult(true, { summary: "done exploring" })));
		const ctx = makeCtx(setup, makeTempDir());

		const before = taskStore.list().length;
		const tool = createTaskToolDefinition();
		const result = await tool.execute(
			"call-1",
			{
				description: "explore repo",
				prompt: "explore the repo thoroughly and report findings",
				subagent_type: "explore",
			},
			undefined,
			undefined,
			ctx,
		);

		expect(result.content[0]).toEqual({ type: "text", text: "done exploring" });
		expect(result.details).toMatchObject({ subagent_type: "explore", ok: true, taskId: expect.any(Number) });

		// The finished task stays visible until the next user turn.
		const created = taskStore.list().slice(before);
		expect(created).toHaveLength(1);
		expect(created[0].status).toBe("done");
	});

	it("skips dispatch when the inherited provider is flagged exhausted", async () => {
		const setup = setupFaux();
		setSubagentPoolForTesting(makeNeverDispatchPool());
		const ctx = makeCtx(setup, makeTempDir());
		// Flag the model's provider as exhausted (as the parent session would on a
		// persistent usage/quota error).
		markProviderExhausted(setup.model.provider, "Usage limit reached. Please try again later.");

		const before = taskStore.list().length;
		const tool = createTaskToolDefinition();
		const result = await tool.execute(
			"call-skip",
			{
				description: "explore repo",
				prompt: "explore the repo thoroughly",
				subagent_type: "explore",
			},
			undefined,
			undefined,
			ctx,
		);

		expect(result.details).toMatchObject({ subagent_type: "explore", ok: false });
		expect(result.content[0].type).toBe("text");
		expect((result.content[0] as { text: string }).text).toContain("exhausted");
		expect((result.content[0] as { text: string }).text).toContain("Usage limit reached");

		// A failed task row is recorded so the skip is visible in the panel.
		const created = taskStore.list().slice(before);
		expect(created).toHaveLength(1);
		expect(created[0].status).toBe("failed");
	});

	it("marks the task failed and throws when the subagent errors", async () => {
		const setup = setupFaux();
		setSubagentPoolForTesting(makeFakePool(fakeResult(false, {}, "nope")));
		const ctx = makeCtx(setup, makeTempDir());

		const before = taskStore.list().length;
		const tool = createTaskToolDefinition();

		// A hard subagent failure surfaces as a thrown tool error (idiomatic for this runtime).
		await expect(
			tool.execute(
				"call-2",
				{
					description: "multi-file change",
					prompt: "do a complicated multi-file thing now",
					subagent_type: "edit",
				},
				undefined,
				undefined,
				ctx,
			),
		).rejects.toThrow("nope");

		const created = taskStore.list().slice(before);
		expect(created).toHaveLength(1);
		expect(created[0].status).toBe("failed");
	});

	it("marks the Task tool background only for agents whose definition opts in", () => {
		// A project-local agent with `background: true`; the agent loop reads the
		// tool's `background` flag per call to run such dispatches non-blocking.
		const cwd = makeTempDir();
		const agentsDir = join(cwd, ".hoocode", "agents");
		mkdirSync(agentsDir, { recursive: true });
		writeFileSync(
			join(agentsDir, "watcher.md"),
			[
				"---",
				"name: watcher",
				"description: A non-blocking background agent.",
				"background: true",
				"---",
				"Body.",
			].join("\n"),
		);

		const tool = createTaskToolDefinition(cwd);
		expect(typeof tool.background).toBe("function");
		const isBackground = tool.background as (toolCall: { arguments: Record<string, unknown> }) => boolean;

		expect(isBackground({ arguments: { subagent_type: "watcher" } })).toBe(true);
		// A foreground built-in (and an unknown agent) must not run in the background.
		expect(isBackground({ arguments: { subagent_type: "explore" } })).toBe(false);
		expect(isBackground({ arguments: { subagent_type: "does-not-exist" } })).toBe(false);
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
			customTools: withSubagent ? [createTaskToolDefinition()] : [],
		});
		const names = session.getActiveToolNames();
		session.dispose();
		return names;
	}

	it("activates the Task tool when registered (opted in)", async () => {
		const names = await activeToolNames(true);
		expect(names).toContain("Task");
		expect(names).toContain("read");
	});

	it("does not expose the Task tool when not registered (opted out)", async () => {
		const names = await activeToolNames(false);
		expect(names).not.toContain("Task");
		expect(names).toContain("read");
	});
});
