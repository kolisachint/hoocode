import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createFindToolDefinition } from "../src/core/tools/find.js";
import { createGrepToolDefinition } from "../src/core/tools/grep.js";

/**
 * Exercises the pure-JS fallbacks for `find` and `grep` that engage when the
 * fd/rg binaries are unavailable. Forced on via HOOCODE_NATIVE_SEARCH so the
 * tests run identically regardless of whether fd/rg are installed in CI.
 */
describe("native search fallback (HOOCODE_NATIVE_SEARCH)", () => {
	let root: string;
	let prevEnv: string | undefined;

	beforeEach(() => {
		prevEnv = process.env.HOOCODE_NATIVE_SEARCH;
		process.env.HOOCODE_NATIVE_SEARCH = "1";
		root = mkdtempSync(join(tmpdir(), "hoo-native-search-"));
	});

	afterEach(() => {
		if (prevEnv === undefined) delete process.env.HOOCODE_NATIVE_SEARCH;
		else process.env.HOOCODE_NATIVE_SEARCH = prevEnv;
		if (root) rmSync(root, { recursive: true, force: true });
	});

	const textOf = (result: { content: Array<{ text?: string }> }): string => result.content[0]?.text ?? "";

	async function runFind(
		args: Parameters<ReturnType<typeof createFindToolDefinition>["execute"]>[1],
	): Promise<string[]> {
		const def = createFindToolDefinition(root);
		const ctx = {} as Parameters<typeof def.execute>[4];
		const result = (await def.execute("call", args, undefined, undefined, ctx)) as {
			content: Array<{ text?: string }>;
		};
		const text = textOf(result);
		if (text === "No files found matching pattern") return [];
		return text
			.split("\n")
			.map((l) => l.trim())
			.filter((l) => l.length > 0 && !l.startsWith("["))
			.sort();
	}

	async function runGrep(
		args: Parameters<ReturnType<typeof createGrepToolDefinition>["execute"]>[1],
	): Promise<string> {
		const def = createGrepToolDefinition(root);
		const ctx = {} as Parameters<typeof def.execute>[4];
		const result = (await def.execute("call", args, undefined, undefined, ctx)) as {
			content: Array<{ text?: string }>;
		};
		return textOf(result);
	}

	describe("find", () => {
		it("includes hidden files that are not gitignored", async () => {
			mkdirSync(join(root, ".secret"));
			writeFileSync(join(root, ".secret", "hidden.txt"), "x");
			writeFileSync(join(root, "visible.txt"), "x");

			const files = await runFind({ pattern: "**/*.txt" });
			expect(files).toContain("visible.txt");
			expect(files).toContain(".secret/hidden.txt");
		});

		it("respects a .gitignore in the search root", async () => {
			writeFileSync(join(root, ".gitignore"), "ignored.txt\n");
			writeFileSync(join(root, "ignored.txt"), "x");
			writeFileSync(join(root, "kept.txt"), "x");

			const files = await runFind({ pattern: "**/*.txt" });
			expect(files).toContain("kept.txt");
			expect(files).not.toContain("ignored.txt");
		});

		it("scopes nested .gitignore files to their own subtree (issue #3303)", async () => {
			mkdirSync(join(root, "a", "deep"), { recursive: true });
			mkdirSync(join(root, "b"), { recursive: true });
			writeFileSync(join(root, "a", ".gitignore"), "ignored.txt\n");
			writeFileSync(join(root, "a", "deep", ".gitignore"), "secret.txt\n");
			writeFileSync(join(root, "a", "ignored.txt"), "");
			writeFileSync(join(root, "a", "kept.txt"), "");
			writeFileSync(join(root, "a", "deep", "ignored.txt"), "");
			writeFileSync(join(root, "a", "deep", "secret.txt"), "");
			writeFileSync(join(root, "a", "deep", "kept.txt"), "");
			writeFileSync(join(root, "b", "ignored.txt"), "");
			writeFileSync(join(root, "b", "kept.txt"), "");
			writeFileSync(join(root, "root.txt"), "");

			const files = await runFind({ pattern: "**/*.txt" });
			expect(files).toEqual(["a/deep/kept.txt", "a/kept.txt", "b/ignored.txt", "b/kept.txt", "root.txt"]);
		});

		it("always excludes node_modules and .git", async () => {
			mkdirSync(join(root, "node_modules"));
			mkdirSync(join(root, ".git"));
			writeFileSync(join(root, "node_modules", "dep.txt"), "x");
			writeFileSync(join(root, ".git", "config.txt"), "x");
			writeFileSync(join(root, "app.txt"), "x");

			const files = await runFind({ pattern: "**/*.txt" });
			expect(files).toEqual(["app.txt"]);
		});

		it("matches a slashless pattern against the basename at any depth", async () => {
			mkdirSync(join(root, "src", "nested"), { recursive: true });
			writeFileSync(join(root, "src", "a.ts"), "");
			writeFileSync(join(root, "src", "nested", "b.ts"), "");
			writeFileSync(join(root, "src", "c.js"), "");

			const files = await runFind({ pattern: "*.ts" });
			expect(files).toEqual(["src/a.ts", "src/nested/b.ts"]);
		});

		it("supports multiple patterns (OR) and extra exclusions", async () => {
			writeFileSync(join(root, "a.ts"), "");
			writeFileSync(join(root, "a.test.ts"), "");
			writeFileSync(join(root, "b.js"), "");

			const files = await runFind({ pattern: ["*.ts", "*.js"], exclude: "**/*.test.ts" });
			expect(files).toEqual(["a.ts", "b.js"]);
		});

		it("honours the type filter and directory depth", async () => {
			mkdirSync(join(root, "dir1", "dir2"), { recursive: true });
			writeFileSync(join(root, "top.txt"), "");
			writeFileSync(join(root, "dir1", "mid.txt"), "");

			const dirs = await runFind({ pattern: "**", type: "d" });
			expect(dirs).toContain("dir1/");
			expect(dirs).toContain("dir1/dir2/");

			const shallow = await runFind({ pattern: "**/*.txt", depth: 1 });
			expect(shallow).toEqual(["top.txt"]);
		});
	});

	describe("grep", () => {
		beforeEach(() => {
			writeFileSync(join(root, "one.ts"), "const needle = 1;\nconst other = 2;\n");
			writeFileSync(join(root, "two.js"), "// NEEDLE here\nfoo();\n");
			writeFileSync(join(root, "three.txt"), "no match here\n");
		});

		it("finds matches with file paths and line numbers", async () => {
			const out = await runGrep({ pattern: "needle" });
			expect(out).toContain("one.ts");
			expect(out).toContain("1: const needle = 1;");
			expect(out).not.toContain("three.txt");
		});

		it("supports case-insensitive search", async () => {
			const sensitive = await runGrep({ pattern: "needle" });
			expect(sensitive).not.toContain("two.js");
			const insensitive = await runGrep({ pattern: "needle", ignoreCase: true });
			expect(insensitive).toContain("two.js");
		});

		it("treats the pattern literally when literal is set", async () => {
			writeFileSync(join(root, "regexy.txt"), "a.b\naxb\n");
			const literal = await runGrep({ pattern: "a.b", literal: true });
			expect(literal).toContain("1: a.b");
			expect(literal).not.toContain("2: axb");
		});

		it("filters files by glob", async () => {
			const out = await runGrep({ pattern: "needle", glob: "*.ts", ignoreCase: true });
			expect(out).toContain("one.ts");
			expect(out).not.toContain("two.js");
		});

		it("includes context lines", async () => {
			const out = await runGrep({ pattern: "needle", context: 1 });
			expect(out).toContain("1: const needle = 1;");
			expect(out).toContain("2- const other = 2;");
		});

		it("respects the match limit and reports it", async () => {
			writeFileSync(join(root, "many.txt"), "hit\nhit\nhit\nhit\n");
			const out = await runGrep({ pattern: "hit", limit: 2 });
			const hitLines = out.split("\n").filter((l) => l.includes(": hit"));
			expect(hitLines.length).toBe(2);
			expect(out).toContain("matches limit reached");
		});

		it("searches a single file when the path is a file", async () => {
			const out = await runGrep({ pattern: "needle", path: join(root, "one.ts") });
			expect(out).toContain("one.ts");
			expect(out).toContain("1: const needle = 1;");
		});

		it("respects .gitignore", async () => {
			writeFileSync(join(root, ".gitignore"), "ignored.ts\n");
			writeFileSync(join(root, "ignored.ts"), "const needle = 9;\n");
			const out = await runGrep({ pattern: "needle" });
			expect(out).not.toContain("ignored.ts");
		});

		it("reports 'No matches found' when nothing matches", async () => {
			const out = await runGrep({ pattern: "zzz-nonexistent-zzz" });
			expect(out).toBe("No matches found");
		});

		it("surfaces an invalid-regex hint pointing at literal", async () => {
			await expect(runGrep({ pattern: "(" })).rejects.toThrow(/literal: true/);
		});

		it("skips binary files", async () => {
			writeFileSync(join(root, "bin.dat"), Buffer.from([0x6e, 0x00, 0x65, 0x65, 0x64, 0x6c, 0x65]));
			const out = await runGrep({ pattern: "eedle" });
			expect(out).not.toContain("bin.dat");
		});
	});
});
