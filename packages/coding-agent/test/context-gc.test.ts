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

function readRangeCall(id: string, path: string, offset?: number, limit?: number): AgentMessage {
	return {
		role: "assistant",
		content: [{ type: "toolCall", id, name: "read", arguments: { path, offset, limit } }],
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

describe("evictSupersededReads — offset-aware read ranges", () => {
	it("keeps two reads of disjoint line ranges of the same file", () => {
		const messages = [
			readRangeCall("c1", "src/big.rs", 1, 40),
			readResult("c1", "HEADER REGION"),
			readRangeCall("c2", "src/big.rs", 200, 60),
			readResult("c2", "TAIL REGION"),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		// Neither read overlaps the other, so both survive untouched.
		expect(out).toBe(messages);
		expect(textOf(out[1])).toBe("HEADER REGION");
		expect(textOf(out[3])).toBe("TAIL REGION");
	});

	it("does not ping-pong between two alternating non-overlapping regions", () => {
		// Reproduces the read loop: the model alternates between two disjoint
		// windows of one file. Every region must stay hydrated; nothing is stubbed.
		const messages = [
			readRangeCall("c1", "src/big.rs", 609, 12),
			readResult("c1", "SSE CONST REGION"),
			readRangeCall("c2", "src/big.rs", 621, 40),
			readResult("c2", "ASSERTIONS REGION"),
			readRangeCall("c3", "src/big.rs", 609, 12),
			readResult("c3", "SSE CONST REGION"),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		// c1 is superseded by c3 (same 609 region), so it is stubbed; c2 (disjoint)
		// and c3 (latest) both stay hydrated.
		expect(textOf(out[1])).toContain("Superseded read");
		expect(textOf(out[3])).toBe("ASSERTIONS REGION");
		expect(textOf(out[5])).toBe("SSE CONST REGION");
	});

	it("stubs an earlier read when a later read overlaps its range", () => {
		const messages = [
			readRangeCall("c1", "src/big.rs", 10, 50), // lines 10-59
			readResult("c1", "FIRST"),
			readRangeCall("c2", "src/big.rs", 40, 50), // lines 40-89, overlaps 40-59
			readResult("c2", "SECOND"),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		expect(textOf(out[1])).toContain("Superseded read");
		expect(textOf(out[3])).toBe("SECOND");
	});

	it("treats a whole-file read (no offset/limit) as overlapping any ranged read", () => {
		const messages = [
			readRangeCall("c1", "src/big.rs", 500, 20),
			readResult("c1", "NARROW WINDOW"),
			readRangeCall("c2", "src/big.rs"), // whole file
			readResult("c2", "WHOLE FILE"),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		expect(textOf(out[1])).toContain("Superseded read");
		expect(textOf(out[3])).toBe("WHOLE FILE");
	});

	it("stubs all disjoint reads of a file once it is mutated", () => {
		const messages = [
			readRangeCall("c1", "src/big.rs", 1, 40),
			readResult("c1", "HEADER REGION"),
			readRangeCall("c2", "src/big.rs", 200, 60),
			readResult("c2", "TAIL REGION"),
			assistantCall("c3", "edit", "src/big.rs"),
			mutateResult("c3", "edit"),
		];
		const out = evictSupersededReads(messages, { cwd: CWD });
		// A mutate changes the whole file on disk, so every earlier read is stale.
		expect(textOf(out[1])).toContain("the file was modified after this read");
		expect(textOf(out[3])).toContain("the file was modified after this read");
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
