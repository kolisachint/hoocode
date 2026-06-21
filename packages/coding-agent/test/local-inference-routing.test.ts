import type { Api, Model } from "@kolisachint/hoocode-ai";
import { describe, expect, test } from "vitest";
import type { ModelRegistry } from "../src/core/model-registry.js";
import {
	COMPRESSIBLE_TOOLS,
	DEFAULT_MAX_BYTES,
	DEFAULT_MIN_BYTES,
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
		expect(resolveRoutingMode({ envMode: "executor-for-summarization" })).toBe("executor-for-summarization");
	});

	test("env primary-only does not activate", () => {
		expect(resolveRoutingMode({ envMode: "primary-only" })).toBe("primary-only");
	});

	test("env precedence over config; invalid env with flag falls back to default", () => {
		expect(
			resolveRoutingMode({
				enableFlag: true,
				envMode: "executor-for-tool-results",
				configMode: "executor-for-summarization",
			}),
		).toBe("executor-for-tool-results");
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
	const inBand = DEFAULT_MIN_BYTES + 1;
	const tooSmall = DEFAULT_MIN_BYTES - 1;
	const tooBig = DEFAULT_MAX_BYTES + 1;

	test("only compresses in executor-for-tool-results mode", () => {
		const summ = LocalInferenceRouter.create({
			mode: "executor-for-summarization",
			config: { executor: executorRef },
			registry,
		});
		expect(summ.shouldCompressToolResult("bash", inBand)).toBe(false);

		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: executorRef },
			registry,
		});
		expect(tr.shouldCompressToolResult("bash", inBand)).toBe(true);
	});

	test("only compresses bash; read and fact-list tools pass through", () => {
		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: executorRef },
			registry,
		});
		expect(tr.shouldCompressToolResult("bash", inBand)).toBe(true);
		expect(tr.shouldCompressToolResult("read", inBand)).toBe(false);
		expect(tr.shouldCompressToolResult("grep", inBand)).toBe(false);
		expect(tr.shouldCompressToolResult("find", inBand)).toBe(false);
		expect(tr.shouldCompressToolResult("ls", inBand)).toBe(false);
		expect(tr.shouldCompressToolResult("edit", inBand)).toBe(false);
	});

	test("respects the global size band (min and max)", () => {
		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: executorRef },
			registry,
		});
		expect(tr.shouldCompressToolResult("bash", tooSmall)).toBe(false);
		expect(tr.shouldCompressToolResult("bash", DEFAULT_MIN_BYTES)).toBe(true);
		expect(tr.shouldCompressToolResult("bash", DEFAULT_MAX_BYTES)).toBe(true);
		expect(tr.shouldCompressToolResult("bash", tooBig)).toBe(false);
	});

	test("honors custom minBytes/maxBytes", () => {
		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: { ...executorRef, minBytes: 10, maxBytes: 20 } },
			registry,
		});
		expect(tr.shouldCompressToolResult("bash", 9)).toBe(false);
		expect(tr.shouldCompressToolResult("bash", 10)).toBe(true);
		expect(tr.shouldCompressToolResult("bash", 20)).toBe(true);
		expect(tr.shouldCompressToolResult("bash", 21)).toBe(false);
	});

	test("never compresses when executor unavailable", () => {
		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: { provider: "mlx", model: "missing" } },
			registry,
		});
		expect(tr.shouldCompressToolResult("bash", inBand)).toBe(false);
	});
});

describe("LocalInferenceRouter.shouldRouteSummarization", () => {
	const executor = fakeModel("mlx", "qwen3");
	const registry = fakeRegistry([executor]);
	const inBand = DEFAULT_MIN_BYTES + 1;

	test("routes only in executor-for-summarization mode within the band", () => {
		const summ = LocalInferenceRouter.create({
			mode: "executor-for-summarization",
			config: { executor: executorRef },
			registry,
		});
		expect(summ.shouldRouteSummarization(inBand)).toBe(true);
		expect(summ.shouldRouteSummarization(DEFAULT_MIN_BYTES - 1)).toBe(false);
		expect(summ.shouldRouteSummarization(DEFAULT_MAX_BYTES + 1)).toBe(false);

		const tr = LocalInferenceRouter.create({
			mode: "executor-for-tool-results",
			config: { executor: executorRef },
			registry,
		});
		expect(tr.shouldRouteSummarization(inBand)).toBe(false);
	});

	test("never routes when executor unavailable", () => {
		const summ = LocalInferenceRouter.create({
			mode: "executor-for-summarization",
			config: { executor: { provider: "mlx", model: "missing" } },
			registry,
		});
		expect(summ.shouldRouteSummarization(inBand)).toBe(false);
	});
});

describe("tool-result prompts", () => {
	test("only bash has a prompt; read and fact-list tools do not", () => {
		expect(getToolResultPrompt("bash")).toBeTypeOf("string");
		expect(getToolResultPrompt("read")).toBeUndefined();
		expect(getToolResultPrompt("grep")).toBeUndefined();
		expect(getToolResultPrompt("find")).toBeUndefined();
		expect(getToolResultPrompt("ls")).toBeUndefined();
	});

	test("compressible tool set matches available prompts", () => {
		for (const tool of COMPRESSIBLE_TOOLS) {
			expect(getToolResultPrompt(tool)).toBeTypeOf("string");
		}
	});

	test("buildToolResultPrompt embeds the output and returns undefined for non-compressible tools", () => {
		const prompt = buildToolResultPrompt("bash", "COMMAND OUTPUT HERE");
		expect(prompt).toContain("COMMAND OUTPUT HERE");
		expect(buildToolResultPrompt("read", "x")).toBeUndefined();
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
