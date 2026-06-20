import type { Api, Model } from "@kolisachint/hoocode-ai";
import { describe, expect, test } from "vitest";
import type { ModelRegistry } from "../src/core/model-registry.js";
import {
	COMPRESSIBLE_TOOLS,
	DEFAULT_TOOL_RESULT_MIN_BYTES,
	LocalInferenceRouter,
	type RoutingConfig,
	resolveRoutingMode,
} from "../src/core/routing/local-inference.js";
import { buildToolResultPrompt, getToolResultPrompt, stripThinkTags } from "../src/core/routing/tool-result-prompts.js";

function fakeModel(provider: string, id: string): Model<Api> {
	return { provider, id, baseUrl: "http://127.0.0.1:8080/v1" } as Model<Api>;
}

function fakeRegistry(models: Model<Api>[]): ModelRegistry {
	return {
		find: (provider: string, id: string) => models.find((m) => m.provider === provider && m.id === id),
	} as unknown as ModelRegistry;
}

const executorRef = { provider: "mlx", model: "qwen3" };

describe("resolveRoutingMode", () => {
	test("defaults to primary-only when nothing activates", () => {
		expect(resolveRoutingMode({})).toBe("primary-only");
		expect(resolveRoutingMode({ enableFlag: false })).toBe("primary-only");
	});

	test("config alone never activates routing", () => {
		expect(resolveRoutingMode({ configMode: "executor-for-summarization" })).toBe("primary-only");
	});

	test("flag activates and defaults to executor-for-summarization", () => {
		expect(resolveRoutingMode({ enableFlag: true })).toBe("executor-for-summarization");
	});

	test("flag honors config mode when set", () => {
		expect(resolveRoutingMode({ enableFlag: true, configMode: "executor-for-tool-results" })).toBe(
			"executor-for-tool-results",
		);
	});

	test("env var activates and selects mode", () => {
		expect(resolveRoutingMode({ envMode: "executor-for-tool-results" })).toBe("executor-for-tool-results");
		expect(resolveRoutingMode({ envMode: "shadow-executor" })).toBe("shadow-executor");
	});

	test("env primary-only does not activate", () => {
		expect(resolveRoutingMode({ envMode: "primary-only" })).toBe("primary-only");
	});

	test("env precedence over config; invalid env with flag falls back to default", () => {
		expect(
			resolveRoutingMode({ enableFlag: true, envMode: "shadow-executor", configMode: "executor-for-summarization" }),
		).toBe("shadow-executor");
		// Invalid env string still activates (non-empty, not primary-only) but resolves to default.
		expect(resolveRoutingMode({ enableFlag: true, envMode: "garbage" })).toBe("executor-for-summarization");
	});
});

describe("LocalInferenceRouter.selectModel", () => {
	const primary = fakeModel("anthropic", "claude");
	const executor = fakeModel("mlx", "qwen3");
	const registry = fakeRegistry([executor]);
	const config: RoutingConfig = { executor: executorRef };

	test("primary-only returns primary and executor unavailable", () => {
		const r = LocalInferenceRouter.create({ mode: "primary-only", config, registry });
		expect(r.isExecutorAvailable()).toBe(false);
		expect(r.selectModel("summarization", primary)).toBe(primary);
	});

	test("executor-for-summarization routes only summarization turns", () => {
		const r = LocalInferenceRouter.create({ mode: "executor-for-summarization", config, registry });
		expect(r.isExecutorAvailable()).toBe(true);
		expect(r.selectModel("summarization", primary)).toBe(executor);
		expect(r.selectModel("tool-result", primary)).toBe(primary);
		expect(r.selectModel("primary", primary)).toBe(primary);
	});

	test("executor-for-tool-results routes only tool-result turns", () => {
		const r = LocalInferenceRouter.create({ mode: "executor-for-tool-results", config, registry });
		expect(r.selectModel("tool-result", primary)).toBe(executor);
		expect(r.selectModel("summarization", primary)).toBe(primary);
	});

	test("shadow-executor keeps primary on the live path", () => {
		const r = LocalInferenceRouter.create({ mode: "shadow-executor", config, registry });
		expect(r.isExecutorAvailable()).toBe(true);
		expect(r.selectModel("summarization", primary)).toBe(primary);
		expect(r.selectModel("tool-result", primary)).toBe(primary);
	});

	test("unresolved executor model degrades to primary", () => {
		const r = LocalInferenceRouter.create({
			mode: "executor-for-summarization",
			config: { executor: { provider: "mlx", model: "missing" } },
			registry,
		});
		expect(r.isExecutorAvailable()).toBe(false);
		expect(r.selectModel("summarization", primary)).toBe(primary);
	});

	test("missing executor config degrades to primary", () => {
		const r = LocalInferenceRouter.create({ mode: "executor-for-summarization", config: {}, registry });
		expect(r.isExecutorAvailable()).toBe(false);
		expect(r.selectModel("summarization", primary)).toBe(primary);
	});
});

