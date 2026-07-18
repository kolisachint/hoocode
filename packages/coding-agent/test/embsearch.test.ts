import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { chunkFile } from "../src/core/embsearch/chunker.js";
import { hashContent, loadIndexMeta, saveIndexMeta } from "../src/core/embsearch/index-meta.js";
import { scanRepo } from "../src/core/embsearch/repo-scan.js";

describe("chunkFile", () => {
	it("returns line ranges and ids for dense files", () => {
		const lines = Array.from({ length: 75 }, (_, i) => `line ${i}`);
		const chunks = chunkFile("src/foo.ts", lines.join("\n"));
		expect(chunks.length).toBeGreaterThan(1);
		expect(chunks[0].id).toBe("src/foo.ts#0");
		expect(chunks[0].startLine).toBe(1);
		expect(chunks[chunks.length - 1].endLine).toBe(75);
	});

	it("skips binary-looking content", () => {
		const chunks = chunkFile("pkg/binary.bin", "hello\u0000world");
		expect(chunks).toEqual([]);
	});

	it("respects character cap", () => {
		const text = "x".repeat(2000);
		const chunks = chunkFile("src/long.ts", text);
		// Should have been split before the line count limit kicked in.
		expect(chunks.length).toBeGreaterThan(0);
		for (const c of chunks) {
			expect(c.text.length).toBeLessThanOrEqual(1000);
		}
	});
});

describe("repo-scan", () => {
	it("counts only tracked-looking source files and skips node_modules", () => {
		const root = mkdtempSync(join(tmpdir(), "embsearch-scan-"));
		try {
			writeFileSync(join(root, "a.ts"), "export const a = 1;\n");
			writeFileSync(join(root, "b.ts"), "export const b = 2;\n");
			mkdirSync(join(root, "node_modules"), { recursive: true });
			writeFileSync(join(root, "node_modules", "big.ts"), "x".repeat(5000));
			mkdirSync(join(root, "dist"), { recursive: true });
			writeFileSync(join(root, "dist", "out.js"), "x".repeat(5000));

			const result = scanRepo(root);
			expect(result.files.map((f) => f.rel).sort()).toEqual(["a.ts", "b.ts", "dist/out.js"]);
			expect(result.totalBytes).toBeGreaterThan(0);
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});

	it("respects hierarchical .gitignore", () => {
		const root = mkdtempSync(join(tmpdir(), "embsearch-ignore-"));
		try {
			writeFileSync(join(root, ".gitignore"), "ignored/\n");
			mkdirSync(join(root, "src"), { recursive: true });
			mkdirSync(join(root, "ignored"), { recursive: true });
			writeFileSync(join(root, "src", "kept.ts"), "export const k = 1;");
			writeFileSync(join(root, "ignored", "skipped.ts"), "export const s = 1;");

			const result = scanRepo(root);
			expect(result.files.map((f) => f.rel)).toEqual(["src/kept.ts"]);
		} finally {
			rmSync(root, { recursive: true, force: true });
		}
	});
});

describe("index-meta", () => {
	it("round-trips and invalidates on model id mismatch", () => {
		const dir = mkdtempSync(join(tmpdir(), "embsearch-meta-"));
		try {
			const meta = {
				formatVersion: 1,
				chunkerVersion: 1,
				modelId: "all-MiniLM-L6-v2-int8",
				repoRoot: "/repo",
				lastUsedMs: Date.now(),
				files: {
					"src/a.ts": {
						mtimeMs: 1000,
						size: 12,
						hash: hashContent("export const a"),
						chunks: [[1, 1]] as [number, number][],
					},
				},
			};
			saveIndexMeta(dir, meta);
			const loaded = loadIndexMeta(dir, "all-MiniLM-L6-v2-int8")!;
			expect(loaded.modelId).toBe("all-MiniLM-L6-v2-int8");
			expect(loaded.files["src/a.ts"].chunks).toEqual([[1, 1]]);

			// Wrong model id returns undefined -> rebuild.
			expect(loadIndexMeta(dir, "mock-hash-v1")).toBeUndefined();
		} finally {
			rmSync(dir, { recursive: true, force: true });
		}
	});
});
