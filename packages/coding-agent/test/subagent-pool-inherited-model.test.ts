import { describe, expect, test } from "vitest";
import type { AgentDefinition } from "../src/core/agent-frontmatter.js";
import { AgentRegistry } from "../src/core/agent-registry.js";
import { SubagentPool, type SubagentPoolTask, type SubagentResult } from "../src/core/subagent-pool.js";

/**
 * Covers the inherited-model fallback heuristics:
 *  - isInheritedModelFallbackError() must recognize the "not supported" wording
 *    used by some providers (in addition to "unsupported").
 *  - shouldRetryWithInheritedModel() must allow project-defined agents (not just
 *    built-ins) that pin an explicit model to fall back to the parent model.
 */

// Narrow view onto the private methods under test.
interface PoolInternals {
	registry?: AgentRegistry;
	isInheritedModelFallbackError(result: SubagentResult): boolean;
	shouldRetryWithInheritedModel(task: SubagentPoolTask, result: SubagentResult): boolean;
}

function makePool(): { pool: SubagentPool; internals: PoolInternals; registry: AgentRegistry } {
	const pool = new SubagentPool({ executable: process.execPath });
	const internals = pool as unknown as PoolInternals;
	const registry = new AgentRegistry();
	internals.registry = registry;
	return { pool, internals, registry };
}

function makeResult(overrides: Partial<SubagentResult>): SubagentResult {
	return {
		task_id: "t1",
		ok: false,
		stdout: "",
		stderr: "",
		exit_code: 1,
		status: "failed",
		...overrides,
	};
}

function makeAgent(overrides: Partial<AgentDefinition>): AgentDefinition {
	return {
		name: "custom",
		description: "A project agent",
		prompt: "do work",
		source: "project",
		model: "claude-haiku-4-5",
		...overrides,
	};
}

describe("isInheritedModelFallbackError", () => {
	test("matches the 'not supported' provider wording", () => {
		const { internals } = makePool();
		const result = makeResult({ error: "400 The requested model is not supported" });
		expect(internals.isInheritedModelFallbackError(result)).toBe(true);
	});

	test("still matches the existing 'unsupported' wording", () => {
		const { internals } = makePool();
		const result = makeResult({ error: "model is unsupported by this provider" });
		expect(internals.isInheritedModelFallbackError(result)).toBe(true);
	});

	test("does not match unrelated failures", () => {
		const { internals } = makePool();
		const result = makeResult({ error: "syntax error in tool output" });
		expect(internals.isInheritedModelFallbackError(result)).toBe(false);
	});
});

describe("shouldRetryWithInheritedModel", () => {
	const task: SubagentPoolTask = {
		task_id: "t1",
		agent_type: "custom",
		task: "do work",
		model: "claude-opus-4-7",
	};
	const result = makeResult({ error: "400 The requested model is not supported" });

	test("retries a project agent that pins an explicit model", () => {
		const { internals, registry } = makePool();
		registry.register(makeAgent({ source: "project" }));
		expect(internals.shouldRetryWithInheritedModel(task, result)).toBe(true);
	});

	test("retries a built-in agent that pins an explicit model", () => {
		const { internals, registry } = makePool();
		registry.register(makeAgent({ source: "builtin" }));
		expect(internals.shouldRetryWithInheritedModel(task, result)).toBe(true);
	});

	test("does not retry user/.claude agents", () => {
		const { internals, registry } = makePool();
		registry.register(makeAgent({ source: "user" }));
		expect(internals.shouldRetryWithInheritedModel(task, result)).toBe(false);
	});

	test("does not retry when the agent inherits the model", () => {
		const { internals, registry } = makePool();
		registry.register(makeAgent({ source: "project", model: "inherit" }));
		expect(internals.shouldRetryWithInheritedModel(task, result)).toBe(false);
	});

	test("does not retry when the parent model is unknown", () => {
		const { internals, registry } = makePool();
		registry.register(makeAgent({ source: "project" }));
		const noParentModel: SubagentPoolTask = { ...task, model: undefined };
		expect(internals.shouldRetryWithInheritedModel(noParentModel, result)).toBe(false);
	});
});
