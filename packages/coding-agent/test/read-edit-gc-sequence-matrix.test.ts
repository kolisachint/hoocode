/**
 * The read/edit/GC sequence matrix.
 *
 * The three tools that share a file - `read`, `edit` and the context GC - are
 * only correct in combination, and the failures found in this area were all
 * ordering bugs: a pointer served after a failed edit, a read evicted while it
 * was still the model's only copy, a success reported for an edit that changed
 * nothing. Testing one ordering by hand misses the rest.
 *
 * So every sequence over {R, r, E, X, G} up to length three is run against a
 * fresh file with the real tools, and each step is checked against the
 * invariants the trio is supposed to guarantee. 155 sequences; before the fixes
 * these covered 52 distinct dead-end orderings.
 */

import { mkdtemp, readFile, writeFile } from "fs/promises";
import { tmpdir } from "os";
import { join } from "path";
import { describe, expect, it } from "vitest";
import { evictSupersededReads } from "../src/core/context-gc.js";
import { createEditToolDefinition, createReadToolDefinition } from "../src/index.js";

/**
 * R  read the whole file        r  read a line range
 * E  an edit that should apply  X  an edit invisible to matching, which can only fail
 * G  a context-GC pass over the transcript
 */
const OPS = ["R", "r", "E", "X", "G"] as const;
type Op = (typeof OPS)[number];

// Line 4 carries a non-breaking space, so `X` (which spells it with an ordinary
// space) is a change the matcher cannot see and the edit can only fail.
const INITIAL = `line1 alpha\nline2 beta\nline3 gamma\nline4\u00a0delta\n`;

interface Violation {
	sequence: string;
	rule: string;
}

/** Exactly what `read` should deliver for these args against this content. */
function delivered(disk: string, args: { offset?: number; limit?: number }): string {
	const lines = disk.split("\n");
	const start = (args.offset ?? 1) - 1;
	return lines.slice(start, args.limit === undefined ? lines.length : start + args.limit).join("\n");
}

/** Drop the continuation notice `read` appends when it stops short. */
function stripNotice(text: string): string {
	return text.replace(/\n\n\[(Showing lines|\d+ more lines) [^\]]*\]\s*$/, "");
}

const isPointer = (t: string) => t.startsWith("[Already in context:");
const isStub = (t: string) => t.startsWith("[Superseded read");

