import type { AgentTool } from "@kolisachint/hoocode-agent-core";
import { fauxAssistantMessage, fauxToolCall } from "@kolisachint/hoocode-ai";
import { Type } from "typebox";
import { afterEach, describe, expect, it } from "vitest";
import { resolveReadPath, resolveToCwd } from "../../../src/core/tools/path-utils.js";
import { createHarness, type Harness } from "../harness.js";

/**
 * Regression test for ERR_SUPPORTED_ESM_URL_SCHEME error on Windows with Bun.
 * File URLs (file:///) must be normalized to file paths when passed to fs operations.
 *
 * The normalization happens in expandPath() which is used by resolveReadPath() and resolveToCwd().
 * These are called from tool execution functions to resolve the absolute path.
 *
 * Note: Tools in actual hoocode use resolveReadPath() or resolveToCwd() internally to get
 * the absolute path for file operations. The normalization of file:// URLs to paths happens
 * during this resolution process.
 */
describe("regression #windows-esm-url-scheme: normalize file URLs to paths", () => {
	const harnesses: Harness[] = [];

	afterEach(() => {
		while (harnesses.length > 0) {
			harnesses.pop()?.cleanup();
		}
	});

	/**
	 * Create a harness with a custom tool that uses resolveReadPath/resolveToCwd
	 * to normalize file URLs. This simulates what actual Read/Write tools do.
	 */
	async function createHarnessWithPathResolvingTool(
		toolName: string,
		captureRef: { path: string | undefined },
		useResolveReadPath: boolean,
	): Promise<Harness> {
		const harness = await createHarness({
			tools: [],
		});
		harnesses.push(harness);

		// The tool will call resolveReadPath/resolveToCwd to get the absolute path
		// This is what actual tools do internally
		const tool: AgentTool = {
			name: toolName,
			label: toolName,
			description: "Test tool using path resolution",
			parameters: Type.Object({ path: Type.String() }),
			execute: async (_toolCallId, params) => {
				const p = params as { path?: string };
				const absolutePath = useResolveReadPath
					? resolveReadPath(p.path ?? "", harness.tempDir)
					: resolveToCwd(p.path ?? "", harness.tempDir);
				captureRef.path = absolutePath;
				return {
					content: [{ type: "text", text: "ok" }],
					details: {},
				};
			},
		};

		// Create a new harness with the tool
		const toolHarness = await createHarness({
			tools: [tool],
		});
		harnesses.push(toolHarness);

		// Clean up the first harness since we're replacing it
		harnesses.splice(harnesses.indexOf(harness), 1);
		harness.cleanup();

		return toolHarness;
	}

	it("converts Windows file URL to path using resolveReadPath", async () => {
		const captured = { path: undefined as string | undefined };
		const harness = await createHarnessWithPathResolvingTool("Read", captured, true);

		harness.setResponses([
			fauxAssistantMessage([fauxToolCall("Read", { path: "file:///C:/project/test.txt" })], {
				stopReason: "toolUse",
			}),
			fauxAssistantMessage("File read successfully"),
		]);

		await harness.session.prompt("Read the file at file:///C:/project/test.txt");

		expect(harness.faux.state.callCount).toBe(2);
		// The resolved path should NOT contain file:///
		expect(captured.path).not.toContain("file:///");
	});

	it("converts Windows file URL to path using resolveToCwd", async () => {
		const captured = { path: undefined as string | undefined };
		const harness = await createHarnessWithPathResolvingTool("Write", captured, false);

		harness.setResponses([
			fauxAssistantMessage(
				[fauxToolCall("Write", { path: "file:///D:/output/result.json", content: '{"key": "value"}' })],
				{ stopReason: "toolUse" },
			),
			fauxAssistantMessage("File written successfully"),
		]);

		await harness.session.prompt("Write to file:///D:/output/result.json");

		expect(harness.faux.state.callCount).toBe(2);
		// The resolved path should NOT contain file:///
		expect(captured.path).not.toContain("file:///");
	});

	it("preserves normal paths without URL scheme", async () => {
		const captured = { path: undefined as string | undefined };
		const harness = await createHarnessWithPathResolvingTool("Read", captured, true);

		harness.setResponses([
			fauxAssistantMessage([fauxToolCall("Read", { path: "relative/path/file.txt" })], { stopReason: "toolUse" }),
			fauxAssistantMessage("File contents"),
		]);

		await harness.session.prompt("Read relative/path/file.txt");

		expect(harness.faux.state.callCount).toBe(2);
		// Normal path should be preserved (after resolution it becomes absolute)
		expect(captured.path).toBeDefined();
		expect(captured.path).not.toContain("file:///");
	});

	it("converts Unix-style file URL", async () => {
		const captured = { path: undefined as string | undefined };
		const harness = await createHarnessWithPathResolvingTool("Read", captured, true);

		harness.setResponses([
			fauxAssistantMessage([fauxToolCall("Read", { path: "file:///home/user/document.txt" })], {
				stopReason: "toolUse",
			}),
			fauxAssistantMessage("Document contents"),
		]);

		await harness.session.prompt("Read /home/user/document.txt");

		expect(harness.faux.state.callCount).toBe(2);
		// The resolved path should NOT contain file:///
		expect(captured.path).not.toContain("file:///");
	});

	it("converts file URL with special characters", async () => {
		const captured = { path: undefined as string | undefined };
		const harness = await createHarnessWithPathResolvingTool("Read", captured, true);

		harness.setResponses([
			fauxAssistantMessage([fauxToolCall("Read", { path: "file:///E:/Users/test%20name/data%2Ffile.txt" })], {
				stopReason: "toolUse",
			}),
			fauxAssistantMessage("File contents"),
		]);

		await harness.session.prompt("Read the file");

		expect(harness.faux.state.callCount).toBe(2);
		// Path should not be a file URL
		expect(captured.path).not.toContain("file:///");
	});
});
