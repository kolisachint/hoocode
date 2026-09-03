import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { collectEntries } from "../../../src/core/tools/native-search.js";

/**
 * Regression test for https://github.com/kolisachint/hoocode/issues/3303
 *
 * File discovery previously collected every `.gitignore` under the search path
 * and passed them to `fd` via `--ignore-file`. fd treats `--ignore-file`
 * entries as a single global ignore source, so rules from `a/.gitignore` also
 * filtered files under sibling `b/`. The fix was hierarchical `.gitignore`
 * handling, scoping each file to its own subtree.
 *
 * The `find` tool that first hit this is gone; `collectEntries` — which now
 * walks the tree for `search`'s lexical retriever and the embsearch repo scan —
 * inherits the guarantee, so the regression is pinned here.
 */
describe("issue #3303 nested .gitignore rules leak into sibling directories", () => {
	let tempRoot: string;

	function walk(): string[] {
		return collectEntries(tempRoot)
			.filter((entry) => entry.type === "f" && entry.rel.endsWith(".txt"))
			.map((entry) => entry.rel)
			.sort();
	}

	afterEach(() => {
		if (tempRoot) rmSync(tempRoot, { recursive: true, force: true });
	});

	describe("flat sibling case", () => {
		beforeEach(() => {
			tempRoot = mkdtempSync(join(tmpdir(), "pi-3303-flat-"));
			mkdirSync(join(tempRoot, "a"), { recursive: true });
			mkdirSync(join(tempRoot, "b"), { recursive: true });
			writeFileSync(join(tempRoot, "a", ".gitignore"), "ignored.txt\n");
			writeFileSync(join(tempRoot, "a", "ignored.txt"), "");
			writeFileSync(join(tempRoot, "a", "kept.txt"), "");
			writeFileSync(join(tempRoot, "b", "ignored.txt"), "");
			writeFileSync(join(tempRoot, "b", "kept.txt"), "");
			writeFileSync(join(tempRoot, "root.txt"), "");
		});

		it("applies a/.gitignore only inside a/ and leaves b/ untouched", () => {
			expect(walk()).toEqual(["a/kept.txt", "b/ignored.txt", "b/kept.txt", "root.txt"]);
		});
	});

	describe("deeply nested case", () => {
		beforeEach(() => {
			tempRoot = mkdtempSync(join(tmpdir(), "pi-3303-deep-"));
			mkdirSync(join(tempRoot, "a", "deep"), { recursive: true });
			mkdirSync(join(tempRoot, "b"), { recursive: true });
			writeFileSync(join(tempRoot, "a", ".gitignore"), "ignored.txt\n");
			writeFileSync(join(tempRoot, "a", "deep", ".gitignore"), "secret.txt\n");
			writeFileSync(join(tempRoot, "a", "ignored.txt"), "");
			writeFileSync(join(tempRoot, "a", "kept.txt"), "");
			writeFileSync(join(tempRoot, "a", "deep", "ignored.txt"), "");
			writeFileSync(join(tempRoot, "a", "deep", "secret.txt"), "");
			writeFileSync(join(tempRoot, "a", "deep", "kept.txt"), "");
			writeFileSync(join(tempRoot, "b", "ignored.txt"), "");
			writeFileSync(join(tempRoot, "b", "kept.txt"), "");
			writeFileSync(join(tempRoot, "root.txt"), "");
		});

		it("scopes each .gitignore to its own subtree", () => {
			// a/.gitignore ignores 'ignored.txt' within a/ and a/deep/.
			// a/deep/.gitignore additionally ignores 'secret.txt' within a/deep/.
			// b/ is untouched by either.
			expect(walk()).toEqual(["a/deep/kept.txt", "a/kept.txt", "b/ignored.txt", "b/kept.txt", "root.txt"]);
		});
	});
});
