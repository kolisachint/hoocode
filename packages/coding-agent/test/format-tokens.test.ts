import { describe, expect, test } from "vitest";
import { formatTokens } from "../src/core/format-tokens.js";

describe("formatTokens", () => {
	test("formats sub-1000 counts as plain numbers", () => {
		expect(formatTokens(0)).toBe("0");
		expect(formatTokens(99)).toBe("99");
		expect(formatTokens(999)).toBe("999");
	});

	test("formats 1000-9999 counts with one decimal k", () => {
		expect(formatTokens(1000)).toBe("1.0k");
		expect(formatTokens(4100)).toBe("4.1k");
		expect(formatTokens(9900)).toBe("9.9k");
	});

	test("formats 10000-999999 counts as whole k", () => {
		expect(formatTokens(10000)).toBe("10k");
		expect(formatTokens(41000)).toBe("41k");
		expect(formatTokens(999000)).toBe("999k");
	});

	test("formats 1000000-9999999 counts with one decimal M", () => {
		expect(formatTokens(1000000)).toBe("1.0M");
		expect(formatTokens(4100000)).toBe("4.1M");
		expect(formatTokens(9900000)).toBe("9.9M");
	});

	test("formats 10M+ counts as whole M", () => {
		expect(formatTokens(10000000)).toBe("10M");
		expect(formatTokens(41000000)).toBe("41M");
	});
});
