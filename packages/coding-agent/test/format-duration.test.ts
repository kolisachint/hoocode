import { describe, expect, test } from "vitest";
import { formatDurationSecs } from "../src/core/format-duration.js";

describe("formatDurationSecs", () => {
	test("formats sub-10-second durations with one decimal", () => {
		expect(formatDurationSecs(0)).toBe("0.0s");
		expect(formatDurationSecs(4.1)).toBe("4.1s");
		expect(formatDurationSecs(9.9)).toBe("9.9s");
	});

	test("formats 10-59 second durations as whole seconds", () => {
		expect(formatDurationSecs(10)).toBe("10s");
		expect(formatDurationSecs(30)).toBe("30s");
		expect(formatDurationSecs(59)).toBe("59s");
	});

	test("formats 1-59 minute durations as minutes and seconds", () => {
		expect(formatDurationSecs(60)).toBe("1m00s");
		expect(formatDurationSecs(90)).toBe("1m30s");
		expect(formatDurationSecs(3599)).toBe("59m59s");
	});

	test("formats 1+ hour durations as hours and minutes", () => {
		expect(formatDurationSecs(3600)).toBe("1h00m");
		expect(formatDurationSecs(3660)).toBe("1h01m");
		expect(formatDurationSecs(7200)).toBe("2h00m");
		expect(formatDurationSecs(7260)).toBe("2h01m");
	});

	test("handles negative values by clamping to zero", () => {
		expect(formatDurationSecs(-10)).toBe("0.0s");
	});

	test("handles fractional hours correctly", () => {
		expect(formatDurationSecs(5400)).toBe("1h30m");
		expect(formatDurationSecs(9000)).toBe("2h30m");
	});
});
