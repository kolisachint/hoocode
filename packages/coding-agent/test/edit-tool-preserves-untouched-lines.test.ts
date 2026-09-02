import { mkdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createEditTool, createReadTool } from "../src/index.js";

/**
 * A fuzzy or indentation-tolerant match must rewrite only the lines it landed
 * on. Before this was fixed, any edit that fell through to the fuzzy tier
 * replaced the file with a fully normalized copy of itself - every tab widened
 * to two spaces and every interior run of spaces collapsed, across lines the
 * edit never referenced - while the diff handed to the permission gate compared
 * normalized against normalized and so displayed only the intended change.
 */
describe("edit tool preserves untouched lines", () => {
	let testDir: string;
	let editTool: ReturnType<typeof createEditTool>;
	let readTool: ReturnType<typeof createReadTool>;

	beforeEach(() => {
		testDir = join(tmpdir(), `coding-agent-untouched-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(testDir, { recursive: true });
		editTool = createEditTool(testDir);
		readTool = createReadTool(testDir);
	});

	afterEach(() => {
		rmSync(testDir, { recursive: true, force: true });
	});

	const textOf = (result: { content?: Array<{ type: string; text?: string }> }): string =>
		(result.content ?? [])
			.filter((c) => c.type === "text")
			.map((c) => c.text ?? "")
			.join("\n");

	it("leaves tabs and aligned columns intact outside the edited block", async () => {
		const testFile = join(testDir, "aligned.ts");
		const original = [
			"function fmt() {",
			"\tconst x = 1;",
			'\tconst pad = "col1    col2    col3";',
			"\tconst re = /a  b/;",
			"\treturn pad;",
			"}",
			"",
		].join("\n");
		writeFileSync(testFile, original);

		// Model emits 2-space indentation instead of tabs, so this reaches the
		// fuzzy tier. It targets only the first const.
		const result = await editTool.execute("edit-1", {
			path: testFile,
			edits: [{ oldText: "  const x = 1;", newText: "  const x = 2;" }],
		});
		expect(textOf(result)).toContain("Successfully replaced");

		const updated = readFileSync(testFile, "utf-8");
		const originalLines = original.split("\n");
		const updatedLines = updated.split("\n");

		expect(updatedLines).toHaveLength(originalLines.length);
		// Only the targeted line changed; every other line is byte-identical.
		for (let i = 0; i < originalLines.length; i++) {
			if (i === 1) continue;
			expect(updatedLines[i]).toBe(originalLines[i]);
		}
		// String literal and regex spacing survive untouched.
		expect(updated).toContain('"col1    col2    col3"');
		expect(updated).toContain("/a  b/");
		expect(updated).toContain("const x = 2;");
	});

	it("applies replaceAll to two fuzzy matches sharing one line", async () => {
		const testFile = join(testDir, "same-line.ts");
		// Both occurrences use smart quotes, so oldText only matches fuzzily, and
		// both land on the same line - which widens to the same span.
		writeFileSync(testFile, "const a = \u2018x\u2019; const b = \u2018x\u2019;\nconst c = 1;\n");

		const result = await editTool.execute("edit-same-line", {
			path: testFile,
			edits: [{ oldText: "'x'", newText: "'y'", replaceAll: true }],
		});
		expect(textOf(result)).toContain("Successfully replaced");

		const updated = readFileSync(testFile, "utf-8");
		expect(updated).toBe("const a = 'y'; const b = 'y';\nconst c = 1;\n");
	});

	it("still reports duplicates for two fuzzy matches sharing one line", async () => {
		const testFile = join(testDir, "same-line-dup.ts");
		writeFileSync(testFile, "const a = \u2018x\u2019; const b = \u2018x\u2019;\n");

		await expect(
			editTool.execute("edit-same-line-dup", {
				path: testFile,
				edits: [{ oldText: "'x'", newText: "'y'" }],
			}),
		).rejects.toThrow(/Found 2 occurrences/);
	});

	it("does not reindent a line for a fuzzy edit that asks for no change", async () => {
		const testFile = join(testDir, "noop.ts");
		const original = "function f() {\n\tconst x = 1;\n\treturn x;\n}\n";
		writeFileSync(testFile, original);

		// oldText only matches fuzzily (spaces vs tabs) and newText is identical to
		// it, so the model asked for nothing. The tab must survive.
		await expect(
			editTool.execute("edit-noop", {
				path: testFile,
				edits: [{ oldText: "  const x = 1;", newText: "  const x = 1;" }],
			}),
		).rejects.toThrow(/No changes made/);
		expect(readFileSync(testFile, "utf-8")).toBe(original);
	});

	it("still applies a deliberate whitespace-only edit matched exactly", async () => {
		const testFile = join(testDir, "reindent.ts");
		writeFileSync(testFile, "function f() {\n\tconst x = 1;\n}\n");

		// oldText matches the file byte-for-byte, so the new indentation is intended.
		const result = await editTool.execute("edit-reindent", {
			path: testFile,
			edits: [{ oldText: "\tconst x = 1;", newText: "    const x = 1;" }],
		});
		expect(textOf(result)).toContain("Successfully replaced");
		expect(readFileSync(testFile, "utf-8")).toBe("function f() {\n    const x = 1;\n}\n");
	});

	it("reports a diff that matches what was actually written", async () => {
		const testFile = join(testDir, "diagram.md");
		const original = ["# Title", "", "\tconst a = 1;", "", "  entry:  0     1     2", "", "done", ""].join("\n");
		writeFileSync(testFile, original);

		const result = await editTool.execute("edit-2", {
			path: testFile,
			edits: [{ oldText: "  const a = 1;", newText: "  const a = 9;" }],
		});

		const diff = (result as { details?: { diff?: string } }).details?.diff ?? "";
		const changedInDiff = diff.split("\n").filter((line) => line.startsWith("+") || line.startsWith("-")).length;

		const updated = readFileSync(testFile, "utf-8");
		const originalLines = original.split("\n");
		const updatedLines = updated.split("\n");
		let changedOnDisk = 0;
		for (let i = 0; i < Math.max(originalLines.length, updatedLines.length); i++) {
			if (originalLines[i] !== updatedLines[i]) changedOnDisk++;
		}

		// One removed line plus one added line for a single changed line.
		expect(changedInDiff).toBe(2);
		expect(changedOnDisk).toBe(1);
		// The ASCII column alignment the edit never referenced is still there.
		expect(updated).toContain("  entry:  0     1     2");
	});

	it("round-trips text the read tool handed back, without rewriting the file", async () => {
		const testFile = join(testDir, "doc.md");
		// A whitespace-only line inside the block the model will copy, plus an
		// aligned diagram elsewhere. Padded past the 1KB mark, which is where the
		// read tool used to start stripping trailing whitespace.
		const original = [
			"# Doc",
			"",
			"```ts",
			"pi.on('event', async () => {",
			"  const { preparation } = event;",
			"  ",
			"  return preparation;",
			"});",
			"```",
			"",
			"     ↑         ↑      └────────┬────────┘",
			"  prompt   from cmp      messages kept",
			"",
			...Array.from({ length: 40 }, (_, i) => `Filler line ${i} to push this document past one kilobyte.`),
			"",
		].join("\n");
		writeFileSync(testFile, original);

		// Read it exactly as the agent would.
		const readResult = await readTool.execute("read-1", { path: "doc.md" }, undefined, () => {});
		const shown = textOf(readResult as { content?: Array<{ type: string; text?: string }> });

		// The model copies four lines verbatim out of what it was shown.
		const shownLines = shown.split("\n");
		const start = shownLines.findIndex((line) => line.includes("pi.on('event'"));
		expect(start).toBeGreaterThanOrEqual(0);
		const oldText = shownLines.slice(start, start + 4).join("\n");
		const newText = oldText.replace("const { preparation } = event;", "const { preparation } = event; // noted");

		const result = await editTool.execute("edit-3", { path: "doc.md", edits: [{ oldText, newText }] });
		expect(textOf(result as { content?: Array<{ type: string; text?: string }> })).toContain("Successfully replaced");

		const updated = readFileSync(testFile, "utf-8");
		const originalLines = original.split("\n");
		const updatedLines = updated.split("\n");
		let changedOnDisk = 0;
		for (let i = 0; i < Math.max(originalLines.length, updatedLines.length); i++) {
			if (originalLines[i] !== updatedLines[i]) changedOnDisk++;
		}

		expect(changedOnDisk).toBe(1);
		// The diagram far from the edit keeps its alignment.
		expect(updated).toContain("     ↑         ↑      └────────┬────────┘");
		expect(updated).toContain("  prompt   from cmp      messages kept");
	});
});
