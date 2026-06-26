import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createDocEditTool } from "../src/core/tools/docedit.js";
import { createDocReadTool, renderEnvelopeText } from "../src/core/tools/docread.js";
import { createDocWriteTool } from "../src/core/tools/docwrite.js";
import type { Envelope } from "../src/core/tools/filetools-shared.js";
import {
	DOCREAD_MAX_RENDER_TOKENS,
	estimateTextTokens,
	invalidateExtractRecord,
	truncateRenderToTokenBudget,
} from "../src/core/tools/filetools-shared.js";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function getText(result: { content: Array<{ type: string; text?: string }> }): string {
	return result.content
		.filter((c) => c.type === "text")
		.map((c) => c.text)
		.join("\n");
}

interface DocReadDetails {
	type?: string;
	fidelity?: string;
	writable?: boolean;
	nodeCount?: number;
}

interface DocEditDetails {
	ops?: number;
}

interface DocWriteDetails {
	ops?: number;
	out?: string;
}

// Deterministic ids from real filetools for the content "<doc><title>Hello</title></doc>".
const ROOT_ID = "el_d3626bb4";
const TITLE_ID = "el_fefd54ad";
// sha256 of the <doc> element's byte range in the same content.
const ROOT_HASH = "sha256:a745a88a9f2aa4cd8112eee1e206bf1cc92638395486f78a48164055819bfc28";

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