async function runSequence(seq: Op[]): Promise<Violation[]> {
	const cwd = await mkdtemp(join(tmpdir(), "sequence-matrix-"));
	const path = "f.txt";
	const abs = join(cwd, path);
	await writeFile(abs, INITIAL);

	const readTool = createReadToolDefinition(cwd, { dedupReads: true });
	const editTool = createEditToolDefinition(cwd);
	const msgs: Array<Record<string, unknown>> = [];
	const readLog: Array<{ msgIndex: number; args: { offset?: number; limit?: number }; text: string }> = [];
	let view: Array<Record<string, unknown>> = msgs;
	let seq_ = 0;
	let editCounter = 0;
	let lastEditFailed = false;

	const found: Violation[] = [];
	const label = seq.join("");
	const flag = (rule: string) => found.push({ sequence: label, rule });
	const textOf = (m: unknown) => String((m as { content?: Array<{ text?: string }> })?.content?.[0]?.text ?? "");

	for (const op of seq) {
		const disk = await readFile(abs, "utf-8");

		if (op === "R" || op === "r") {
			const args = op === "R" ? {} : { offset: 2, limit: 2 };
			const id = `call-${++seq_}`;
			msgs.push({
				role: "assistant",
				content: [{ type: "toolCall", id, name: "read", arguments: { path, ...args } }],
			});
			const res = (await readTool.execute(id, { path, ...args }, undefined, undefined, {
				sessionManager: { getBranch: () => msgs },
			} as never)) as { content: Array<{ type: string; text?: string }> };
			const text = res.content
				.filter((c) => c.type === "text")
				.map((c) => c.text ?? "")
				.join("");
			const msgIndex = msgs.length;
			msgs.push({
				role: "toolResult",
				toolCallId: id,
				toolName: "read",
				content: [{ type: "text", text }],
				isError: false,
			});

			if (isPointer(text)) {
				const covering = readLog.filter((r) => {
					const rs = r.args.offset ?? 1;
					const re = r.args.limit === undefined ? Number.POSITIVE_INFINITY : rs + r.args.limit;
					const qs = args.offset ?? 1;
					const qe = args.limit === undefined ? Number.POSITIVE_INFINITY : qs + args.limit;
					return rs <= qs && re >= qe;
				});
				const c = covering.at(-1);
				if (!c) {
					flag("a pointer named no earlier read that covers the request");
				} else {
					if (stripNotice(c.text) !== delivered(disk, c.args)) {
						flag("a pointer claimed the file was unchanged when it was not");
					}
					if (isStub(textOf(view[c.msgIndex]))) {
						flag("a pointer named a read that GC had already elided");
					}
				}
				if (lastEditFailed) flag("a re-read after a failed edit was answered with a pointer");
			} else {
				if (stripNotice(text) !== delivered(disk, args))
					flag("a read returned content that is not what is on disk");
				readLog.push({ msgIndex, args, text });
			}
			lastEditFailed = false;
			continue;
		}

		if (op === "E" || op === "X") {
			const targets = ["line1 alpha", "line2 beta", "line3 gamma"];
			const target = targets[editCounter++ % targets.length];
			const edits =
				op === "E"
					? [{ oldText: target, newText: `${target} EDIT${editCounter}` }]
					: [{ oldText: "line4 delta", newText: "line4 delta" }];
			const id = `call-${++seq_}`;
			msgs.push({
				role: "assistant",
				content: [{ type: "toolCall", id, name: "edit", arguments: { path, edits } }],
			});
			let ok = true;
			try {
				await editTool.execute(id, { path, edits }, undefined, undefined, {} as never);
				msgs.push({
					role: "toolResult",
					toolCallId: id,
					toolName: "edit",
					content: [{ type: "text", text: "ok" }],
					isError: false,
				});
			} catch (e) {
				ok = false;
				const text = e instanceof Error ? e.message : String(e);
				msgs.push({
					role: "toolResult",
					toolCallId: id,
					toolName: "edit",
					content: [{ type: "text", text }],
					isError: true,
				});
			}
			const after = await readFile(abs, "utf-8");
			if (ok && after === disk) flag("an edit reported success without changing the file");
			if (!ok && after !== disk) flag("an edit that failed still wrote to the file");
			if (op === "X" && ok) flag("an edit invisible to matching reported success");
			if (op === "E" && !ok) flag("a well-formed edit was rejected");
			lastEditFailed = !ok;
			continue;
		}

		view = evictSupersededReads(msgs as never, { cwd }) as never;
		for (const r of readLog) {
			if (!isStub(textOf(view[r.msgIndex]))) continue;
			const laterMutate = msgs.some(
				(m, j) =>
					j > r.msgIndex &&
					m.role === "toolResult" &&
					(m.toolName === "edit" || m.toolName === "write") &&
					!m.isError,
			);
			const laterRead = readLog.some((o) => o.msgIndex > r.msgIndex);
			if (!laterMutate && !laterRead) flag("GC elided a read that nothing had superseded");
		}
	}
	return found;
}

function allSequences(maxLength: number): Op[][] {
	const out: Op[][] = [];
	const walk = (prefix: Op[]) => {
		if (prefix.length > 0) out.push(prefix);
		if (prefix.length === maxLength) return;
		for (const op of OPS) walk([...prefix, op]);
	};
	walk([]);
	return out;
}

describe("read/edit/gc sequence matrix", () => {
	it("holds every invariant across all sequences up to length three", async () => {
		const sequences = allSequences(3);
		expect(sequences).toHaveLength(155);

		const violations: Violation[] = [];
		for (const seq of sequences) violations.push(...(await runSequence(seq)));

		// Reported as `sequence: rule` so a failure names the exact ordering.
		expect(violations.map((v) => `${v.sequence}: ${v.rule}`)).toEqual([]);
	}, 120_000);
});
