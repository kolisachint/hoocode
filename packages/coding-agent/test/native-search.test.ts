import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { collectEntries, type NativeGrepOptions, nativeGrep } from "../src/core/tools/native-search.js";

/**
 * Exercises the pure-JS content-search fallback that engages when the rg binary
 * is unavailable. It backs the lexical half of `search`, so it has to behave
 * identically whether or not rg is installed in CI.
 */
describe("native search fallback", () => {
	let root: string;

	beforeEach(() => {
		root = mkdtempSync(join(tmpdir(), "hoo-native-search-"));
	});

	afterEach(() => {
		if (root) rmSync(root, { recursive: true, force: true });
	});

	async function runGrep(opts: Partial<NativeGrepOptions> & { pattern: string }, searchRoot = root) {
		const result = await nativeGrep(searchRoot, {
			isDirectory: searchRoot === root,
			limit: 100,
			readFile: (p) => readFileSync(p, "utf-8"),
			...opts,
		});
		return {
			...result,
			text: result.matches.map((m) => `${m.filePath}:${m.lineNumber}: ${m.lineText}`).join("\n"),
		};
	}

	describe("collectEntries", () => {
		it("includes hidden files that are not gitignored", () => {
			writeFileSync(join(root, ".hidden.ts"), "x");
			const rels = collectEntries(root).map((e) => e.rel);
			expect(rels).toContain(".hidden.ts");
		});
	});

	describe("nativeGrep", () => {
		beforeEach(() => {
			writeFileSync(join(root, "one.ts"), "const needle = 1;\nconst other = 2;\n");
			writeFileSync(join(root, "two.js"), "// NEEDLE here\nfoo();\n");
			writeFileSync(join(root, "three.txt"), "no match here\n");
		});

		it("finds matches with file paths and line numbers", async () => {
			const { text } = await runGrep({ pattern: "needle" });
			expect(text).toContain("one.ts");
			expect(text).toContain("1: const needle = 1;");
			expect(text).not.toContain("three.txt");
		});

		it("supports case-insensitive search", async () => {
			const sensitive = await runGrep({ pattern: "needle" });
			expect(sensitive.text).not.toContain("two.js");
			const insensitive = await runGrep({ pattern: "needle", ignoreCase: true });
			expect(insensitive.text).toContain("two.js");
		});

		it("treats the pattern literally when literal is set", async () => {
			writeFileSync(join(root, "regexy.txt"), "a.b\naxb\n");
			const { text } = await runGrep({ pattern: "a.b", literal: true });
			expect(text).toContain("1: a.b");
			expect(text).not.toContain("2: axb");
		});

		it("filters files by glob", async () => {
			const { text } = await runGrep({ pattern: "needle", glob: "*.ts", ignoreCase: true });
			expect(text).toContain("one.ts");
			expect(text).not.toContain("two.js");
		});

		it("respects the match limit and reports it", async () => {
			writeFileSync(join(root, "many.txt"), "hit\nhit\nhit\nhit\n");
			const result = await runGrep({ pattern: "hit", limit: 2 });
			expect(result.matches.length).toBe(2);
			expect(result.matchLimitReached).toBe(true);
		});

		it("searches a single file when the root is a file", async () => {
			const { text } = await runGrep({ pattern: "needle", isDirectory: false }, join(root, "one.ts"));
			expect(text).toContain("one.ts");
			expect(text).toContain("1: const needle = 1;");
		});

		it("respects .gitignore", async () => {
			writeFileSync(join(root, ".gitignore"), "ignored.ts\n");
			writeFileSync(join(root, "ignored.ts"), "const needle = 9;\n");
			const { text } = await runGrep({ pattern: "needle" });
			expect(text).not.toContain("ignored.ts");
		});

		it("returns no matches when nothing matches", async () => {
			const result = await runGrep({ pattern: "zzz-nonexistent-zzz" });
			expect(result.matches).toEqual([]);
		});

		it("tags an invalid regex so callers can suggest literal mode", async () => {
			await expect(runGrep({ pattern: "(" })).rejects.toMatchObject({ invalidRegex: true });
		});

		it("skips binary files", async () => {
			writeFileSync(join(root, "bin.dat"), Buffer.from([0x6e, 0x00, 0x65, 0x65, 0x64, 0x6c, 0x65]));
			const { text } = await runGrep({ pattern: "eedle" });
			expect(text).not.toContain("bin.dat");
		});
	});
});