describe("document tools", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = join(tmpdir(), `filetools-cwd-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
	});

	afterEach(() => {
		rmSync(cwd, { recursive: true, force: true });
	});

	// =====================================================================
	// Unit tests for token estimation
	// =====================================================================

	describe("estimateTextTokens", () => {
		it("returns 0 for empty string", () => {
			expect(estimateTextTokens("")).toBe(0);
		});

		it("returns 1 for four characters", () => {
			expect(estimateTextTokens("abcd")).toBe(1);
		});

		it("rounds up", () => {
			expect(estimateTextTokens("a")).toBe(1);
			expect(estimateTextTokens("abc")).toBe(1);
			expect(estimateTextTokens("abcde")).toBe(2);
		});

		it("counts newlines as one character each", () => {
			expect(estimateTextTokens("\n".repeat(8))).toBe(2);
		});
	});

	// =====================================================================
	// Unit tests for truncation helper
	// =====================================================================

	describe("truncateRenderToTokenBudget", () => {
		it("returns full text and droppedLines=0 when under budget", () => {
			const lines = ["hello", "world"];
			const { text, droppedLines } = truncateRenderToTokenBudget(lines, 100);
			expect(text).toBe("hello\nworld");
			expect(droppedLines).toBe(0);
		});

		it("truncates to token budget keeping whole lines", () => {
			const lines = Array.from({ length: 20 }, (_, i) => `line${String(i).padStart(2, "0")}`);
			const { text, droppedLines } = truncateRenderToTokenBudget(lines, 5);
			expect(droppedLines).toBeGreaterThan(0);
			const firstLineLen = "line00".length + 1;
			const maxLinesBeforeBudget = Math.floor((5 * 4) / firstLineLen);
			const renderedLines = text.split("\n");
			expect(renderedLines.length).toBeLessThanOrEqual(maxLinesBeforeBudget + 1);
			expect(droppedLines).toBe(lines.length - renderedLines.length);
		});

		it("returns droppedLines=0 when text exactly equals budget", () => {
			const lines = ["a"];
			const { text, droppedLines } = truncateRenderToTokenBudget(lines, 1);
			expect(text).toBe("a");
			expect(droppedLines).toBe(0);
		});

		it("uses default budget of DOCREAD_MAX_RENDER_TOKENS", () => {
			const lines = Array.from({ length: 10000 }, () => "x".repeat(80));
			const { text, droppedLines } = truncateRenderToTokenBudget(lines);
			expect(droppedLines).toBeGreaterThan(0);
			const renderedTokens = estimateTextTokens(text);
			expect(renderedTokens).toBeLessThanOrEqual(DOCREAD_MAX_RENDER_TOKENS);
		});
	});

	// =====================================================================
	// Unit tests for renderEnvelopeText (pure rendering logic)
	// =====================================================================

	describe("renderEnvelopeText", () => {
		it("renders a normal writable envelope with ids", () => {
			const envelope: Envelope = {
				version: "1.0",
				source: { path: "/tmp/doc.xml", type: "xml", hash: "sha256:abc" },
				fidelity: "lossless",
				writable: true,
				structure: [
					{
						id: "n1",
						tag: "doc",
						children: [
							{ id: "n2", tag: "title", text: "Hello" },
							{ id: "n3", tag: "body", attrs: [{ name: "class", value: "main" }], text: "World" },
						],
					},
				],
			};
			const text = renderEnvelopeText(envelope, false);
			expect(text).toContain("writable");
			expect(text).toContain("#n1 <doc>");
			expect(text).toContain("#n2 <title>");
			expect(text).toContain('Hello"');
			expect(text).toContain('#n3 <body class="main">');
		});

		it("renders a readonly envelope without ids", () => {
			const envelope: Envelope = {
				version: "1.0",
				source: { path: "/tmp/doc.xml", type: "xml", hash: "sha256:abc" },
				fidelity: "read_only",
				writable: false,
				structure: [{ id: "", tag: "note", text: "locked" }],
			};
			const text = renderEnvelopeText(envelope, true);
			expect(text).toContain("read-only");
			expect(text).toContain("<note>");
			expect(text).not.toContain("#"); // no ids
			expect(text).not.toContain("writable"); // only "read-only" appears
		});

		it("renders nothing special when under token budget (no truncation)", () => {
			const envelope: Envelope = {
				version: "1.0",
				source: { path: "/tmp/small.xml", type: "xml", hash: "sha256:abc" },
				fidelity: "lossless",
				writable: true,
				structure: [{ id: "x1", tag: "a", text: "tiny" }],
			};
			const text = renderEnvelopeText(envelope, false);
			expect(text).not.toContain("[Truncated:");
			expect(text).toContain("#x1 <a>");
		});

		it("truncates a large envelope and appends notice", () => {
			// Build enough nodes to exceed the budget.
			const nodes: Array<{ id: string; tag: string; text: string }> = [];
			for (let i = 0; i < 5000; i++) {
				nodes.push({ id: `n${i}`, tag: "item", text: `v${i}` });
			}
			const envelope: Envelope = {
				version: "1.0",
				source: { path: "/tmp/big.xml", type: "xml", hash: "sha256:big" },
				fidelity: "lossless",
				writable: true,
				structure: nodes,
			};
			const text = renderEnvelopeText(envelope, false);
			expect(text).toContain("[Truncated:");
			expect(text).toContain("more node lines omitted");
			expect(text).toContain("re-run with readonly:true");
			// Early nodes present.
			expect(text).toContain("#n0 <item>");
			expect(text).toContain("#n1 <item>");
			// Late nodes omitted.
			expect(text).not.toContain("#n4999 <item>");
		});

		it("truncation notice gives smaller-file hint in readonly mode", () => {
			const nodes: Array<{ id: string; tag: string; text: string }> = [];
			for (let i = 0; i < 5000; i++) {
				nodes.push({ id: "", tag: "item", text: `v${i}` });
			}
			const envelope: Envelope = {
				version: "1.0",
				source: { path: "/tmp/big.xml", type: "xml", hash: "sha256:big" },
				fidelity: "read_only",
				writable: false,
				structure: nodes,
			};
			const text = renderEnvelopeText(envelope, true);
			expect(text).toContain("[Truncated:");
			expect(text).toContain("narrow to a smaller or more targeted file");
			expect(text).not.toContain("re-run with readonly:true");
		});

		it("truncation notice still includes hint for non-readonly mode even when envelope is read-only", () => {
			// readonly=false passed but envelope.writable could still be false
			// (e.g. a PDF). The hint should reflect the call-site intent, not
			// the envelope property.
			const nodes: Array<{ id: string; tag: string; text: string }> = [];
			for (let i = 0; i < 5000; i++) {
				nodes.push({ id: "x", tag: "item", text: `v${i}` });
			}
			const envelope: Envelope = {
				version: "1.0",
				source: { path: "/tmp/big.xml", type: "xml", hash: "sha256:big" },
				fidelity: "read_only",
				writable: false,
				structure: nodes,
			};
			const text = renderEnvelopeText(envelope, false);
			expect(text).toContain("[Truncated:");
			// readonly=false call-site decision -> non-readonly hint
			expect(text).toContain("re-run with readonly:true");
		});
	});

	// =====================================================================
	// DocRead integration tests (uses real filetools binary)
	// =====================================================================

	describe("DocRead", () => {
		it("extracts a document into id-addressed text", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			const tool = createDocReadTool(cwd);
			const result = await tool.execute("r1", { path: docPath });
			const text = getText(result);
			expect(text).toContain("#el_d3626bb4 <doc>");
			expect(text).toContain("#el_fefd54ad <title>");
			expect(text).toContain('"Hello"');
			expect(text).toContain("writable");
			const details = result.details as DocReadDetails;
			expect(details.type).toBe("xml");
			expect(details.writable).toBe(true);
			expect(details.nodeCount).toBeGreaterThan(0);
		});

		it("readonly:true passes --readonly flag and returns writable=false output", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			const tool = createDocReadTool(cwd);
			const result = await tool.execute("r2", { path: docPath, readonly: true });
			const text = getText(result);

			// readonly mode: writable=false, no ids in the rendered output.
			expect(text).toContain("read-only");
			expect(text).toContain("<doc>");
			expect(text).toContain("<title>");
			// No hash-based ids in readonly output.
			expect(text).not.toContain("#el_");

			const details = result.details as DocReadDetails;
			expect(details.writable).toBe(false);
		});

		it("truncates output for large documents", async () => {
			// Build a large XML file with many nested nodes.
			let xml = "<root>";
			for (let i = 0; i < 5000; i++) {
				xml += `<item id="${i}">value-${i}</item>`;
			}
			xml += "</root>";

			const docPath = join(cwd, "huge.xml");
			writeFileSync(docPath, xml);

			const tool = createDocReadTool(cwd);
			const result = await tool.execute("r3", { path: docPath });
			const text = getText(result);

			expect(text).toContain("[Truncated:");
			expect(text).toContain("more node lines omitted");
			// Early nodes present.
			expect(text).toContain("<root>");
			// Not all nodes rendered (the last nodes won't appear).
			expect(text).not.toContain('id="4999"');
		});

		it("truncation notice includes readonly hint when readonly:false", async () => {
			let xml = "<root>";
			for (let i = 0; i < 5000; i++) {
				xml += `<item id="${i}">v${i}</item>`;
			}
			xml += "</root>";

			const docPath = join(cwd, "large.xml");
			writeFileSync(docPath, xml);

			const tool = createDocReadTool(cwd);
			const result = await tool.execute("r4", { path: docPath, readonly: false });
			const text = getText(result);

			expect(text).toContain("re-run with readonly:true");
		});
	});

	// =====================================================================
	// DocEdit integration tests (uses real filetools binary)
	// =====================================================================

	describe("DocEdit", () => {
		it("requires a prior DocRead", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			invalidateExtractRecord(docPath);
			const tool = createDocEditTool(cwd);
			await expect(
				tool.execute("e1", { path: docPath, patch: [{ op: "replace", path: "/structure/n1/text", value: "Hi" }] }),
			).rejects.toThrow(/DocRead/);
		});

		it("applies a patch in place after a DocRead", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			await createDocReadTool(cwd).execute("r2", { path: docPath });
			const result = await createDocEditTool(cwd).execute("e2", {
				path: docPath,
				patch: [{ op: "replace", path: `/structure/${TITLE_ID}/text`, value: "Patched" }],
			});
			expect((result.details as DocEditDetails).ops).toBe(1);
			const content = readFileSync(docPath, "utf8");
			expect(content).toContain("Patched");
			expect(content).not.toContain("<title>Hello");
		});

		it("allows guard (test) ops before mutating ops", async () => {
			const docPath = join(cwd, "guarded.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			await createDocReadTool(cwd).execute("r3", { path: docPath });
			const result = await createDocEditTool(cwd).execute("e3", {
				path: docPath,
				patch: [
					{ op: "test", path: `/structure/${ROOT_ID}`, hash: ROOT_HASH },
					{ op: "replace", path: `/structure/${TITLE_ID}/text`, value: "Guarded" },
				],
			});
			expect((result.details as DocEditDetails).ops).toBe(2);
			expect(readFileSync(docPath, "utf8")).toContain("Guarded");
		});

		it("re-extracts after applying a patch (ids refreshed)", async () => {
			const docPath = join(cwd, "refresh.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			await createDocReadTool(cwd).execute("r4", { path: docPath });
			const result = await createDocEditTool(cwd).execute("e4", {
				path: docPath,
				patch: [{ op: "replace", path: `/structure/${TITLE_ID}/text`, value: "Updated" }],
			});
			expect(getText(result as any)).toContain("re-extracted");
		});
	});

	// =====================================================================
	// DocWrite integration tests (uses real filetools binary)
	// =====================================================================

	describe("DocWrite", () => {
		it("reconstructs to a new path leaving the source untouched", async () => {
			const docPath = join(cwd, "report.xml");
			const outPath = join(cwd, "report_v2.xml");
			const original = "<doc><title>Hello</title></doc>";
			writeFileSync(docPath, original);
			await createDocReadTool(cwd).execute("r5", { path: docPath });
			const result = await createDocWriteTool(cwd).execute("w1", {
				path: docPath,
				out: outPath,
				patch: [{ op: "replace", path: `/structure/${TITLE_ID}/text`, value: "Patched" }],
			});
			const details = result.details as DocWriteDetails;
			expect(details.ops).toBe(1);
			expect(existsSync(outPath)).toBe(true);
			const outContent = readFileSync(outPath, "utf8");
			expect(outContent).toContain("Patched");
			expect(outContent).not.toContain("<title>Hello");
			// Source left unchanged.
			expect(readFileSync(docPath, "utf8")).toBe(original);
		});

		it("allows guard (test) ops before mutating ops", async () => {
			const docPath = join(cwd, "src.xml");
			const outPath = join(cwd, "dst.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			await createDocReadTool(cwd).execute("r6", { path: docPath });
			const result = await createDocWriteTool(cwd).execute("w2", {
				path: docPath,
				out: outPath,
				patch: [
					{ op: "test", path: `/structure/${ROOT_ID}`, hash: ROOT_HASH },
					{ op: "replace", path: `/structure/${TITLE_ID}/text`, value: "GuardedWrite" },
				],
			});
			expect((result.details as DocWriteDetails).ops).toBe(2);
			expect(existsSync(outPath)).toBe(true);
			expect(readFileSync(outPath, "utf8")).toContain("GuardedWrite");
		});

		it("requires a prior DocRead on the source", async () => {
			const docPath = join(cwd, "orphan.xml");
			const outPath = join(cwd, "orphan_out.xml");
			writeFileSync(docPath, "<doc/>");
			invalidateExtractRecord(docPath);
			await expect(
				createDocWriteTool(cwd).execute("w3", {
					path: docPath,
					out: outPath,
					patch: [{ op: "replace", path: "/structure/n1/text", value: "nope" }],
				}),
			).rejects.toThrow(/DocRead/);
		});
	});
});