describe("LocalInferenceRouter.shouldCompressToolResult", () => {
	const executor = fakeModel("mlx", "qwen3");
	const registry = fakeRegistry([executor]);
	const big = DEFAULT_TOOL_RESULT_MIN_BYTES + 1;
	const small = DEFAULT_TOOL_RESULT_MIN_BYTES - 1;

	test("only compresses in executor-for-tool-results mode", () => {
		const summ = LocalInferenceRouter.create({
			mode: "executor-for-summarization",
			config: { executor: executorRef },
			registry,
		});
		expect(summ.shouldCompressToolResult("read", big)).toBe(false);

		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: executorRef },
			registry,
		});
		expect(tr.shouldCompressToolResult("read", big)).toBe(true);
	});

	test("only compresses read and bash", () => {
		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: executorRef },
			registry,
		});
		expect(tr.shouldCompressToolResult("read", big)).toBe(true);
		expect(tr.shouldCompressToolResult("bash", big)).toBe(true);
		expect(tr.shouldCompressToolResult("grep", big)).toBe(false);
		expect(tr.shouldCompressToolResult("find", big)).toBe(false);
		expect(tr.shouldCompressToolResult("ls", big)).toBe(false);
		expect(tr.shouldCompressToolResult("edit", big)).toBe(false);
	});

	test("respects the size threshold", () => {
		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: executorRef },
			registry,
		});
		expect(tr.shouldCompressToolResult("read", small)).toBe(false);
		expect(tr.shouldCompressToolResult("read", DEFAULT_TOOL_RESULT_MIN_BYTES)).toBe(true);
	});

	test("honors a custom toolResultMinBytes", () => {
		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: { ...executorRef, toolResultMinBytes: 10 } },
			registry,
		});
		expect(tr.shouldCompressToolResult("read", 10)).toBe(true);
		expect(tr.shouldCompressToolResult("read", 9)).toBe(false);
	});

	test("never compresses when executor unavailable", () => {
		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: { provider: "mlx", model: "missing" } },
			registry,
		});
		expect(tr.shouldCompressToolResult("read", big)).toBe(false);
	});
});

describe("tool-result prompts", () => {
	test("read and bash have prompts; fact-list tools do not", () => {
		expect(getToolResultPrompt("read")).toBeTypeOf("string");
		expect(getToolResultPrompt("bash")).toBeTypeOf("string");
		expect(getToolResultPrompt("grep")).toBeUndefined();
		expect(getToolResultPrompt("find")).toBeUndefined();
		expect(getToolResultPrompt("ls")).toBeUndefined();
	});

	test("compressible tool set matches available prompts", () => {
		for (const tool of COMPRESSIBLE_TOOLS) {
			expect(getToolResultPrompt(tool)).toBeTypeOf("string");
		}
	});

	test("buildToolResultPrompt embeds the output and returns undefined for unknown tools", () => {
		const prompt = buildToolResultPrompt("read", "FILE CONTENT HERE");
		expect(prompt).toContain("FILE CONTENT HERE");
		expect(buildToolResultPrompt("grep", "x")).toBeUndefined();
	});
});

describe("stripThinkTags", () => {
	test("removes empty think blocks Qwen3 emits under /no_think", () => {
		expect(stripThinkTags("<think>\n\n</think>\n\nactual output")).toBe("actual output");
	});

	test("removes non-empty think blocks", () => {
		expect(stripThinkTags("<think>reasoning here</think>result")).toBe("result");
	});

	test("removes stray unpaired tags and trims", () => {
		expect(stripThinkTags("  </think>  summary  ")).toBe("summary");
	});

	test("leaves plain text unchanged", () => {
		expect(stripThinkTags("just a summary")).toBe("just a summary");
	});
});
