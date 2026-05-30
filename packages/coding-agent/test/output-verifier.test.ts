import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { CONFIG_DIR_NAME } from "../src/config.js";
import { OutputVerifier } from "../src/core/output-verifier.js";

function createResultJson(task_id: string, cwd: string, content: Record<string, unknown>): string {
	const dir = join(cwd, CONFIG_DIR_NAME, "agents", task_id);
	mkdirSync(dir, { recursive: true });
	const path = join(dir, "result.json");
	writeFileSync(path, JSON.stringify(content, null, 2));
	return path;
}

describe("OutputVerifier", () => {
	describe("verify", () => {
		it("returns valid for a correct result.json", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("task-1", testCwd, {
				summary: "All files updated successfully",
				files_changed: ["src/foo.ts", "src/bar.ts"],
				confidence: 0.95,
				status: "complete",
			});

			const result = verifier.verify("task-1");
			expect(result.valid).toBe(true);
			expect(result.reason).toBeUndefined();

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when result.json does not exist", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);

			const result = verifier.verify("missing-task");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("not found");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when result.json contains invalid JSON", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			const dir = join(testCwd, CONFIG_DIR_NAME, "agents", "bad-json");
			mkdirSync(dir, { recursive: true });
			writeFileSync(join(dir, "result.json"), "not json");

			const result = verifier.verify("bad-json");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("Invalid JSON");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when result.json is not an object", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("not-obj", testCwd, "string" as unknown as Record<string, unknown>);

			const result = verifier.verify("not-obj");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("not an object");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when summary is missing", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("no-summary", testCwd, {
				files_changed: [],
				confidence: 0.9,
				status: "complete",
			});

			const result = verifier.verify("no-summary");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("summary");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when summary is empty", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("empty-summary", testCwd, {
				summary: "   ",
				files_changed: [],
				confidence: 0.9,
				status: "complete",
			});

			const result = verifier.verify("empty-summary");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("Empty");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when files_changed is missing", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("no-files", testCwd, {
				summary: "Done",
				confidence: 0.9,
				status: "complete",
			});

			const result = verifier.verify("no-files");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("files_changed");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when files_changed contains non-strings", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("bad-files", testCwd, {
				summary: "Done",
				files_changed: ["ok", 123, true],
				confidence: 0.9,
				status: "complete",
			});

			const result = verifier.verify("bad-files");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("Non-string");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when confidence is missing", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("no-confidence", testCwd, {
				summary: "Done",
				files_changed: [],
				status: "complete",
			});

			const result = verifier.verify("no-confidence");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("confidence");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when confidence is below 0.5", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("low-confidence", testCwd, {
				summary: "Done",
				files_changed: [],
				confidence: 0.3,
				status: "complete",
			});

			const result = verifier.verify("low-confidence");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("0.3");
			expect(result.reason).toContain("0.5");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("passes when confidence is exactly 0.5", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("exact-confidence", testCwd, {
				summary: "Done",
				files_changed: [],
				confidence: 0.5,
				status: "complete",
			});

			const result = verifier.verify("exact-confidence");
			expect(result.valid).toBe(true);

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when status is missing", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("no-status", testCwd, {
				summary: "Done",
				files_changed: [],
				confidence: 0.9,
			});

			const result = verifier.verify("no-status");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("status");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("fails when status is invalid", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("bad-status", testCwd, {
				summary: "Done",
				files_changed: [],
				confidence: 0.9,
				status: "done",
			});

			const result = verifier.verify("bad-status");
			expect(result.valid).toBe(false);
			expect(result.reason).toContain("Invalid status");

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("passes for status 'partial'", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("partial", testCwd, {
				summary: "Some changes applied",
				files_changed: ["a.ts"],
				confidence: 0.75,
				status: "partial",
			});

			const result = verifier.verify("partial");
			expect(result.valid).toBe(true);

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("passes for status 'failed'", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier(testCwd);
			createResultJson("failed", testCwd, {
				summary: "Could not apply changes",
				files_changed: [],
				confidence: 0.6,
				status: "failed",
			});

			const result = verifier.verify("failed");
			expect(result.valid).toBe(true);

			rmSync(testCwd, { recursive: true, force: true });
		});

		it("accepts a cwd override", () => {
			const testCwd = mkdtempSync(join(tmpdir(), "hoocode-test-"));
			const verifier = new OutputVerifier("/nonexistent");
			createResultJson("override", testCwd, {
				summary: "Done",
				files_changed: [],
				confidence: 0.9,
				status: "complete",
			});

			const result = verifier.verify("override", testCwd);
			expect(result.valid).toBe(true);

			rmSync(testCwd, { recursive: true, force: true });
		});
	});
});
