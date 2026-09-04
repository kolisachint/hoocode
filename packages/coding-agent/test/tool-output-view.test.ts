import stripAnsi from "strip-ansi";
import { describe, expect, test } from "vitest";
import {
	cycleToolOutputView,
	DEFAULT_TOOL_OUTPUT_VIEW,
	isToolOutputView,
	LEGACY_TOOL_OUTPUT_VIEWS,
	MAX_TOOL_OUTPUT_VIEW,
	TOOL_OUTPUT_VIEWS,
} from "../src/core/tool-output-view.js";
import { renderToolSignalLine, toolSignal, toolSubject } from "../src/modes/interactive/components/tool-signal.js";
import { initTheme } from "../src/modes/interactive/theme/theme.js";

describe("tool output view dial", () => {
	test("cycles forward and backward, wrapping at both ends", () => {
		expect(TOOL_OUTPUT_VIEWS).toEqual(["radar", "peek", "full"]);
		expect(cycleToolOutputView("radar", "forward")).toBe("peek");
		expect(cycleToolOutputView("peek", "forward")).toBe("full");
		expect(cycleToolOutputView("full", "forward")).toBe("radar");
		expect(cycleToolOutputView("radar", "backward")).toBe("full");
		expect(cycleToolOutputView("full", "backward")).toBe("peek");
	});

	test("recognises its own values and nothing else", () => {
		expect(isToolOutputView("peek")).toBe(true);
		expect(isToolOutputView("glance")).toBe(false);
		expect(isToolOutputView(undefined)).toBe(false);
	});

	test("maps every retired value, and leaves the live ones alone", () => {
		expect(LEGACY_TOOL_OUTPUT_VIEWS).toEqual({ collapsed: "radar", glance: "peek", standard: "full" });
		// `peek` is a live name again, so an old config carrying it needs no
		// migration at all — it already names the stop it always meant.
		expect(LEGACY_TOOL_OUTPUT_VIEWS.peek).toBeUndefined();
	});

	test("the jump key's target is the top of the dial", () => {
		expect(MAX_TOOL_OUTPUT_VIEW).toBe(TOOL_OUTPUT_VIEWS[TOOL_OUTPUT_VIEWS.length - 1]);
		expect(TOOL_OUTPUT_VIEWS).toContain(DEFAULT_TOOL_OUTPUT_VIEW);
		expect(DEFAULT_TOOL_OUTPUT_VIEW).not.toBe(MAX_TOOL_OUTPUT_VIEW);
	});
});

describe("radar signal row", () => {
	test("picks the most identifying argument as the subject", () => {
		const cwd = process.cwd();
		expect(toolSubject({ command: "npm run check" }, cwd)).toBe("npm run check");
		expect(toolSubject({ file_path: "src/a.ts", limit: 20 }, cwd)).toBe("src/a.ts");
		expect(toolSubject({ pattern: "*.test.ts", path: "src" }, cwd)).toBe("*.test.ts");
		// Unknown argument names still yield something rather than a blank row.
		expect(toolSubject({ somethingCustom: "value" }, cwd)).toBe("value");
		expect(toolSubject({ count: 3 }, cwd)).toBe("");
		expect(toolSubject(undefined, cwd)).toBe("");
	});

	test("collapses a multi-line command onto one line", () => {
		expect(toolSubject({ command: "cat <<EOF\nline\nEOF" }, process.cwd())).toBe("cat <<EOF line EOF");
	});

	test("shortens a path inside the working directory", () => {
		const cwd = process.cwd();
		expect(toolSubject({ file_path: `${cwd}/src/main.ts` }, cwd)).toBe("src/main.ts");
	});

	test("reports output size, outcome, or error as the signal", () => {
		const base = { toolName: "bash", args: {}, cwd: process.cwd(), isPartial: false, showImages: false };
		expect(toolSignal({ ...base, result: { content: [{ type: "text", text: "a\nb\nc" }], isError: false } })).toEqual(
			{ text: "3 lines", color: "success" },
		);
		expect(toolSignal({ ...base, result: { content: [{ type: "text", text: "a" }], isError: false } })).toEqual({
			text: "1 line",
			color: "success",
		});
		expect(toolSignal({ ...base, result: { content: [], isError: false } })).toEqual({
			text: "ok",
			color: "success",
		});
		expect(toolSignal({ ...base, result: { content: [{ type: "text", text: "boom" }], isError: true } })).toEqual({
			text: "error",
			color: "error",
		});
		expect(toolSignal({ ...base, isPartial: true })).toEqual({ text: "running", color: "warning" });
	});

	test("aligns the tool column and keeps the row within its width", () => {
		initTheme("dark");
		const row = (toolName: string) =>
			stripAnsi(
				renderToolSignalLine(
					{
						toolName,
						args: { command: "npm run check" },
						cwd: process.cwd(),
						result: { content: [{ type: "text", text: "a\nb" }], isError: false },
						isPartial: false,
						showImages: false,
					},
					80,
				),
			);

		const bash = row("bash");
		const websearch = row("websearch");
		// Same subject start column regardless of how long the tool name is.
		expect(bash.indexOf("npm run check")).toBe(websearch.indexOf("npm run check"));
		expect(bash.endsWith("2 lines")).toBe(true);
		expect(bash.length).toBeLessThanOrEqual(80);
	});

	test("truncates a long subject rather than pushing out the signal", () => {
		initTheme("dark");
		const row = stripAnsi(
			renderToolSignalLine(
				{
					toolName: "bash",
					args: { command: "x".repeat(400) },
					cwd: process.cwd(),
					result: { content: [{ type: "text", text: "a" }], isError: false },
					isPartial: false,
					showImages: false,
				},
				60,
			),
		);
		expect(row.length).toBeLessThanOrEqual(60);
		expect(row.endsWith("1 line")).toBe(true);
	});
});
