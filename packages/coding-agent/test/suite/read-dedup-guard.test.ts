import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { fauxAssistantMessage, fauxToolCall } from "@kolisachint/hoocode-ai";
import { afterEach, describe, expect, it } from "vitest";
import { DEDUP_POINTER_PREFIX } from "../../src/core/tools/read-dedup.js";
import { createHarness, type Harness } from "./harness.js";

/**
 * End-to-end coverage for the at-call read dedup guard through the *real* tool
 * pipeline (so the read tool actually receives `ctx.sessionManager`). Drives two
 * reads of the same file in one session via the faux provider and checks the
 * second is short-circuited to a pointer instead of re-fetching the content.
 */

function readResultTexts(harness: Harness): string[] {
	return harness.session.messages
		.filter((m): m is typeof m & { role: "toolResult"; toolName: string } => {
			const r = (m as { role?: string; toolName?: string }).role;
			return r === "toolResult" && (m as { toolName?: string }).toolName === "read";
		})
		.map((m) => {
			const content = (m as { content?: Array<{ type: string; text?: string }> }).content ?? [];
			return content
				.filter((c) => c.type === "text")
				.map((c) => c.text ?? "")
				.join("");
		});
}

describe("read dedup guard (real tool pipeline)", () => {
	const harnesses: Harness[] = [];
	afterEach(() => {
		while (harnesses.length > 0) harnesses.pop()?.cleanup();
	});

	it("short-circuits a re-read of a file already read this session", async () => {
		const harness = await createHarness({ useRealBuiltinTools: true });
		harnesses.push(harness);
		const file = join(harness.tempDir, "doc.md");
		writeFileSync(file, "line one\nline two\nline three\n");

		harness.setResponses([
			fauxAssistantMessage([fauxToolCall("read", { path: file })], { stopReason: "toolUse" }),
			fauxAssistantMessage([fauxToolCall("read", { path: file })], { stopReason: "toolUse" }),
			fauxAssistantMessage("done"),
		]);
		await harness.session.prompt("read the doc twice");

		const results = readResultTexts(harness);
		expect(results).toHaveLength(2);
		// First read returns the real content.
		expect(results[0]).toContain("line two");
		expect(results[0]).not.toContain(DEDUP_POINTER_PREFIX);
		// Second read is deduped to a pointer, not the content.
		expect(results[1]).toContain(DEDUP_POINTER_PREFIX);
		expect(results[1]).not.toContain("line two");
	});

	it("does not short-circuit when contextGc (the guard gate) is disabled", async () => {
		const harness = await createHarness({
			useRealBuiltinTools: true,
			settings: { contextGc: { enabled: false } },
		});
		harnesses.push(harness);
		const file = join(harness.tempDir, "doc.md");
		writeFileSync(file, "alpha\nbeta\ngamma\n");

		harness.setResponses([
			fauxAssistantMessage([fauxToolCall("read", { path: file })], { stopReason: "toolUse" }),
			fauxAssistantMessage([fauxToolCall("read", { path: file })], { stopReason: "toolUse" }),
			fauxAssistantMessage("done"),
		]);
		await harness.session.prompt("read the doc twice");

		const results = readResultTexts(harness);
		expect(results).toHaveLength(2);
		// Both reads return the real content; nothing is deduped.
		expect(results[1]).toContain("beta");
		expect(results[1]).not.toContain(DEDUP_POINTER_PREFIX);
	});
});
