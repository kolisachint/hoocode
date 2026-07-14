import { resolve } from "node:path";
import type { AgentMessage } from "@kolisachint/hoocode-agent-core";
import { describe, expect, it } from "vitest";
import { evictSupersededReads } from "../src/core/context-gc.js";

const CWD = "/project";

function assistantCall(id: string, name: string, path: string): AgentMessage {
	return {
		role: "assistant",
		content: [{ type: "toolCall", id, name, arguments: { path } }],
	} as unknown as AgentMessage;
}

function bashCall(id: string, command: string): AgentMessage {
	return {
		role: "assistant",
		content: [{ type: "toolCall", id, name: "bash", arguments: { command } }],
	} as unknown as AgentMessage;
}

function bashResult(id: string, text: string, isError = false): AgentMessage {
	return {
		role: "toolResult",
		toolCallId: id,
		toolName: "bash",
		content: [{ type: "text", text }],
		isError,
	} as unknown as AgentMessage;
}

function readResult(id: string, text: string): AgentMessage {
	return {
		role: "toolResult",
		toolCallId: id,
		toolName: "read",
		content: [{ type: "text", text }],
		isError: false,
	} as unknown as AgentMessage;
}

function mutateResult(id: string, tool: "edit" | "write", isError = false): AgentMessage {
	return {
		role: "toolResult",
		toolCallId: id,
		toolName: tool,
		content: [{ type: "text", text: "ok" }],
		isError,
	} as unknown as AgentMessage;
}

function textOf(m: AgentMessage): string {
	return (m as unknown as { content: Array<{ text?: string }> }).content.map((c) => c.text ?? "").join("");
}

describe("evictSupersededReads", () => {
	it("stubs a read once the same file is edited later", () => {
		const messages = [
			assistantCall("c1", "read", "src/a.ts"),
			readResult("c1", "ORIGINAL CONTENTS OF A"),
			assistantCall("c2", "edit", "src/a.ts"),
			mutateResult("c2", "edit"),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		expect(textOf(out[1])).not.toContain("ORIGINAL CONTENTS OF A");
		expect(textOf(out[1])).toContain("Superseded read of src/a.ts");
		expect(textOf(out[1])).toContain("modified after this read");
	});

	it("keeps the most recent read and stubs the earlier one", () => {
		const messages = [
			assistantCall("c1", "read", "a.ts"),
			readResult("c1", "FIRST READ"),
			assistantCall("c2", "read", "a.ts"),
			readResult("c2", "SECOND READ"),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		expect(textOf(out[1])).toContain("read again later");
		expect(textOf(out[3])).toBe("SECOND READ");
	});

	it("does not evict a read whose edit failed", () => {
		const messages = [
			assistantCall("c1", "read", "a.ts"),
			readResult("c1", "KEEP ME"),
			assistantCall("c2", "edit", "a.ts"),
			mutateResult("c2", "edit", true),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		expect(textOf(out[1])).toBe("KEEP ME");
	});

	it("does not evict reads of different files", () => {
		const messages = [
			assistantCall("c1", "read", "a.ts"),
			readResult("c1", "A"),
			assistantCall("c2", "edit", "b.ts"),
			mutateResult("c2", "edit"),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		expect(textOf(out[1])).toBe("A");
		expect(out).toBe(messages); // no change → same reference
	});

	it("matches relative and absolute paths for the same file", () => {
		const messages = [
			assistantCall("c1", "read", "src/a.ts"),
			readResult("c1", "ORIG"),
			assistantCall("c2", "edit", resolve(CWD, "src/a.ts")),
			mutateResult("c2", "edit"),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		expect(textOf(out[1])).toContain("Superseded read");
	});

	it("keeps a single read of a file untouched", () => {
		const messages = [assistantCall("c1", "read", "a.ts"), readResult("c1", "ONLY READ")];
		const out = evictSupersededReads(messages, { cwd: CWD });
		expect(out).toBe(messages);
		expect(textOf(out[1])).toBe("ONLY READ");
	});
});

describe("evictSupersededReads — bash output eviction", () => {
	const small = "x".repeat(100);
	const large = "x".repeat(2500);

	it("does not evict bash output at zero pressure", () => {
		const messages = [bashCall("b1", "ls -la"), bashResult("b1", large)];
		const out = evictSupersededReads(messages, { cwd: CWD });
		expect(out).toBe(messages);
		expect(textOf(out[1])).toBe(large);
	});

	it("does not evict a small bash output at moderate pressure", () => {
		const messages = [bashCall("b1", "ls -la"), bashResult("b1", small)];
		const out = evictSupersededReads(messages, { cwd: CWD, budgetPressure: 0.65 });
		expect(out).toBe(messages);
		expect(textOf(out[1])).toBe(small);
	});

	it("evicts a large bash output at moderate pressure with a budget stub", () => {
		const messages = [bashCall("b1", "cat huge.log"), bashResult("b1", large)];
		const out = evictSupersededReads(messages, { cwd: CWD, budgetPressure: 0.65 });
		expect(textOf(out[1])).not.toContain("xxxx");
		expect(textOf(out[1])).toContain("65% token budget");
	});

	it("never evicts side-effecting commands even when large (psql)", () => {
		const messages = [bashCall("b1", "psql -c 'select 1'"), bashResult("b1", large)];
		const out = evictSupersededReads(messages, { cwd: CWD, budgetPressure: 0.65 });
		expect(out).toBe(messages);
		expect(textOf(out[1])).toBe(large);
	});

	it("never evicts side-effecting commands even at high pressure (rm -rf)", () => {
		const messages = [bashCall("b1", "echo hi && rm -rf dist"), bashResult("b1", small)];
		const out = evictSupersededReads(messages, { cwd: CWD, budgetPressure: 0.85 });
		expect(out).toBe(messages);
		expect(textOf(out[1])).toBe(small);
	});

	it("never evicts a redirecting command (output side effect)", () => {
		const messages = [bashCall("b1", "echo data > out.txt"), bashResult("b1", small)];
		const out = evictSupersededReads(messages, { cwd: CWD, budgetPressure: 0.85 });
		expect(out).toBe(messages);
		expect(textOf(out[1])).toBe(small);
	});

	it("evicts even a small bash output at high pressure", () => {
		const messages = [bashCall("b1", "ls"), bashResult("b1", small)];
		const out = evictSupersededReads(messages, { cwd: CWD, budgetPressure: 0.85 });
		expect(textOf(out[1])).toContain("85% token budget");
	});

	it("never evicts an errored bash result", () => {
		const messages = [bashCall("b1", "cat huge.log"), bashResult("b1", large, true)];
		const out = evictSupersededReads(messages, { cwd: CWD, budgetPressure: 0.85 });
		expect(out).toBe(messages);
		expect(textOf(out[1])).toBe(large);
	});
});
