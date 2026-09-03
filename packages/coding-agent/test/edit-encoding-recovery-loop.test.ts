/**
 * The read -> edit -> context-GC loop, driven end to end.
 *
 * A file holds a character the model cannot reproduce from its own rendering: a
 * non-breaking space pasted in from a document, a smart quote, a tab. The model
 * reads the file, retypes the line in plain ASCII, and asks for an edit. That
 * used to produce a loop with no exit:
 *
 *   - the edit failed with "No changes made", naming nothing actionable;
 *   - the at-call read dedup then refused to re-fetch the file, on the grounds
 *     that a failed edit had not changed it, so the model could never see the
 *     character it was missing;
 *   - every retry re-entered the same state.
 *
 * These tests run the real read tool, the real edit tool and the real context GC
 * over a real transcript, with a "model" that recovers only from what the tools
 * actually tell it. Convergence is the assertion.
 *
 * Every non-ASCII character here is written as an escape on purpose. A literal
 * non-breaking space in a fixture is invisible in review and does not survive
 * every tool that touches the file - which is the bug these tests cover.
 *
 * File access goes through `fs/promises` rather than the sync API because
 * `restore-sandbox-env.test.ts` installs a module-wide `vi.mock("node:fs")`
 * whose stub leaks into whatever runs after it in the same process.
 */

import { mkdtemp, readFile, writeFile } from "fs/promises";
import { tmpdir } from "os";
import { join } from "path";
import { describe, expect, it } from "vitest";
import { evictSupersededReads } from "../src/core/context-gc.js";
import { createEditToolDefinition, createReadToolDefinition } from "../src/index.js";

type Msg = {
	role: string;
	content: Array<{ type: string; id?: string; name?: string; arguments?: Record<string, unknown>; text?: string }>;
	toolCallId?: string;
	toolName?: string;
	isError?: boolean;
};

/** A transcript plus the read/edit drivers that append to it, as a session does. */
function makeHarness(cwd: string) {
	const readTool = createReadToolDefinition(cwd, { dedupReads: true });
	const editTool = createEditToolDefinition(cwd);
	const msgs: Msg[] = [];
	let seq = 0;

	const call = (name: string, args: Record<string, unknown>) => {
		const id = `${name}-${++seq}`;
		msgs.push({ role: "assistant", content: [{ type: "toolCall", id, name, arguments: args }] });
		return id;
	};
	const record = (id: string, toolName: string, text: string, isError: boolean) => {
		msgs.push({ role: "toolResult", toolCallId: id, toolName, content: [{ type: "text", text }], isError });
	};

	return {
		msgs,
		async read(path: string, args: { offset?: number; limit?: number } = {}) {
			const id = call("read", { path, ...args });
			const r = (await readTool.execute(id, { path, ...args }, undefined, undefined, {
				sessionManager: { getBranch: () => msgs },
			} as never)) as { content: Array<{ type: string; text?: string }> };
			const text = r.content
				.filter((c) => c.type === "text")
				.map((c) => c.text ?? "")
				.join("");
			record(id, "read", text, false);
			return text;
		},
		async edit(path: string, edits: Array<{ oldText: string; newText: string }>) {
			const id = call("edit", { path, edits });
			try {
				const r = (await editTool.execute(id, { path, edits }, undefined, undefined, {} as never)) as {
					content: Array<{ text?: string }>;
				};
				const text = r.content.map((c) => c.text ?? "").join("");
				record(id, "edit", text, false);
				return { ok: true, text };
			} catch (e) {
				const text = e instanceof Error ? e.message : String(e);
				record(id, "edit", text, true);
				return { ok: false, text };
			}
		},
		gc: () => evictSupersededReads(msgs as never, { cwd }),
	};
}

/** Characters a model routinely re-emits as their plain-ASCII lookalike. */
const DRIFT: Array<[RegExp, string, number]> = [
	[/\u00a0/g, " ", 0x00a0],
	[/[\u2018\u2019]/g, "'", 0x2019],
	[/[\u201c\u201d]/g, '"', 0x201d],
	[/\t/g, "  ", 0x0009],
];

/**
 * Stand-in for a model retyping a line it just read: every code point in `keep`
 * is reproduced exactly, everything else drifts to its ASCII lookalike.
 */
function retype(line: string, keep: Set<number>): string {
	let out = line;
	for (const [pattern, ascii, cp] of DRIFT) {
		if (!keep.has(cp)) out = out.replace(pattern, ascii);
	}
	return out;
}

/** What a model can learn from the tool's own error text. */
function codePointsNamedIn(message: string): number[] {
	return [...message.matchAll(/U\+([0-9A-F]{4,6})/g)].map((m) => Number.parseInt(m[1], 16));
}

function tempDir(): Promise<string> {
	return mkdtemp(join(tmpdir(), "edit-recovery-loop-"));
}

