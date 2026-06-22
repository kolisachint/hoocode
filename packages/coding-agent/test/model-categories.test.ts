import { describe, expect, it } from "vitest";
import { isModelCategory, resolveModelCategory, resolveModelReference } from "../src/core/model-categories.js";
import type { Settings } from "../src/core/settings-manager.js";

describe("model categories", () => {
	it("recognizes the three category names and nothing else", () => {
		expect(isModelCategory("fast")).toBe(true);
		expect(isModelCategory("standard")).toBe(true);
		expect(isModelCategory("capable")).toBe(true);
		expect(isModelCategory("opus")).toBe(false);
		expect(isModelCategory("anthropic/claude-haiku")).toBe(false);
	});

	it("resolves a configured category to its model id", () => {
		const settings: Settings = { modelCategories: { fast: "myprovider/tiny", capable: "myprovider/big" } };
		expect(resolveModelCategory("fast", settings)).toBe("myprovider/tiny");
		expect(resolveModelCategory("capable", settings)).toBe("myprovider/big");
	});

	it("returns undefined for an unconfigured category (no built-in Claude fallback)", () => {
		expect(resolveModelCategory("standard", { modelCategories: { fast: "x" } })).toBeUndefined();
		expect(resolveModelCategory("fast", {})).toBeUndefined();
		expect(resolveModelCategory("capable", undefined)).toBeUndefined();
	});

	it("passes non-category model references through unchanged", () => {
		expect(resolveModelReference("anthropic/claude-sonnet", {})).toBe("anthropic/claude-sonnet");
		expect(resolveModelReference("gpt-4o", undefined)).toBe("gpt-4o");
	});

	it("resolves category references via settings, undefined when unconfigured", () => {
		const settings: Settings = { modelCategories: { standard: "vendor/mid" } };
		expect(resolveModelReference("standard", settings)).toBe("vendor/mid");
		expect(resolveModelReference("fast", settings)).toBeUndefined();
	});
});
