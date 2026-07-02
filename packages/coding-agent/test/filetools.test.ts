import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createDocEditTool } from "../src/core/tools/doc/docedit.js";
import { createDocGrepTool, renderGrepView } from "../src/core/tools/doc/docgrep.js";
import { createDocPeekTool, renderReadView } from "../src/core/tools/doc/docpeek.js";
import { createDocReadTool, renderEnvelopeText } from "../src/core/tools/doc/docread.js";
import { createDocScanTool, renderScanView } from "../src/core/tools/doc/docscan.js";
import { createDocWriteTool } from "../src/core/tools/doc/docwrite.js";
import type { Envelope, GrepView, ReadView, ScanView } from "../src/core/tools/filetools-shared.js";
import {
	DOCREAD_MAX_RENDER_TOKENS,
	estimateTextTokens,
	findMissingPatchIds,
	findNodeById,
	invalidateExtractRecord,
	patchOpNodeId,
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
	affected?: Array<{ id: string; tag: string; text?: string }>;
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
	// Unit tests for id/patch helpers
	// =====================================================================

	describe("node/patch helpers", () => {
		const structure = [
			{
				id: "root",
				tag: "doc",
				children: [
					{ id: "a", tag: "title", text: "Hello" },
					{ id: "b", tag: "body", children: [{ id: "c", tag: "p", text: "deep" }] },
				],
			},
		];

		it("findNodeById finds nested nodes", () => {
			expect(findNodeById(structure, "c")?.tag).toBe("p");
			expect(findNodeById(structure, "root")?.tag).toBe("doc");
			expect(findNodeById(structure, "missing")).toBeUndefined();
		});

		it("patchOpNodeId extracts ids from pointers and anchors", () => {
			expect(patchOpNodeId({ op: "replace", path: "/structure/a/text", value: "x" })).toBe("a");
			expect(patchOpNodeId({ op: "replace", path: "/structure/b/attrs/class", value: "x" })).toBe("b");
			expect(patchOpNodeId({ op: "remove", path: "/structure/c" })).toBe("c");
			expect(patchOpNodeId({ op: "test", path: "/structure/root", hash: "sha256:x" })).toBe("root");
			expect(patchOpNodeId({ op: "add", after: "a", value: { tag: "p" } })).toBe("a");
			expect(patchOpNodeId({ op: "add", before: "b", value: { tag: "p" } })).toBe("b");
		});

		it("findMissingPatchIds returns ids absent from the structure", () => {
			expect(
				findMissingPatchIds(
					[
						{ op: "replace", path: "/structure/a/text", value: "x" },
						{ op: "replace", path: "/structure/c/text", value: "y" },
					],
					structure,
				),
			).toEqual([]);
			expect(findMissingPatchIds([{ op: "replace", path: "/structure/gone/text", value: "x" }], structure)).toEqual([
				"gone",
			]);
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
		it("auto-extracts without a prior DocRead and applies a valid patch", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			invalidateExtractRecord(docPath);
			const tool = createDocEditTool(cwd);
			const result = await tool.execute("e1", {
				path: docPath,
				patch: [{ op: "replace", path: `/structure/${TITLE_ID}/text`, value: "Hi" }],
			});
			expect((result.details as DocEditDetails).ops).toBe(1);
			expect(readFileSync(docPath, "utf8")).toContain("Hi");
		});

		it("throws with the fresh structure when the patch targets stale ids", async () => {
			const docPath = join(cwd, "stale.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			invalidateExtractRecord(docPath);
			const tool = createDocEditTool(cwd);
			await expect(
				tool.execute("e1b", {
					path: docPath,
					patch: [{ op: "replace", path: "/structure/el_doesnotexist/text", value: "Hi" }],
				}),
			).rejects.toThrow(new RegExp(`no longer exist[\\s\\S]*#${TITLE_ID} <title>`));
			// The document was not modified.
			expect(readFileSync(docPath, "utf8")).toContain("<title>Hello");
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

		it("reports the nodes affected by the patch", async () => {
			const docPath = join(cwd, "affected.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			await createDocReadTool(cwd).execute("r4a", { path: docPath });
			const result = await createDocEditTool(cwd).execute("e4a", {
				path: docPath,
				patch: [{ op: "replace", path: `/structure/${TITLE_ID}/text`, value: "Updated" }],
			});
			const affected = (result.details as DocEditDetails).affected;
			expect(affected).toBeDefined();
			expect(affected?.some((n) => n.id === TITLE_ID)).toBe(true);
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

		it("auto-extracts the source without a prior DocRead", async () => {
			const docPath = join(cwd, "orphan.xml");
			const outPath = join(cwd, "orphan_out.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			invalidateExtractRecord(docPath);
			const result = await createDocWriteTool(cwd).execute("w3", {
				path: docPath,
				out: outPath,
				patch: [{ op: "replace", path: `/structure/${TITLE_ID}/text`, value: "Auto" }],
			});
			expect((result.details as DocWriteDetails).ops).toBe(1);
			expect(existsSync(outPath)).toBe(true);
			expect(readFileSync(outPath, "utf8")).toContain("Auto");
		});

		it("throws with the fresh structure when the patch targets stale ids", async () => {
			const docPath = join(cwd, "orphan2.xml");
			const outPath = join(cwd, "orphan2_out.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			invalidateExtractRecord(docPath);
			await expect(
				createDocWriteTool(cwd).execute("w3b", {
					path: docPath,
					out: outPath,
					patch: [{ op: "replace", path: "/structure/el_doesnotexist/text", value: "nope" }],
				}),
			).rejects.toThrow(new RegExp(`no longer exist[\\s\\S]*#${TITLE_ID} <title>`));
			expect(existsSync(outPath)).toBe(false);
		});
	});
});

// ===========================================================================
// Discovery tools: DocScan / DocGrep / DocPeek (the token-sensitive loop)
// ===========================================================================

describe("document discovery tools", () => {
	let cwd: string;

	beforeEach(() => {
		cwd = join(tmpdir(), `filetools-disc-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
	});

	afterEach(() => {
		rmSync(cwd, { recursive: true, force: true });
	});

	// ---- Pure render unit tests (no binary) -------------------------------

	describe("renderScanView", () => {
		it("renders block previews with structural-path ids and a pagination notice", () => {
			const view: ScanView = {
				file_type: "xml",
				block_count: 2,
				total_tokens: 8,
				offset: 0,
				returned: 2,
				total: 5,
				blocks: [
					{
						id: "node[a:0]",
						block_type: "para",
						preview: "Hello",
						content_hash: "el_1",
						parent_id: null,
						token_estimate: 3,
						section_name: "Intro",
						section_number: 1,
					},
					{
						id: "node[b:1]",
						block_type: "para",
						preview: "",
						content_hash: "el_2",
						parent_id: "node[a:0]",
						token_estimate: 1,
						section_name: "",
						section_number: 0,
					},
				],
			};
			const text = renderScanView(view);
			expect(text).toContain("xml — 2/5 blocks");
			expect(text).toContain('#node[a:0] [para] Intro#1 ~3tok :: "Hello"');
			expect(text).toContain("#node[b:1] [para] ~1tok");
			expect(text).toContain("3 more blocks not shown — re-run with offset:2 to continue");
		});
	});

	describe("renderGrepView", () => {
		it("renders matches as #block_id:line :: snippet and flags read-only", () => {
			const view: GrepView = {
				pattern: "Q1",
				returned: 2,
				matches: [
					{ block_id: "el_a", line: 4, snippet: "Q1 revenue", writable: true },
					{ block_id: "el_b", line: 9, snippet: "Q1 cost", writable: false },
				],
			};
			const text = renderGrepView(view);
			expect(text).toContain('grep "Q1" — 2 matches');
			expect(text).toContain('#el_a:4 :: "Q1 revenue"');
			expect(text).toContain('#el_b:9 (read-only) :: "Q1 cost"');
		});

		it("renders a header-only line when there are no matches", () => {
			const text = renderGrepView({ pattern: "zzz", returned: 0, matches: [] });
			expect(text).toBe('grep "zzz" — 0 matches');
		});
	});

	describe("renderReadView", () => {
		it("renders hydrated nodes in the id-addressed dialect with a pagination notice", () => {
			const view: ReadView = {
				offset: 0,
				returned: 1,
				total: 3,
				nodes: [{ id: "el_1", tag: "title", text: "Hello" }],
			};
			const text = renderReadView(view);
			expect(text).toContain("1/3 blocks (offset 0)");
			expect(text).toContain('#el_1 <title> :: "Hello"');
			expect(text).toContain("2 more blocks not shown — re-run with offset:1 to continue");
		});
	});

	// ---- Integration tests (real filetools binary) ------------------------

	describe("DocScan", () => {
		it("outlines a document into a paginated block manifest", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title><body>World</body></doc>");
			const result = await createDocScanTool(cwd).execute("s1", { path: docPath });
			const text = getText(result);
			expect(text).toContain("xml — 3/3 blocks");
			expect(text).toContain("#node[title:0]");
			expect(text).toContain("Hello");
			expect((result.details as { total?: number }).total).toBe(3);
		});

		it("paginates with offset/limit", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title><body>World</body></doc>");
			const result = await createDocScanTool(cwd).execute("s2", { path: docPath, offset: 0, limit: 1 });
			const text = getText(result);
			expect(text).toContain("1/3 blocks");
			expect(text).toContain("more blocks not shown — re-run with offset:1");
		});
	});

	describe("DocGrep", () => {
		it("returns editable el_ node ids for a literal match", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title><body>World</body></doc>");
			const result = await createDocGrepTool(cwd).execute("g1", { path: docPath, pattern: "Hello" });
			const text = getText(result);
			expect(text).toContain('grep "Hello" — 1 match');
			// The match id is the same el_ id space DocEdit patches against.
			expect(text).toMatch(/#el_[0-9a-f]+:1 :: "Hello"/);
			expect((result.details as { matches?: number }).matches).toBe(1);
		});

		it("matches case-insensitively with ignoreCase", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			const result = await createDocGrepTool(cwd).execute("g2", {
				path: docPath,
				pattern: "hello",
				ignoreCase: true,
			});
			expect((result.details as { matches?: number }).matches).toBe(1);
		});

		it("returns no matches for an absent pattern", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title></doc>");
			const result = await createDocGrepTool(cwd).execute("g3", { path: docPath, pattern: "absent-xyz" });
			expect((result.details as { matches?: number }).matches).toBe(0);
			expect(getText(result)).toContain("0 matches");
		});
	});

	describe("DocPeek", () => {
		it("hydrates a single block by its DocScan path id", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title><body>World</body></doc>");
			const result = await createDocPeekTool(cwd).execute("p1", { path: docPath, id: ["node[title:0]"] });
			const text = getText(result);
			// Hydrated node carries the editable el_ id and the text.
			expect(text).toMatch(/#el_[0-9a-f]+ <title> :: "Hello"/);
			expect(text).not.toContain("World");
			expect((result.details as { returned?: number }).returned).toBe(1);
		});

		it("reads the whole document when no ids are given", async () => {
			const docPath = join(cwd, "report.xml");
			writeFileSync(docPath, "<doc><title>Hello</title><body>World</body></doc>");
			const result = await createDocPeekTool(cwd).execute("p2", { path: docPath });
			const text = getText(result);
			expect(text).toContain("<doc>");
			expect(text).toContain('<title> :: "Hello"');
			expect(text).toContain('<body> :: "World"');
		});
	});
});

// ===========================================================================
// OOXML end-to-end loop (xlsx + pptx): scan → grep → peek → edit.
// Regression for issue #78 — before filetools v0.1.7 the discovery loop reached
// xlsx sheet structure only (DocGrep/DocPeek did not see cell values); v0.1.7
// "reach full cell/text content via scan/grep/read for all formats" closed it,
// and also covers pptx. These fixtures are minimal valid OOXML packages built
// with a dependency-free STORED-zip writer, exercised through the real binary.
// ===========================================================================

const CRC32_TABLE = (() => {
	const table = new Uint32Array(256);
	for (let n = 0; n < 256; n++) {
		let c = n;
		for (let k = 0; k < 8; k++) {
			c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
		}
		table[n] = c >>> 0;
	}
	return table;
})();

function crc32(buf: Buffer): number {
	let c = 0xffffffff;
	for (let i = 0; i < buf.length; i++) {
		c = CRC32_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
	}
	return (c ^ 0xffffffff) >>> 0;
}

/**
 * Write a minimal STORED (uncompressed) zip — enough for filetools to read an
 * OOXML package without pulling in a zip-writer dependency.
 */
function writeZip(path: string, entries: Array<{ name: string; content: string }>): void {
	const chunks: Buffer[] = [];
	const centrals: Buffer[] = [];
	let offset = 0;
	for (const { name, content } of entries) {
		const nameBuf = Buffer.from(name, "utf8");
		const data = Buffer.from(content, "utf8");
		const crc = crc32(data);

		const local = Buffer.alloc(30 + nameBuf.length);
		local.writeUInt32LE(0x04034b50, 0);
		local.writeUInt16LE(20, 4); // version needed
		local.writeUInt16LE(0, 8); // method: stored
		local.writeUInt32LE(crc, 14);
		local.writeUInt32LE(data.length, 18); // compressed size
		local.writeUInt32LE(data.length, 22); // uncompressed size
		local.writeUInt16LE(nameBuf.length, 26);
		nameBuf.copy(local, 30);
		chunks.push(local, data);

		const central = Buffer.alloc(46 + nameBuf.length);
		central.writeUInt32LE(0x02014b50, 0);
		central.writeUInt16LE(20, 4); // version made by
		central.writeUInt16LE(20, 6); // version needed
		central.writeUInt16LE(0, 10); // method: stored
		central.writeUInt32LE(crc, 16);
		central.writeUInt32LE(data.length, 20);
		central.writeUInt32LE(data.length, 24);
		central.writeUInt16LE(nameBuf.length, 28);
		central.writeUInt32LE(offset, 42); // local header offset
		nameBuf.copy(central, 46);
		centrals.push(central);

		offset += local.length + data.length;
	}

	const centralBuf = Buffer.concat(centrals);
	const eocd = Buffer.alloc(22);
	eocd.writeUInt32LE(0x06054b50, 0);
	eocd.writeUInt16LE(entries.length, 8); // entries on this disk
	eocd.writeUInt16LE(entries.length, 10); // total entries
	eocd.writeUInt32LE(centralBuf.length, 12); // central directory size
	eocd.writeUInt32LE(offset, 16); // central directory offset

	writeFileSync(path, Buffer.concat([...chunks, centralBuf, eocd]));
}

const XLSX_PARTS: Array<{ name: string; content: string }> = [
	{
		name: "[Content_Types].xml",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
			'<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
			'<Default Extension="xml" ContentType="application/xml"/>' +
			'<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>' +
			'<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' +
			"</Types>",
	},
	{
		name: "_rels/.rels",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
			'<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>' +
			"</Relationships>",
	},
	{
		name: "xl/workbook.xml",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">' +
			'<sheets><sheet name="Budget" sheetId="1" r:id="rId1"/></sheets></workbook>',
	},
	{
		name: "xl/_rels/workbook.xml.rels",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
			'<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>' +
			"</Relationships>",
	},
	{
		name: "xl/worksheets/sheet1.xml",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>' +
			'<row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="D1" t="inlineStr"><is><t>MAGICVALUE</t></is></c></row>' +
			'<row r="2"><c r="A2" t="inlineStr"><is><t>Widget</t></is></c><c r="D2" t="inlineStr"><is><t>Quarterly</t></is></c></row>' +
			"</sheetData></worksheet>",
	},
];

const PPTX_PARTS: Array<{ name: string; content: string }> = [
	{
		name: "[Content_Types].xml",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
			'<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
			'<Default Extension="xml" ContentType="application/xml"/>' +
			'<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>' +
			'<Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>' +
			"</Types>",
	},
	{
		name: "_rels/.rels",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
			'<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>' +
			"</Relationships>",
	},
	{
		name: "ppt/presentation.xml",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">' +
			'<p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>',
	},
	{
		name: "ppt/_rels/presentation.xml.rels",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
			'<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>' +
			"</Relationships>",
	},
	{
		name: "ppt/slides/slide1.xml",
		content:
			'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
			'<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">' +
			"<p:cSld><p:spTree><p:sp><p:txBody>" +
			"<a:p><a:r><a:t>Hello Slide Title</a:t></a:r></a:p>" +
			"<a:p><a:r><a:t>PPTXMAGIC body text</a:t></a:r></a:p>" +
			"</p:txBody></p:sp></p:spTree></p:cSld></p:sld>",
	},
];

describe("OOXML end-to-end loop (xlsx)", () => {
	let cwd: string;
	let docPath: string;

	beforeEach(() => {
		cwd = join(tmpdir(), `filetools-xlsx-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
		docPath = join(cwd, "data.xlsx");
		writeZip(docPath, XLSX_PARTS);
		invalidateExtractRecord(docPath);
	});

	afterEach(() => {
		rmSync(cwd, { recursive: true, force: true });
	});

	it("DocScan previews reach cell text", async () => {
		const text = getText(await createDocScanTool(cwd).execute("xs", { path: docPath }));
		expect(text).toContain("#sheet[0]");
		// Pre-v0.1.7 the row preview showed structure only; now it carries cell text.
		expect(text).toContain("MAGICVALUE");
	});

	it("DocGrep matches a cell value", async () => {
		const result = await createDocGrepTool(cwd).execute("xg", { path: docPath, pattern: "MAGICVALUE" });
		expect((result.details as { matches?: number }).matches).toBe(1);
		expect(getText(result)).toContain("MAGICVALUE");
	});

	it("DocPeek hydrates the row-range block into cell text", async () => {
		const text = getText(await createDocPeekTool(cwd).execute("xp", { path: docPath, id: ["sheet[0].rows[0-1]"] }));
		expect(text).toContain("MAGICVALUE");
		expect(text).toContain("Widget");
	});

	it("DocEdit rewrites a cell and the change is visible to DocGrep", async () => {
		// The editable el_ id for a spreadsheet cell comes from the extract (DocRead).
		const readText = getText(await createDocReadTool(cwd).execute("xr", { path: docPath }));
		const match = readText.match(/#(el_[0-9a-f]+)[^\n]*"MAGICVALUE"/);
		expect(match).not.toBeNull();
		const elId = match![1];

		const edit = await createDocEditTool(cwd).execute("xe", {
			path: docPath,
			patch: [{ op: "replace", path: `/structure/${elId}/text`, value: "EDITEDVALUE" }],
		});
		expect((edit.details as DocEditDetails).ops).toBe(1);

		const after = await createDocGrepTool(cwd).execute("xg2", { path: docPath, pattern: "EDITEDVALUE" });
		expect((after.details as { matches?: number }).matches).toBe(1);
		const gone = await createDocGrepTool(cwd).execute("xg3", { path: docPath, pattern: "MAGICVALUE" });
		expect((gone.details as { matches?: number }).matches).toBe(0);
	});
});

describe("OOXML end-to-end loop (pptx)", () => {
	let cwd: string;
	let docPath: string;

	beforeEach(() => {
		cwd = join(tmpdir(), `filetools-pptx-${Date.now()}-${Math.random().toString(36).slice(2)}`);
		mkdirSync(cwd, { recursive: true });
		docPath = join(cwd, "deck.pptx");
		writeZip(docPath, PPTX_PARTS);
		invalidateExtractRecord(docPath);
	});

	afterEach(() => {
		rmSync(cwd, { recursive: true, force: true });
	});

	it("DocScan previews reach slide text", async () => {
		const text = getText(await createDocScanTool(cwd).execute("ps", { path: docPath }));
		expect(text).toContain("#slide[0]");
		expect(text).toContain("PPTXMAGIC");
	});

	it("DocGrep matches slide text", async () => {
		const result = await createDocGrepTool(cwd).execute("pg", { path: docPath, pattern: "PPTXMAGIC" });
		expect((result.details as { matches?: number }).matches).toBe(1);
		expect(getText(result)).toContain("PPTXMAGIC");
	});

	it("DocPeek hydrates the slide into its text", async () => {
		const text = getText(await createDocPeekTool(cwd).execute("pp", { path: docPath, id: ["slide[0]"] }));
		expect(text).toContain("PPTXMAGIC");
		expect(text).toContain("Hello Slide Title");
	});

	it("DocEdit rewrites slide text and the change is visible to DocGrep", async () => {
		const readText = getText(await createDocReadTool(cwd).execute("pr", { path: docPath }));
		const match = readText.match(/#(el_[0-9a-f]+)[^\n]*"PPTXMAGIC body text"/);
		expect(match).not.toBeNull();
		const elId = match![1];

		const edit = await createDocEditTool(cwd).execute("pe", {
			path: docPath,
			patch: [{ op: "replace", path: `/structure/${elId}/text`, value: "PPTXEDITED text" }],
		});
		expect((edit.details as DocEditDetails).ops).toBe(1);

		const after = await createDocGrepTool(cwd).execute("pg2", { path: docPath, pattern: "PPTXEDITED" });
		expect((after.details as { matches?: number }).matches).toBe(1);
		const gone = await createDocGrepTool(cwd).execute("pg3", { path: docPath, pattern: "PPTXMAGIC" });
		expect((gone.details as { matches?: number }).matches).toBe(0);
	});
});