describe("read -> edit -> gc recovery loop", () => {
	it("converges when the model's goal is the character it cannot retype", async () => {
		const cwd = await tempDir();
		const path = "greet.ts";
		// The model's task is to replace the non-breaking space with a real one. That
		// is the case with no way out before these fixes: retyping the line in ASCII
		// makes oldText and newText identical to the matcher, so the edit can only
		// fail, and the failure used to name nothing and block the re-read as well.
		const original = 'export function greet(name: string) {\n\tconst msg = "Hello\u00a0world";\n}\n';
		await writeFile(join(cwd, path), original);
		const h = makeHarness(cwd);

		const keep = new Set<number>();
		let applied = false;
		let rounds = 0;

		// The loop a model actually runs: read, attempt, learn from the error, repeat.
		while (!applied && rounds < 5) {
			rounds++;
			const seen = await h.read(path);
			// The re-read must hand over real bytes, not a "nothing changed" pointer.
			expect(seen).not.toContain("[Already in context:");
			const typed = retype(seen.split("\n")[1], keep);

			const res = await h.edit(path, [{ oldText: typed, newText: typed.replace(/\u00a0/g, " ") }]);
			if (res.ok) {
				applied = true;
				break;
			}
			// The error text is the only recovery signal.
			for (const cp of codePointsNamedIn(res.text)) keep.add(cp);
			h.gc();
		}

		expect(applied).toBe(true);
		expect(rounds).toBeLessThanOrEqual(3);

		// The change it asked for landed, and the tab it retyped as spaces did not move.
		expect(await readFile(join(cwd, path), "utf-8")).toBe(
			'export function greet(name: string) {\n\tconst msg = "Hello world";\n}\n',
		);
	});

	it("lets the model re-read after a failed edit instead of pointing at the stale read", async () => {
		const cwd = await tempDir();
		const path = "quote.ts";
		await writeFile(join(cwd, path), `const label = \u201cwidget\u201d;\n`);
		const h = makeHarness(cwd);

		await h.read(path);
		// A smart-quote cleanup: oldText and newText are the same ASCII text, so the
		// change is invisible to the matcher and the edit can only fail.
		const failed = await h.edit(path, [{ oldText: 'const label = "widget";', newText: 'const label = "widget";' }]);
		expect(failed.ok).toBe(false);

		// This is the read that used to come back as a dedup pointer.
		const again = await h.read(path);
		expect(again).not.toContain("[Already in context:");
		expect(again).toContain(`\u201cwidget\u201d`);
	});

	it("names the characters that are actually blocking the edit", async () => {
		const cwd = await tempDir();
		const path = "nbsp.ts";
		await writeFile(join(cwd, path), '\tconst msg = "Hello\u00a0world";\n');
		const h = makeHarness(cwd);

		const res = await h.edit(path, [
			{ oldText: '  const msg = "Hello world";', newText: '  const msg = "Hello world";' },
		]);
		expect(res.ok).toBe(false);
		expect(res.text).toContain("No changes made");
		// Both offenders are named, so the model is never left guessing which one.
		expect(codePointsNamedIn(res.text)).toEqual(expect.arrayContaining([0x0009, 0x00a0]));
		expect(res.text).toContain("NO-BREAK SPACE");
	});

	it("fails a no-op edit hidden among edits that do change bytes", async () => {
		const cwd = await tempDir();
		const path = "batch.ts";
		const original = `const a = 1;\nconst b = \u201c2\u201d;\nconst c = 3;\n`;
		await writeFile(join(cwd, path), original);
		const h = makeHarness(cwd);

		// edits[1] normalizes to itself. It used to be swallowed while the call
		// reported "Successfully replaced 2 block(s)".
		const res = await h.edit(path, [
			{ oldText: "const a = 1;", newText: "const a = 10;" },
			{ oldText: 'const b = "2";', newText: 'const b = "2";' },
		]);
		expect(res.ok).toBe(false);
		expect(res.text).toContain("edits[1]");
		expect(await readFile(join(cwd, path), "utf-8")).toBe(original);
	});

	it("does not let an indented oldText match inside a deeper-indented line", async () => {
		const cwd = await tempDir();
		const path = "nesting.py";
		const original = "def handler(req):\n    return check(req)\n\ndef fallback(req):\n        return validate(req)\n";
		await writeFile(join(cwd, path), original);
		const h = makeHarness(cwd);

		// The only `return validate(req)` sits at eight spaces, inside fallback().
		// A four-space oldText names a different nesting level and must not reach it.
		const res = await h.edit(path, [
			{ oldText: "    return validate(req)", newText: "    return validate(req, strict=True)" },
		]);
		expect(res.ok).toBe(false);
		expect(await readFile(join(cwd, path), "utf-8")).toBe(original);
	});

	it("still edits the right line when the indentation genuinely matches", async () => {
		const cwd = await tempDir();
		const path = "ok.py";
		await writeFile(join(cwd, path), "def handler(req):\n    return validate(req)\n");
		const h = makeHarness(cwd);

		const res = await h.edit(path, [
			{ oldText: "    return validate(req)", newText: "    return validate(req, strict=True)" },
		]);
		expect(res.ok).toBe(true);
		expect(await readFile(join(cwd, path), "utf-8")).toBe(
			"def handler(req):\n    return validate(req, strict=True)\n",
		);
	});

	it("keeps GC evicting the superseded read once the loop has moved on", async () => {
		const cwd = await tempDir();
		const path = "gc.ts";
		await writeFile(join(cwd, path), "const v = 1;\n");
		const h = makeHarness(cwd);

		await h.read(path);
		await h.edit(path, [{ oldText: "const v = 1;", newText: "const v = 2;" }]);
		await h.read(path);

		const out = h.gc() as unknown as Msg[];
		const reads = out.filter((m) => m.role === "toolResult" && m.toolName === "read");
		expect(reads[0].content[0].text).toContain("Superseded read");
		expect(reads[1].content[0].text).toContain("const v = 2;");
	});
});
